//! Port of `src/libgit2/hashsig.c`: similarity signatures of file contents
//! (`git_hashsig`), used by the rename and copy detection of `diff_tform.c`.
//!
//! A signature hashes each run of a file (a line, or 80 bytes of a longer
//! one) and keeps the 127 smallest and the 127 largest hashes, in two
//! fixed-size heaps. Two signatures are compared by the overlap of these
//! sets, on a scale of 0 to 100.
//!
//! The heaps are the C's, quirks included, because they decide which hashes
//! are kept and so the scores `diff_tform` sees:
//!
//! - an insertion into a full heap whose value beats the top removes the
//!   top but does not insert the value (the heap then holds 126 values);
//! - the sift down reads the slot just past the end of the heap (a value
//!   that was moved out of it, still in the 127-slot array) and can swap it
//!   back in.
//!
//! [`Signature`] is the shared logic (kind 1, opaque: C only passes
//! `git_hashsig *` around). [`HashSig`] is the safe API, and [`ffi`] the C
//! exports.

use core::cmp::Ordering;
use core::ffi::{CStr, c_int};

use crate::git_error_set;
use crate::util::errors::{self, Error, ErrorClass, ErrorCode, Result};
use crate::util::{futils, posix, util as gutil};

/// `HASHSIG_SCALE`.
const SCALE: c_int = 100;
/// `HASHSIG_MAX_RUN`.
const MAX_RUN: usize = 80;
/// `HASHSIG_HASH_START`.
const HASH_START: u64 = 0x0123_4567_8ABC_DEF0;
/// `HASHSIG_HEAP_SIZE`.
pub const HEAP_SIZE: usize = (1 << 7) - 1;
/// `HASHSIG_HEAP_MIN_SIZE`.
const HEAP_MIN_SIZE: usize = 4;
/// The read size of `git_hashsig_create_fromfile` (its stack buffer). The
/// hash state restarts at each block, so this is part of the result.
const FILE_BLOCK: usize = 0x1000;

/// `GIT_HASHSIG_NORMAL`.
pub const GIT_HASHSIG_NORMAL: u32 = 0;
/// `GIT_HASHSIG_IGNORE_WHITESPACE`.
pub const GIT_HASHSIG_IGNORE_WHITESPACE: u32 = 1 << 0;
/// `GIT_HASHSIG_SMART_WHITESPACE`.
pub const GIT_HASHSIG_SMART_WHITESPACE: u32 = 1 << 1;
/// `GIT_HASHSIG_ALLOW_SMALL_FILES`.
pub const GIT_HASHSIG_ALLOW_SMALL_FILES: u32 = 1 << 2;

/// `hashsig_heap`: a fixed array of 127 hashes, the first `size` a heap
/// under `hashsig_cmp_max` (`MIN == false`: keeps the largest values) or
/// `hashsig_cmp_min` (`MIN == true`: keeps the smallest).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Heap<const MIN: bool> {
    size: usize,
    values: [u32; HEAP_SIZE],
}

impl<const MIN: bool> Heap<MIN> {
    const fn new() -> Self {
        // `git__calloc`: the slots past `size` start at 0, and are read.
        Heap {
            size: 0,
            values: [0; HEAP_SIZE],
        }
    }

    /// `hashsig_cmp_max` / `hashsig_cmp_min`.
    #[inline(always)]
    fn cmp(a: u32, b: u32) -> Ordering {
        if MIN { b.cmp(&a) } else { a.cmp(&b) }
    }

    /// `hashsig_heap_up`.
    #[inline]
    fn up(&mut self, mut el: usize) {
        let v = &mut self.values;
        while el > 0 {
            let parent_el = (el - 1) >> 1;
            if Self::cmp(v[parent_el], v[el]) != Ordering::Greater {
                break;
            }
            v.swap(el, parent_el);
            el = parent_el;
        }
    }

    /// `hashsig_heap_down`. As in C, the right child may be the slot at
    /// `size` (outside the heap, inside the array).
    #[inline]
    fn down(&mut self, mut el: usize) {
        let v = &mut self.values;
        while el < self.size / 2 {
            let lel = (el << 1) + 1;
            let rel = (el << 1) + 2;
            let (x, lv, rv) = (v[el], v[lel], v[rel]);
            if Self::cmp(x, lv) == Ordering::Less && Self::cmp(x, rv) == Ordering::Less {
                break;
            }
            let swapel = if Self::cmp(lv, rv) == Ordering::Less {
                lel
            } else {
                rel
            };
            v[el] = v[swapel];
            v[swapel] = x;
            el = swapel;
        }
    }

