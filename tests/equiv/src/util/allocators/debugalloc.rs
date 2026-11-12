//! Equivalence of `src/util/allocators/debugalloc.c` with `Allocator::debug`.
//!
//! - Values: `Allocator::debug` holds the `free` that `git_debugalloc_init_allocator` writes into a
//!   raw `git_allocator`, whatever the raw value held before, call after call, and guards in place
//!   of its `malloc` and `realloc`, which behave as C's but for the sizes C wraps.
//! - The family: the same steps run on the routines read from either value, on independently owned
//!   blocks, and are compared after each step: whether a call returned NULL, the length word each
//!   block starts with, the bytes a resize keeps, and the release of every block, which the
//!   `asan-ubsan` variant's `LeakSanitizer` and `ASan` check. Pointer values are unspecified and
//!   are not compared.
//! - Installed: libgit2 initialised on the family installed before it keeps it, and the strategies'
//!   blocks, and libgit2's own, carry its length word.
//!
//! A request `malloc` cannot meet aborts under `ASan` unless its allocator may return NULL,
//! and installing an allocator is process-global, so those steps run alone in child
//! processes, the former with `allocator_may_return_null=1` for `ASan`, `TSan` and
//! `BorrowSanitizer` alike.

use std::ffi::{c_char, c_int, c_void};
use std::process::Command;
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::oid::{Oid, OidShortener, OidType};
use libgit2::libgit2::src::libgit2::settings::set_allocator;
use libgit2::libgit2::src::util::alloc::{
    AllocBox, AllocVec, Allocator, ArrayFree, FreeFn, MallocFn, ReallocFn, StrFree,
};
use libgit2::libgit2::src::util::errors::{ErrorClass, clear, last};

const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

/// Runs `body` alone in a child process, with `options` appended to the sanitizers'
/// inherited options (`ASAN_OPTIONS`, `TSAN_OPTIONS`, `BSAN_OPTIONS`), so that no other test shares
/// libgit2's globals or those options.
fn isolated(test: &str, options: &str, body: fn()) {
    let name = test.strip_prefix("equiv::").unwrap_or(test);
    if std::env::var(CHILD).as_deref() == Ok(name) {
        body();
        return;
    }
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD, name);
    for var in ["ASAN_OPTIONS", "TSAN_OPTIONS", "BSAN_OPTIONS"] {
        let inherited = std::env::var(var).unwrap_or_default();
        let separator = if inherited.is_empty() || options.is_empty() {
            ""
        } else {
            ":"
        };
        child.env(var, format!("{inherited}{separator}{options}"));
    }
    let out = child.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child `{name}` failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// --- values ---------------------------------------------------------------------------

unsafe extern "C" fn other_malloc(_: usize, _: *const c_char, _: c_int) -> *mut c_void {
    ptr::null_mut()
}

unsafe extern "C" fn other_realloc(
    _: *mut c_void,
    _: usize,
    _: *const c_char,
    _: c_int,
) -> *mut c_void {
    ptr::null_mut()
}

unsafe extern "C" fn other_free(_: *mut c_void) {}

/// The family's routines as C writes them over `before`, and C's status.
fn raw_init(before: ffi::git_allocator) -> (c_int, ffi::git_allocator) {
    let mut raw = before;
    // SAFETY: `raw` is a live, writable `git_allocator`, which the routine only writes.
    let rc = unsafe { ffi::git_debugalloc_init_allocator(&raw mut raw) };
    (rc, raw)
}