    /// `hashsig_heap_insert`.
    #[inline]
    fn insert(&mut self, val: u32) {
        if self.size < HEAP_SIZE {
            self.values[self.size] = val;
            self.size += 1;
            self.up(self.size - 1);
        } else if Self::cmp(val, self.values[0]) == Ordering::Greater {
            // C pops the top and does not insert `val`.
            self.size -= 1;
            self.values[0] = self.values[self.size];
            self.down(0);
        }
    }

    /// `hashsig_heap_sort` (`git__qsort_r` by the heap's comparison; equal
    /// hashes are equal values, so any sort gives the C's array).
    fn sort(&mut self) {
        self.values[..self.size].sort_unstable_by(|a, b| Self::cmp(*a, *b));
    }

    /// `hashsig_heap_compare`: the overlap of two sorted heaps, 0 to 100.
    fn compare(&self, other: &Self) -> c_int {
        let (a, b) = (&self.values[..self.size], &other.values[..other.size]);
        let (mut i, mut j, mut matches) = (0usize, 0usize, 0 as c_int);
        while i < a.len() && j < b.len() {
            match Self::cmp(a[i], b[j]) {
                Ordering::Less => i += 1,
                Ordering::Greater => j += 1,
                Ordering::Equal => {
                    i += 1;
                    j += 1;
                    matches += 1;
                }
            }
        }
        // sizes are at most 127 each: no overflow, and the sum is > 0 here
        SCALE * (matches * 2) / (a.len() + b.len()) as c_int
    }
}

/// `struct git_hashsig`: a similarity signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    mins: Heap<true>,
    maxs: Heap<false>,
    lines: usize,
    opt: u32,
}

/// `hashsig_in_progress`, of which only `use_ignores` is used (C fills a
/// table that nothing reads).
struct Progress {
    use_ignores: bool,
}

impl Signature {
    /// `hashsig_alloc`.
    fn empty(opt: u32) -> Signature {
        Signature {
            mins: Heap::new(),
            maxs: Heap::new(),
            lines: 0,
            opt,
        }
    }

    /// `hashsig_in_progress_init`.
    fn progress(&self) -> Result<Progress> {
        crate::git_assert!(
            (self.opt & GIT_HASHSIG_IGNORE_WHITESPACE) == 0
                || (self.opt & GIT_HASHSIG_SMART_WHITESPACE) == 0,
            "!(sig->opt & GIT_HASHSIG_IGNORE_WHITESPACE) || !(sig->opt & GIT_HASHSIG_SMART_WHITESPACE)"
        );
        Ok(Progress {
            use_ignores: self.opt & (GIT_HASHSIG_IGNORE_WHITESPACE | GIT_HASHSIG_SMART_WHITESPACE)
                != 0,
        })
    }

    /// `hashsig_add_hashes`.
    ///
    /// Where the C reads the byte just past `data` (after skipping
    /// whitespace up to the end), the Rust uses 0: see DEFECTS.md. That
    /// byte only decides `use_ignores` for the next block of a file.
    fn add_hashes(&mut self, data: &[u8], prog: &mut Progress) {
        let end = data.len();
        let at = |i: usize| -> u8 { if i < end { data[i] } else { 0 } };
        let ws = self.opt & (GIT_HASHSIG_IGNORE_WHITESPACE | GIT_HASHSIG_SMART_WHITESPACE) != 0;
        let smart = self.opt & GIT_HASHSIG_SMART_WHITESPACE != 0;
        let mut use_ignores = prog.use_ignores;
        let mut scan = 0usize;

        while scan < end {
            let mut state = HASH_START;
            let mut len = 0usize;

            while scan < end && len < MAX_RUN {
                let mut ch = data[scan];

                if use_ignores {
                    while scan < end && gutil::isspace_nonlf(ch) {
                        scan += 1;
                        ch = at(scan);
                    }
                } else if ws {
                    while scan < end && ch == b'\r' {
                        scan += 1;
                        ch = at(scan);
                    }
                }

                // peek at the next character to decide what to do next
                if smart {
                    use_ignores = ch == b'\n';
                }

                if scan >= end {
                    break;
                }
                scan += 1;

                // the run's terminator
                if ch == b'\n' || ch == 0 {
                    self.lines += 1;
                    break;
                }

                len += 1;
                // HASHSIG_HASH_MIX
                state = (state << 5).wrapping_sub(state).wrapping_add(u64::from(ch));
            }

            if len > 0 {
                self.mins.insert(state as u32);
                self.maxs.insert(state as u32);

                while scan < end && (data[scan] == b'\n' || data[scan] == 0) {
                    scan += 1;
                }
            }
        }

        prog.use_ignores = use_ignores;
    }