/// Whether `rust` holds `c`'s `free`, and set members of its own, the guards, in place of
/// `c`'s `malloc` and `realloc`.
fn same_members(rust: Allocator, c: &ffi::git_allocator) -> bool {
    let h = rust.as_ref();
    let addr = |f: Option<usize>| f.filter(|&a| a != 0);
    let (malloc, realloc, free) = (
        addr(h.malloc().map(|f| f as usize)),
        addr(h.realloc().map(|f| f as usize)),
        addr(h.free().map(|f| f as usize)),
    );
    free.is_some()
        && free == c.gfree.map(|f| f as usize)
        && malloc.is_some_and(|m| Some(m) != c.gmalloc.map(|f| f as usize))
        && realloc.is_some_and(|r| Some(r) != c.grealloc.map(|f| f as usize))
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn debug_holds_cs_free_and_guards_over_any_value() {
    let (m, r, f): (MallocFn, ReallocFn, FreeFn) = (other_malloc, other_realloc, other_free);
    let befores = [
        ffi::git_allocator::default(),
        ffi::git_allocator {
            gmalloc: Some(m),
            grealloc: Some(r),
            gfree: Some(f),
        },
        ffi::git_allocator {
            gmalloc: Some(m),
            grealloc: None,
            gfree: Some(f),
        },
    ];
    for (step, before) in befores.into_iter().enumerate() {
        let (rc, raw) = raw_init(before);
        assert_eq!(rc, 0, "step {step}");
        assert!(same_members(Allocator::debug(), &raw), "step {step}");
        // A second call writes the same routines again: the family has no state.
        let (rc, again) = raw_init(raw);
        assert_eq!(rc, 0, "step {step}, again");
        assert!(
            same_members(Allocator::debug(), &again),
            "step {step}, again"
        );
    }
}

// --- the family -----------------------------------------------------------------------

/// The three routines of one value.
#[derive(Clone, Copy)]
struct Family {
    malloc: MallocFn,
    realloc: ReallocFn,
    free: FreeFn,
}

impl Family {
    fn of_rust() -> Self {
        let a = Allocator::debug();
        let h = a.as_ref();
        Self {
            malloc: h.malloc().unwrap(),
            realloc: h.realloc().unwrap(),
            free: h.free().unwrap(),
        }
    }

    fn of_c() -> Self {
        let (rc, raw) = raw_init(ffi::git_allocator::default());
        assert_eq!(rc, 0);
        Self {
            malloc: raw.gmalloc.unwrap(),
            realloc: raw.grealloc.unwrap(),
            free: raw.gfree.unwrap(),
        }
    }

    /// `malloc(len)`, naming this file as the call site, as libgit2 names its own.
    fn malloc(self, len: usize) -> *mut u8 {
        // SAFETY: `file` is a C string live for the call; C's own routine is called only
        // with sizes it does not wrap (`family_failures_return_null_and_keep_the_block`).
        unsafe { (self.malloc)(len, c"debugalloc.rs".as_ptr(), 1) }.cast()
    }

    /// `realloc(ptr, len)`.
    ///
    /// # Safety
    ///
    /// `ptr` is NULL or a live block of the family, given up on success; `len` as for
    /// `malloc`.
    unsafe fn realloc(self, ptr: *mut u8, len: usize) -> *mut u8 {
        // SAFETY: per this function's contract; a NULL `file` is one C ignores.
        unsafe { (self.realloc)(ptr.cast(), len, ptr::null(), 0) }.cast()
    }

    /// `free(ptr)`.
    ///
    /// # Safety
    ///
    /// `ptr` is NULL or a live block of the family, which the caller gives up.
    unsafe fn free(self, ptr: *mut u8) {
        // SAFETY: per this function's contract.
        unsafe { (self.free)(ptr.cast()) }
    }
}

/// The length word before a live block of the family.
///
/// # Safety
///
/// `p` is a live, non-NULL block of the family, which `malloc` placed a word into its block.
unsafe fn length_word(p: *const u8) -> usize {
    // SAFETY: per this function's contract, the word before `p` is the start of the block
    // `malloc` returned, in the same allocation.
    let word = unsafe { p.sub(size_of::<usize>()) };
    // SAFETY: as above; C wrote the word's bytes with `memcpy`, and a byte array needs no
    // alignment.
    let word = unsafe { word.cast::<[u8; size_of::<usize>()]>().read() };
    usize::from_ne_bytes(word)
}

/// The `len` bytes of a live block.
///
/// # Safety
///
/// `p` is a live block of at least `len` bytes, all of them written, unchanged while the
/// slice lives.
unsafe fn bytes<'a>(p: *const u8, len: usize) -> &'a [u8] {
    // SAFETY: per this function's contract.
    unsafe { std::slice::from_raw_parts(p, len) }
}

/// Fills the `len` bytes of a live block with a pattern of `seed`.
///
/// # Safety
///
/// `p` is a live block of at least `len` bytes that the caller owns.
unsafe fn fill(p: *mut u8, len: usize, seed: u8) {
    // SAFETY: per this function's contract, the `len` bytes at `p` are the caller's, and no
    // reference to them is live.
    let block = unsafe { std::slice::from_raw_parts_mut(p, len) };
    block.copy_from_slice(&pattern(len, seed));
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| seed.wrapping_add(i.to_le_bytes()[0]))
        .collect()
}

/// What a step left: whether the call returned NULL, and the length word and bytes of the
/// block it returned.
#[derive(Debug, PartialEq, Eq)]
enum Left {
    Null,
    Block {
        word: usize,
        aligned: bool,
        bytes: Vec<u8>,
    },
}

/// What the block `p` holds, its first `keep` bytes compared.
///
/// # Safety
///
/// `p` is NULL or a live block of the family whose first `keep` bytes are written.
unsafe fn left(p: *const u8, keep: usize) -> Left {
    if p.is_null() {
        return Left::Null;
    }
    Left::Block {
        // SAFETY: per this function's contract.
        word: unsafe { length_word(p) },
        aligned: p.addr().is_multiple_of(align_of::<usize>()),
        // SAFETY: per this function's contract.
        bytes: unsafe { bytes(p, keep) }.to_vec(),
    }
}

/// The block sizes the steps ask for: one byte, below and at a word, a struct, a page.
const SIZES: [usize; 5] = [1, 7, 8, 24, 4096];

/// One family's run of the steps: what each left, in order.
fn steps(family: Family) -> Vec<Left> {
    let mut out = Vec::new();
    // Zero-byte requests are failures, of `malloc` and of `realloc(NULL, 0)` alike.
    out.push(Left::from_null(family.malloc(0)));
    // SAFETY: a NULL `ptr`; zero bytes.
    out.push(Left::from_null(unsafe {
        family.realloc(ptr::null_mut(), 0)
    }));
    for (n, len) in SIZES.into_iter().enumerate() {
        let seed = u8::try_from(n).unwrap().wrapping_mul(37);
        let p = family.malloc(len);
        assert!(!p.is_null(), "malloc({len})");
        // SAFETY: a fresh block of `len` bytes, the caller's.
        unsafe { fill(p, len, seed) };
        // SAFETY: a live block, its `len` bytes written.
        out.push(unsafe { left(p, len) });

        // Grown: the block moves, its bytes and a new length word kept.
        // SAFETY: `p` is live and given up; `2 * len` is non-zero and small.
        let grown = unsafe { family.realloc(p, 2 * len) };
        assert!(
            !grown.is_null() && grown != p,
            "realloc({len} -> {})",
            2 * len
        );
        // SAFETY: a live block whose first `len` bytes C copied.
        out.push(unsafe { left(grown, len) });

        // Shrunk to half the original, at least a byte: the prefix kept.
        let half = len.div_ceil(2);
        // SAFETY: `grown` is live and given up; `half` is non-zero.
        let shrunk = unsafe { family.realloc(grown, half) };
        // SAFETY: a live block whose `half` bytes C copied.
        out.push(unsafe { left(shrunk, half) });

        // Resized to zero bytes: released, and NULL returned.
        // SAFETY: `shrunk` is live and given up; the family defines a zero-byte resize as a
        // release, which no other `ReallocFn` contract is relied on for here.
        out.push(Left::from_null(unsafe { family.realloc(shrunk, 0) }));

        // `realloc(NULL, len)` allocates afresh, copying nothing.
        // SAFETY: a NULL `ptr`; `len` is non-zero.
        let fresh = unsafe { family.realloc(ptr::null_mut(), len) };
        // SAFETY: a live block of `len` bytes, none read.
        out.push(unsafe { left(fresh, 0) });
        // SAFETY: `fresh` is live and given up.
        unsafe { family.free(fresh) };
    }
    // SAFETY: NULL, which releases nothing.
    unsafe { family.free(ptr::null_mut()) };
    out
}