    /// `hashsig_finalize_hashes`.
    fn finalize(&mut self) -> Result<()> {
        if self.mins.size < HEAP_MIN_SIZE && self.opt & GIT_HASHSIG_ALLOW_SMALL_FILES == 0 {
            let _ = git_error_set!(
                ErrorClass::INVALID,
                "file too small for similarity signature calculation"
            );
            return Err(ErrorCode::BUFS);
        }
        self.mins.sort();
        self.maxs.sort();
        Ok(())
    }

    /// `git_hashsig_create`: the signature of `buf`.
    pub fn from_buf(buf: &[u8], opts: u32) -> Result<Signature> {
        let mut sig = Signature::empty(opts);
        let mut prog = sig.progress()?;
        sig.add_hashes(buf, &mut prog);
        sig.finalize()?;
        Ok(sig)
    }

    /// `git_hashsig_create_fromfile`: the signature of the file at `path`,
    /// read in blocks of 4096 bytes (as the C, whose hash runs restart at
    /// each block).
    pub fn from_file(path: &CStr, opts: u32) -> Result<Signature> {
        let mut sig = Signature::empty(opts);
        let fd = futils::open_ro(path)?;
        let r = sig.read_fd(fd, path);
        let _ = posix::close(fd);
        r?;
        sig.finalize()?;
        Ok(sig)
    }

    fn read_fd(&mut self, fd: c_int, path: &CStr) -> Result<()> {
        let mut prog = self.progress()?;
        let mut buf = [0u8; FILE_BLOCK];
        loop {
            match posix::read(fd, &mut buf) {
                Ok(0) => return Ok(()),
                Ok(n) => self.add_hashes(&buf[..n], &mut prog),
                Err(_) => {
                    let _ = git_error_set!(
                        ErrorClass::OS,
                        "read error on '{}' calculating similarity hashes",
                        errors::Bytes(path.to_bytes())
                    );
                    return Err(ErrorCode::GENERIC);
                }
            }
        }
    }

    /// `git_hashsig_compare`: the similarity of two signatures, 0 to 100.
    pub fn compare(&self, b: &Signature) -> c_int {
        let a = self;
        // no element in either: each file is empty or blank; they are
        // similar if both are empty or whitespace is ignored
        if a.mins.size == 0 && b.mins.size == 0 {
            if (a.lines == 0 && b.lines == 0) || a.opt & GIT_HASHSIG_IGNORE_WHITESPACE != 0 {
                return SCALE;
            }
            return 0;
        }

        // with fewer than the maximum number of elements, both arrays are
        // the same: use one
        if a.mins.size < HEAP_SIZE {
            a.mins.compare(&b.mins)
        } else {
            let mins = a.mins.compare(&b.mins);
            let maxs = a.maxs.compare(&b.maxs);
            (mins + maxs) / 2
        }
    }
}

// ---------------------------------------------------------------------------
// The safe API
// ---------------------------------------------------------------------------

/// Options of a [`HashSig`] (`git_hashsig_option_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct HashSigOptions(u32);

impl HashSigOptions {
    /// `GIT_HASHSIG_NORMAL`: use all data.
    pub const NORMAL: HashSigOptions = HashSigOptions(GIT_HASHSIG_NORMAL);
    /// `GIT_HASHSIG_IGNORE_WHITESPACE`: ignore whitespace.
    pub const IGNORE_WHITESPACE: HashSigOptions = HashSigOptions(GIT_HASHSIG_IGNORE_WHITESPACE);
    /// `GIT_HASHSIG_SMART_WHITESPACE`: ignore `\r` and all space after `\n`.
    pub const SMART_WHITESPACE: HashSigOptions = HashSigOptions(GIT_HASHSIG_SMART_WHITESPACE);
    /// `GIT_HASHSIG_ALLOW_SMALL_FILES`: allow hashing of small files.
    pub const ALLOW_SMALL_FILES: HashSigOptions = HashSigOptions(GIT_HASHSIG_ALLOW_SMALL_FILES);

    /// The flags as bits.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The options of `bits` (unknown bits are kept, as C does).
    pub const fn from_bits(bits: u32) -> HashSigOptions {
        HashSigOptions(bits)
    }

    /// Whether every flag of `other` is set.
    pub const fn contains(self, other: HashSigOptions) -> bool {
        self.0 & other.0 == other.0
    }
}

impl core::ops::BitOr for HashSigOptions {
    type Output = HashSigOptions;
    fn bitor(self, rhs: HashSigOptions) -> HashSigOptions {
        HashSigOptions(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for HashSigOptions {
    fn bitor_assign(&mut self, rhs: HashSigOptions) {
        self.0 |= rhs.0;
    }
}

/// A similarity signature of a buffer or a file (`git_hashsig`).
///
/// ```
/// use libgit2_rs::{HashSig, HashSigOptions};
/// # let _g = libgit2_rs::util::test_support::init();
/// let a = HashSig::new(b"one\ntwo\nthree\nfour\nfive\n", HashSigOptions::NORMAL)?;
/// let b = HashSig::new(b"one\ntwo\nthree\nfour\nsix\n", HashSigOptions::NORMAL)?;
/// assert_eq!(a.compare(&a), 100);
/// assert_eq!(a.compare(&b), 80);
/// # Ok::<(), libgit2_rs::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HashSig(Box<Signature>);

impl HashSig {
    /// `git_hashsig_create`: the signature of `buf`. Fails with
    /// `ErrorCode::BUFS` for content with fewer than 4 runs, unless
    /// [`HashSigOptions::ALLOW_SMALL_FILES`] is set.
    pub fn new(buf: &[u8], opts: HashSigOptions) -> core::result::Result<HashSig, Error> {
        Ok(HashSig(Box::new(Signature::from_buf(buf, opts.0)?)))
    }

    /// `git_hashsig_create_fromfile`: the signature of the file at `path`.
    pub fn from_file(
        path: &std::path::Path,
        opts: HashSigOptions,
    ) -> core::result::Result<HashSig, Error> {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            Error::from(git_error_set!(
                ErrorClass::INVALID,
                "path contains a NUL byte"
            ))
        })?;
        Ok(HashSig(Box::new(Signature::from_file(&c, opts.0)?)))
    }

    /// `git_hashsig_compare`: the similarity with `other`, from 0
    /// (different) to 100 (same).
    pub fn compare(&self, other: &HashSig) -> i32 {
        self.0.compare(&other.0)
    }
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `hashsig.c`.
pub mod ffi {
    use core::ffi::{c_char, c_int, c_void};

    use super::Signature;
    use crate::git_assert_arg;
    use crate::util::alloc;
    use crate::util::errors::{self, Result};
    use crate::util::export::{barrier, bytes, cstr};

    /// `git_hashsig`, as C sees it: opaque.
    pub type GitHashsig = Signature;

    /// Allocate the signature in storage of `git__allocator` first (C's
    /// `git__calloc`, so that a failing allocator fails first, as in C),
    /// then compute it with `make` and hand it out through `out`.
    fn create(out: *mut *mut GitHashsig, make: impl FnOnce() -> Result<Signature>) -> c_int {
        let p = alloc::malloc(size_of::<Signature>()).cast::<Signature>();
        if p.is_null() {
            return errors::oom().raw();
        }
        match make() {
            Ok(sig) => {
                // SAFETY: `p` is a fresh allocation of the size of
                // `Signature`, aligned to 8 (git__allocator's alignment,
                // enough for it).
                unsafe { p.write(sig) };
                // SAFETY: `out` is writable (the caller's contract).
                unsafe { out.write(p) };
                0
            }
            Err(e) => {
                // SAFETY: our allocation, never initialized, not used after.
                unsafe { alloc::free(p.cast::<c_void>()) };
                e.raw()
            }
        }
    }

    /// `git_hashsig_create`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `buf` is readable for `buflen` bytes (or `buflen`
    /// is 0).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_hashsig_create(
        out: *mut *mut GitHashsig,
        buf: *const c_char,
        buflen: usize,
        opts: c_int,
    ) -> c_int {
        barrier(|| {
            // C checks neither `out` nor `buf`; checking `out` avoids a
            // NULL write (it would crash in C).
            git_assert_arg!(!out.is_null(), "out", -1);
            // SAFETY: the caller's contract; nothing writes `buf` during the
            // call.
            let data = unsafe { bytes(buf, buflen) };
            create(out, || Signature::from_buf(data, opts as u32))
        })
    }

    /// `git_hashsig_create_fromfile`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `path` is a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_hashsig_create_fromfile(
        out: *mut *mut GitHashsig,
        path: *const c_char,
        opts: c_int,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null(), "out", -1);
            // SAFETY: the caller passes a NUL-terminated string.
            let Some(path) = (unsafe { cstr(path) }) else {
                let _ = errors::assert_arg_failed("path");
                return -1;
            };
            create(out, || Signature::from_file(path, opts as u32))
        })
    }

    /// `git_hashsig_free`.
    ///
    /// # Safety
    ///
    /// `sig` is NULL or a signature from `git_hashsig_create*`, not used
    /// after.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_hashsig_free(sig: *mut GitHashsig) {
        barrier(|| {
            // SAFETY: NULL or a live allocation of git__allocator holding a
            // `Signature` (which has no destructor to run).
            unsafe { alloc::free(sig.cast::<c_void>()) }
        })
    }

    /// `git_hashsig_compare`.
    ///
    /// # Safety
    ///
    /// `a` and `b` are live signatures (C does not accept NULL), which
    /// nothing frees during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_hashsig_compare(
        a: *const GitHashsig,
        b: *const GitHashsig,
    ) -> c_int {
        barrier(|| {
            // C dereferences both; a NULL is reported rather than crash.
            git_assert_arg!(!a.is_null(), "a", -1);
            git_assert_arg!(!b.is_null(), "b", -1);
            // SAFETY: live signatures (the caller's contract), only read.
            let a = unsafe { &*a };
            // SAFETY: as above.
            let b = unsafe { &*b };
            a.compare(b)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The C's heap (`hashsig_heap_*`), transcribed with a raw array and
    /// `int` indices, to check the Rust one against.
    struct CHeap {
        size: i32,
        values: [u32; HEAP_SIZE],
        min: bool,
    }

    impl CHeap {
        fn cmp(&self, a: u32, b: u32) -> i32 {
            if self.min {
                if a > b {
                    -1
                } else if a < b {
                    1
                } else {
                    0
                }
            } else if a < b {
                -1
            } else if a > b {
                1
            } else {
                0
            }
        }
        fn up(&mut self, mut el: i32) {
            let mut parent_el = (el - 1) >> 1;
            while el > 0 && self.cmp(self.values[parent_el as usize], self.values[el as usize]) > 0
            {
                self.values.swap(el as usize, parent_el as usize);
                el = parent_el;
                parent_el = (el - 1) >> 1;
            }
        }
        fn down(&mut self, mut el: i32) {
            while el < self.size / 2 {
                let (lel, rel) = ((el << 1) + 1, (el << 1) + 2);
                let v = self.values[el as usize];
                let lv = self.values[lel as usize];
                let rv = self.values[rel as usize];
                if self.cmp(v, lv) < 0 && self.cmp(v, rv) < 0 {
                    break;
                }
                let swapel = if self.cmp(lv, rv) < 0 { lel } else { rel };
                self.values[el as usize] = self.values[swapel as usize];
                self.values[swapel as usize] = v;
                el = swapel;
            }
        }
        fn insert(&mut self, val: u32) {
            if self.size < HEAP_SIZE as i32 {
                self.values[self.size as usize] = val;
                self.size += 1;
                self.up(self.size - 1);
            } else if self.cmp(val, self.values[0]) > 0 {
                self.size -= 1;
                self.values[0] = self.values[self.size as usize];
                self.down(0);
            }
        }
    }

    fn lcg(seed: &mut u64) -> u32 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (*seed >> 33) as u32
    }

    #[test]
    fn heaps_match_the_c_transcription() {
        let mut seed = 7u64;
        for round in 0..40 {
            let mut cmin = CHeap {
                size: 0,
                values: [0; HEAP_SIZE],
                min: true,
            };
            let mut cmax = CHeap {
                size: 0,
                values: [0; HEAP_SIZE],
                min: false,
            };
            let mut rmin = Heap::<true>::new();
            let mut rmax = Heap::<false>::new();
            let n = 50 + round * 37;
            for _ in 0..n {
                // small ranges make ties
                let v = if round % 3 == 0 {
                    lcg(&mut seed) % 50
                } else {
                    lcg(&mut seed)
                };
                cmin.insert(v);
                cmax.insert(v);
                rmin.insert(v);
                rmax.insert(v);
                assert_eq!(cmin.size as usize, rmin.size);
                assert_eq!(cmin.values, rmin.values);
                assert_eq!(cmax.size as usize, rmax.size);
                assert_eq!(cmax.values, rmax.values);
            }
        }
    }

    #[test]
    fn small_files() {
        let _g = crate::util::test_support::init();
        assert_eq!(
            Signature::from_buf(b"a\nb\nc\n", 0).unwrap_err(),
            ErrorCode::BUFS
        );
        assert_eq!(
            errors::last_message().unwrap(),
            b"file too small for similarity signature calculation"
        );
        let s = Signature::from_buf(b"a\nb\nc\n", GIT_HASHSIG_ALLOW_SMALL_FILES).unwrap();
        assert_eq!(s.mins.size, 3);
        assert_eq!(s.lines, 3);
        let e = Signature::from_buf(b"", GIT_HASHSIG_ALLOW_SMALL_FILES).unwrap();
        let blank = Signature::from_buf(b"\n\n", GIT_HASHSIG_ALLOW_SMALL_FILES).unwrap();
        assert_eq!(e.compare(&e), 100);
        assert_eq!(e.compare(&blank), 0);
        let blank_ws = Signature::from_buf(
            b"\n\n",
            GIT_HASHSIG_ALLOW_SMALL_FILES | GIT_HASHSIG_IGNORE_WHITESPACE,
        )
        .unwrap();
        assert_eq!(blank_ws.compare(&e), 100);
    }

    #[test]
    fn both_whitespace_flags_assert() {
        let _g = crate::util::test_support::init();
        let r = Signature::from_buf(
            b"a\nb\nc\nd\n",
            GIT_HASHSIG_IGNORE_WHITESPACE | GIT_HASHSIG_SMART_WHITESPACE,
        );
        assert_eq!(r.unwrap_err(), ErrorCode::GENERIC);
        assert_eq!(
            errors::last_message().unwrap(),
            b"unrecoverable internal error: '!(sig->opt & GIT_HASHSIG_IGNORE_WHITESPACE) || !(sig->opt & GIT_HASHSIG_SMART_WHITESPACE)'"
        );
    }

    #[test]
    fn whitespace_modes() {
        let _g = crate::util::test_support::init();
        let a = b"line one\nline two\nline three\nline four\n";
        let b = b"line   one\r\n  line two\r\nline three  \r\nline\tfour\r\n";
        let n = |x: &[u8], o| Signature::from_buf(x, o).unwrap();
        assert!(n(a, 0).compare(&n(b, 0)) < 100);
        let iw = GIT_HASHSIG_IGNORE_WHITESPACE;
        assert_eq!(n(a, iw).compare(&n(b, iw)), 100);
        let sw = GIT_HASHSIG_SMART_WHITESPACE;
        // smart: leading space and CRs go, inner spaces stay
        let c = b"line one\r\n   line two\r\nline three\r\n\tline four\r\n";
        assert_eq!(n(a, sw).compare(&n(c, sw)), 100);
    }

    #[test]
    fn long_files_use_both_heaps() {
        let _g = crate::util::test_support::init();
        let mut a = Vec::new();
        let mut b = Vec::new();
        for i in 0..1000 {
            a.extend_from_slice(format!("line number {i}\n").as_bytes());
            b.extend_from_slice(
                format!("line number {}\n", if i % 4 == 0 { i + 7000 } else { i }).as_bytes(),
            );
        }
        let sa = Signature::from_buf(&a, 0).unwrap();
        let sb = Signature::from_buf(&b, 0).unwrap();
        assert!(sa.mins.size >= HEAP_SIZE - 1);
        assert_eq!(sa.compare(&sa), 100);
        let s = sa.compare(&sb);
        assert!(s > 0 && s < 100, "{s}");
        assert_eq!(
            HashSig::new(&a, HashSigOptions::NORMAL)
                .unwrap()
                .compare(&HashSig::new(&b, HashSigOptions::NORMAL).unwrap()),
            s
        );
    }

    #[test]
    fn long_runs_split_at_80() {
        let _g = crate::util::test_support::init();
        let line = [b'x'; 400];
        let s = Signature::from_buf(&line, 0).unwrap();
        // five runs of 80 identical bytes: five equal hashes
        assert_eq!(s.mins.size, 5);
        assert_eq!(s.lines, 0);
        assert!(s.mins.values[..5].iter().all(|v| *v == s.mins.values[0]));
    }
}