impl Left {
    fn from_null(p: *mut u8) -> Self {
        assert!(p.is_null(), "expected NULL");
        Self::Null
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn family_routines_behave_alike_through_either_value() {
    let (rust, c) = (steps(Family::of_rust()), steps(Family::of_c()));
    assert_eq!(rust.len(), c.len());
    for (step, (r, c)) in rust.iter().zip(&c).enumerate() {
        assert_eq!(r, c, "step {step}");
    }
    // The steps' expectations, read off C's side: each block starts with its size, is
    // aligned to a word, and keeps the bytes a resize copies.
    let mut expected = vec![Left::Null, Left::Null];
    for (n, len) in SIZES.into_iter().enumerate() {
        let seed = u8::try_from(n).unwrap().wrapping_mul(37);
        let block = |word, keep| Left::Block {
            word,
            aligned: true,
            bytes: pattern(keep, seed),
        };
        let half = len.div_ceil(2);
        expected.extend([
            block(len, len),
            block(2 * len, len),
            block(half, half),
            Left::Null,
            Left::Block {
                word: len,
                aligned: true,
                bytes: Vec::new(),
            },
        ]);
    }
    assert_eq!(c, expected);
}

#[test]
#[cfg_attr(miri, ignore = "spawns a process, and calls C")]
fn family_failures_return_null_and_keep_the_block() {
    isolated(
        concat!(
            module_path!(),
            "::family_failures_return_null_and_keep_the_block"
        ),
        "allocator_may_return_null=1",
        || {
            // `isize::MAX` bytes, and the word C adds, are more than `malloc` can give, and
            // so is every size up to the last that C does not wrap, which the guards refuse
            // before C: either way the request fails.
            let huge = [
                isize::MAX.unsigned_abs(),
                isize::MAX.unsigned_abs() + 1,
                usize::MAX - size_of::<usize>(),
            ];
            for (family, huge) in [Family::of_rust(), Family::of_c()]
                .into_iter()
                .flat_map(|f| huge.map(|h| (f, h)))
            {
                assert!(family.malloc(huge).is_null(), "malloc({huge})");
                // SAFETY: a NULL `ptr`; `huge` is non-zero.
                assert!(unsafe { family.realloc(ptr::null_mut(), huge) }.is_null());
                let p = family.malloc(16);
                // SAFETY: a fresh block of 16 bytes.
                unsafe { fill(p, 16, 5) };
                // SAFETY: `p` is live, and stays the caller's on failure.
                let q = unsafe { family.realloc(p, huge) };
                assert!(q.is_null(), "realloc(p, {huge})");
                // The failed resize left `p` live and whole.
                // SAFETY: `p` is live, its 16 bytes written.
                let kept = unsafe { left(p, 16) };
                assert_eq!(
                    kept,
                    Left::Block {
                        word: 16,
                        aligned: true,
                        bytes: pattern(16, 5)
                    }
                );
                // SAFETY: `p` is live and given up.
                unsafe { family.free(p) };
            }
        },
    );
}

// --- installed ------------------------------------------------------------------------

#[test]
#[cfg_attr(miri, ignore = "spawns a process, and calls C")]
fn installed_before_init_libgit2_and_the_strategies_run_on_it() {
    isolated(
        concat!(
            module_path!(),
            "::installed_before_init_libgit2_and_the_strategies_run_on_it"
        ),
        "",
        || {
            let debug = Allocator::debug();
            // SAFETY: libgit2 is not initialised and holds no allocation, so every
            // allocation it ever makes is this family's, which meets `Alloc`'s contract;
            // this child runs one test on one thread.
            unsafe { set_allocator(Some(debug.as_ref())) };

            // The strategies pin the runtime, so libgit2 initialises over the installed
            // family and keeps it (`git_allocator_global_init`): each block starts with
            // the length word, on both sides.
            let words = AllocVec::<u64>::from_slice(&[1, 2, 3]).unwrap();
            assert_eq!(words.as_slice(), &[1, 2, 3]);
            // SAFETY: the run's block is the family's, live while `words` is.
            assert_eq!(unsafe { length_word(words.as_ptr().cast()) }, 24);
            // SAFETY: the primitive the run's strategy calls, for the same size.
            let raw = unsafe { ffi::git__malloc(24) };
            assert!(!raw.is_null());
            // SAFETY: a live block of the family.
            assert_eq!(unsafe { length_word(raw.cast()) }, 24);
            // SAFETY: the family's block, given up.
            unsafe { ffi::git__free(raw) };

            let s = StrFree::dup(c"debug").unwrap();
            // SAFETY: the string's block is the family's, live while `s` is: its terminator
            // included.
            assert_eq!(unsafe { length_word(s.as_ptr().cast()) }, 6);
            let id = AllocBox::<Oid>::new_zeroed().unwrap();
            // SAFETY: the box's block is the family's, live while `id` is.
            let id_word = unsafe { length_word(id.as_c_ptr().cast()) };
            assert_eq!(id_word, size_of::<ffi::git_oid>());
            assert!(id.as_ptr().addr().is_multiple_of(align_of::<usize>()));

            // A zero-byte request fails under this family, through the strategies and
            // through the primitives alike, with libgit2's out-of-memory error; the
            // standard allocator serves it.
            clear();
            assert!(ArrayFree::new_uninit::<u8>(0).is_none());
            assert_eq!(last().class(), Some(ErrorClass::NoMemory));
            clear();
            // SAFETY: no live allocation is involved; any size may be asked for.
            assert!(unsafe { ffi::git__mallocarray(0, 1) }.is_null());
            assert_eq!(last().class(), Some(ErrorClass::NoMemory));
            clear();
            assert!(ArrayFree::new_zeroed::<u64>(0).is_none());
            assert_eq!(last().class(), Some(ErrorClass::NoMemory));
            clear();
            // SAFETY: as above.
            assert!(unsafe { ffi::git__calloc(0, 8) }.is_null());
            assert_eq!(last().class(), Some(ErrorClass::NoMemory));
            clear();

            // libgit2's own C allocates through it too: a shortener's tree, and the error
            // message libgit2 formats, live in this family's blocks and are released
            // through it.
            let mut shortener = OidShortener::new(4).unwrap();
            let hexes = [
                "0123456789abcdef0123456789abcdef01234567",
                "0123456789abcdef0123456789abcdef01234568",
                "fedcba9876543210fedcba9876543210fedcba98",
            ];
            let mut length = 0;
            for hex in hexes {
                length = shortener.as_mut().add(hex).unwrap();
            }
            assert_eq!(length, 40);
            drop(shortener);
            let err = Oid::from_string(c"not hex", OidType::Sha1).unwrap_err();
            assert_ne!(err.message().to_bytes(), b"");
            clear();
        },
    );
}
