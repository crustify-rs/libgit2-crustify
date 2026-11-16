//! Port of `src/util/str.c`: `git_str`, libgit2's growable string buffer.
//!
//! [`GitStr`] is the mirrored `git_str` (kind 3: C embeds it by value
//! everywhere): `{ char *ptr; size_t asize; size_t size; }`, with the C's
//! growth policy (×1.5, rounded up to 8), so `asize` is the C's after the
//! same operations. Its buffer comes from `git__allocator`.
//!
//! # States
//!
//! A `git_str` is in one of these states, which every function accepts:
//!
//! | state | `ptr` | `asize` | `size` | contents |
//! |---|---|---|---|---|
//! | empty | `git_str__initstr`, NULL, or anything | 0 | 0 | none |
//! | owned | an allocation of `asize` bytes from `git__allocator` | > 0 | < `asize` | `size` bytes, then a NUL |
//! | borrowed (`GIT_STR_INIT_CONST`, `attach_notowned`) | `size` readable bytes it does not own | 0 | > 0 | `size` bytes, maybe not NUL-terminated |
//! | out of memory | `git_str__oom` | stale | stale | none: every operation fails with -1 |
//!
//! This is [`GitStr`]'s type invariant. Its safe constructors make only
//! empty and owned strings; a borrowed one comes from [`GitStr::borrowed`]
//! (tied to the borrow) or the `unsafe` [`GitStr::attach_notowned`]; C
//! hands exports strings in any state, as its contract with them.
//!
//! ```
//! use libgit2_rs::util::str::GitStr;
//! # let _g = libgit2_rs::util::test_support::init();
//! let mut s = GitStr::new();
//! s.puts(b"refs/heads")?;
//! s.joinpath_self(b"main")?;
//! assert_eq!(s.as_bytes(), b"refs/heads/main");
//! assert_eq!(s.capacity(), 16);
//! # Ok::<(), libgit2_rs::util::errors::ErrorCode>(())
//! ```

use core::cell::UnsafeCell;
use core::ffi::{CStr, c_char, c_int, c_uint};
use core::fmt;
use core::mem::MaybeUninit;
use core::ptr;

use super::alloc::{self, GitCStr};
use super::ctype_compat::{isspace, isxdigit};
use super::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use super::integer::{alloc_add, alloc_multiply};
use crate::{git_assert, git_error_set};

// ---------------------------------------------------------------------------
// The sentinels
// ---------------------------------------------------------------------------

/// A one-byte C array (`char git_str__initstr[1]`), always 0.
///
/// C may store 0 into it (`git_str_clear` of a stale out-of-memory string
/// does), so it lives in writable memory; Rust never reads or writes it, it
/// only compares addresses with it.
#[repr(transparent)]
pub struct Sentinel(UnsafeCell<c_char>);

// SAFETY: Rust never accesses the byte; C only ever stores the 0 it holds.
unsafe impl Sync for Sentinel {}

impl Sentinel {
    /// Its address, as C's `git_str__initstr` / `git_str__oom`.
    #[inline(always)]
    pub const fn as_ptr(&self) -> *mut c_char {
        self.0.get()
    }
}

/// The `ptr` of an empty `git_str` (`GIT_STR_INIT`).
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub static git_str__initstr: Sentinel = Sentinel(UnsafeCell::new(0));

/// The `ptr` of a `git_str` whose allocation failed.
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub static git_str__oom: Sentinel = Sentinel(UnsafeCell::new(0));

#[inline(always)]
fn initstr() -> *mut c_char {
    git_str__initstr.as_ptr()
}

#[inline(always)]
fn oomstr() -> *mut c_char {
    git_str__oom.as_ptr()
}

// ---------------------------------------------------------------------------
// The type
// ---------------------------------------------------------------------------

/// `git_str`, mirrored. See the [module documentation](self) for its
/// states. Dropping it frees an owned buffer (as `git_str_dispose`, which
/// leaves an empty string, so a drop after a C dispose frees nothing).
#[repr(C)]
pub struct GitStr {
    ptr: *mut c_char,
    asize: usize,
    size: usize,
}

// SAFETY: an owned buffer belongs to the `GitStr` alone and
// `git__allocator` frees from any thread; a borrowed one is readable from
// any thread for as long as the string borrows it (`borrowed`'s lifetime,
// `attach_notowned`'s contract); `&GitStr` only reads.
unsafe impl Send for GitStr {}
// SAFETY: as above: shared access only reads.
unsafe impl Sync for GitStr {}

/// `git_str_bom_t`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Bom {
    /// `GIT_STR_BOM_NONE`
    #[default]
    None = 0,
    /// `GIT_STR_BOM_UTF8`
    Utf8 = 1,
    /// `GIT_STR_BOM_UTF16_LE`
    Utf16Le = 2,
    /// `GIT_STR_BOM_UTF16_BE`
    Utf16Be = 3,
    /// `GIT_STR_BOM_UTF32_LE`
    Utf32Le = 4,
    /// `GIT_STR_BOM_UTF32_BE`
    Utf32Be = 5,
}

/// `git_str_text_stats`, mirrored.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextStats {
    /// The BOM at the head of the text.
    pub bom: Bom,
    /// NUL count.
    pub nul: c_uint,
    /// CR count.
    pub cr: c_uint,
    /// LF count.
    pub lf: c_uint,
    /// CRLF count.
    pub crlf: c_uint,
    /// Printable characters (approximate).
    pub printable: c_uint,
    /// Non-printable characters (approximate).
    pub nonprintable: c_uint,
}

/// Where the bytes an operation appends come from: outside the buffer, or
/// a run of its own contents (C allows `git_str_join(buf, '/', buf->ptr,
/// …)` and the like).
#[derive(Clone, Copy)]
pub(crate) enum Seg<'a> {
    /// Bytes elsewhere.
    Ext(&'a [u8]),
    /// The buffer's own bytes `off..off + len`, within its contents.
    Own { off: usize, len: usize },
}

impl Seg<'_> {
    #[inline]
    fn len(&self) -> usize {
        match *self {
            Seg::Ext(b) => b.len(),
            Seg::Own { len, .. } => len,
        }
    }
}

impl Default for GitStr {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for GitStr {
    fn drop(&mut self) {
        self.dispose();
    }
}

impl fmt::Debug for GitStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitStr")
            .field("contents", &Bytes(self.as_bytes()))
            .field("asize", &self.asize)
            .field("size", &self.size)
            .field("oom", &self.is_oom())
            .finish()
    }
}

/// Appending through `write!`: each piece as [`GitStr::put`]. (For the
/// C's growth, and so its `asize`, use [`GitStr::printf`].)
impl fmt::Write for GitStr {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.put(s.as_bytes()).map_err(|_| fmt::Error)
    }
}

impl GitStr {
    /// An empty string (`GIT_STR_INIT`). Does not allocate.
    pub const fn new() -> GitStr {
        GitStr {
            ptr: git_str__initstr.0.get(),
            asize: 0,
            size: 0,
        }
    }

    /// An empty string with room for `initial_size` bytes
    /// (`git_str_init`).
    pub fn with_capacity(initial_size: usize) -> Result<GitStr> {
        let mut s = GitStr::new();
        s.ensure(initial_size)?;
        Ok(s)
    }

    /// A string with a copy of `data`.
    pub fn from_bytes(data: &[u8]) -> Result<GitStr> {
        let mut s = GitStr::new();
        s.put(data)?;
        Ok(s)
    }

    /// A borrowed view of `data` (`GIT_STR_INIT_CONST`), for the
    /// functions that read a `git_str`.
    pub fn borrowed(data: &[u8]) -> GitStrRef<'_> {
        GitStrRef {
            s: GitStr {
                ptr: data.as_ptr().cast_mut().cast(),
                asize: 0,
                size: data.len(),
            },
            _data: core::marker::PhantomData,
        }
    }

    /// A string from its fields.
    ///
    /// # Safety
    ///
    /// The fields are in one of the states of the [module
    /// documentation](self), and an owned buffer's ownership passes to the
    /// string.
    pub const unsafe fn from_raw_parts(ptr: *mut c_char, asize: usize, size: usize) -> GitStr {
        GitStr { ptr, asize, size }
    }

    /// Take the fields, leaving the string empty without freeing anything:
    /// the buffer (if owned) is now the caller's.
    pub fn take_raw(&mut self) -> (*mut c_char, usize, usize) {
        let r = (self.ptr, self.asize, self.size);
        self.ptr = initstr();
        self.asize = 0;
        self.size = 0;
        r
    }

    // ----- accessors -----

    /// The raw pointer (`git_str_cstr`); see the states.
    #[inline]
    pub fn as_ptr(&self) -> *const c_char {
        self.ptr
    }

    /// The string, as C passes it: `&mut git_str`.
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut GitStr {
        self
    }

    /// `size` (`git_str_len`). It is stale for an out-of-memory string.
    #[inline]
    pub fn len(&self) -> usize {
        self.size
    }

    /// Whether `size` is 0.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// `asize`: the allocation's size, 0 unless owned.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.asize
    }

    /// `git_str_oom`.
    #[inline]
    pub fn is_oom(&self) -> bool {
        self.ptr == oomstr()
    }

    /// `git_str_is_allocated`.
    #[inline]
    pub fn is_allocated(&self) -> bool {
        !self.ptr.is_null() && self.asize > 0
    }

    /// Whether the buffer is an allocation this string owns.
    #[inline]
    fn owned(&self) -> bool {
        self.asize > 0 && !self.ptr.is_null() && !self.is_oom()
    }

    /// The contents; empty for an out-of-memory string.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        if self.size == 0 || self.ptr.is_null() || self.is_oom() {
            return &[];
        }
        let n = if self.asize > 0 {
            self.size.min(self.asize)
        } else {
            self.size
        };
        // SAFETY: by the type invariant, an owned buffer holds `size <
        // asize` initialized bytes (clamped to `asize` regardless), and a
        // borrowed one `size` readable bytes, for as long as `self` is
        // borrowed; nothing writes them meanwhile, as `&self` is shared.
        unsafe { core::slice::from_raw_parts(self.ptr.cast::<u8>(), n) }
    }

    /// The contents as a C string: up to the first NUL of an owned buffer,
    /// `""` for an empty or out-of-memory one, and `None` for a borrowed one
    /// (which need not be NUL-terminated).
    pub fn as_c_str(&self) -> Option<&CStr> {
        if !self.owned() {
            return if self.asize == 0 && self.size > 0 && !self.is_oom() {
                None
            } else {
                Some(c"")
            };
        }
        let n = self.size.min(self.asize - 1);
        // SAFETY: an owned buffer's bytes `0..=size` are initialized (the
        // contents and the NUL), within the allocation.
        let b = unsafe { core::slice::from_raw_parts(self.ptr.cast::<u8>(), n + 1) };
        CStr::from_bytes_until_nul(b).ok()
    }

    /// The contents, mutable, of an owned buffer (empty otherwise).
    #[inline]
    fn content_mut(&mut self) -> &mut [u8] {
        if !self.owned() || self.size == 0 {
            return &mut [];
        }
        let n = self.size.min(self.asize);
        // SAFETY: an owned buffer is this string's alone, with `size`
        // initialized bytes; `&mut self` makes this the only access.
        unsafe { core::slice::from_raw_parts_mut(self.ptr.cast::<u8>(), n) }
    }

    /// The whole allocation of an owned buffer (empty otherwise), for
    /// writing.
    #[inline]
    fn alloc_mut(&mut self) -> &mut [MaybeUninit<u8>] {
        if !self.owned() {
            return &mut [];
        }
        // SAFETY: an owned buffer is an allocation of `asize` bytes that
        // this string alone owns; `&mut self` makes this the only access.
        // `MaybeUninit` admits the unwritten bytes past the contents.
        unsafe { core::slice::from_raw_parts_mut(self.ptr.cast::<MaybeUninit<u8>>(), self.asize) }
    }

    /// Write `data` at `at` (within the allocation).
    #[inline]
    fn write_at(&mut self, at: usize, data: &[u8]) {
        self.alloc_mut()[at..at + data.len()].write_copy_of_slice(data);
    }

    /// Write the byte `c` at `at`.
    #[inline]
    fn write_byte(&mut self, at: usize, c: u8) {
        self.alloc_mut()[at].write(c);
    }

    /// Copy the segment `seg` to `at` (after the room for it was made).
    #[inline]
    fn write_seg(&mut self, at: usize, seg: Seg<'_>) {
        match seg {
            Seg::Ext(b) => self.write_at(at, b),
            Seg::Own { off, len } => self.alloc_mut().copy_within(off..off + len, at),
        }
    }

    /// Set the size to `n` and write the NUL after it. Every byte before
    /// `n` must have been written (this is how the module keeps the
    /// contents initialized).
    #[inline]
    fn set_size_nul(&mut self, n: usize) {
        self.write_byte(n, 0);
        self.size = n;
    }

    /// The byte at `i` of the contents.
    #[inline]
    fn byte(&self, i: usize) -> u8 {
        self.as_bytes()[i]
    }

    // ----- growth -----

    /// `git_str_try_grow`: make room for `target_size` bytes (the NUL
    /// included; 0 means `size`), growing by ×1.5 rounded up to 8. On
    /// allocation failure with `mark_oom`, the buffer is freed and the
    /// string becomes out of memory. A borrowed string fails with
    /// `GIT_EINVALID` ("cannot grow a borrowed buffer").
    pub fn try_grow(&mut self, target_size: usize, mark_oom: bool) -> Result<()> {
        if self.is_oom() {
            return Err(ErrorCode::GENERIC);
        }
        if self.asize == 0 && self.size != 0 {
            let _ = errors::set_str(ErrorClass::INVALID, "cannot grow a borrowed buffer");
            return Err(ErrorCode::INVALID);
        }
        let target = if target_size == 0 {
            self.size
        } else {
            target_size
        };
        if target <= self.asize {
            return Ok(());
        }
        let (mut new_size, old) = if self.asize == 0 {
            (target, ptr::null_mut())
        } else {
            let n = self.asize;
            let grown = (n << 1).wrapping_sub(n >> 1);
            (if grown < target { target } else { grown }, self.ptr)
        };
        new_size = new_size.wrapping_add(7) & !7;

        // A rounding that wrapped to 0 is an overflow too (C goes on and
        // writes `ptr[SIZE_MAX]`; rust/DEFECTS.md).
        if new_size < self.size || new_size == 0 {
            if mark_oom {
                self.mark_oom();
            }
            errors::set_oom();
            return Err(ErrorCode::GENERIC);
        }

        // SAFETY: `old` is NULL or this string's owned allocation, which
        // `realloc` may move; on failure it stays ours.
        let p = unsafe { alloc::realloc(old.cast(), new_size) }.cast::<c_char>();
        if p.is_null() {
            if mark_oom {
                self.mark_oom();
            }
            return Err(ErrorCode::GENERIC);
        }
        self.asize = new_size;
        self.ptr = p;
        if self.size >= self.asize {
            self.size = self.asize - 1;
        }
        let n = self.size;
        self.write_byte(n, 0);
        Ok(())
    }

    /// Free an owned buffer and become out of memory.
    fn mark_oom(&mut self) {
        if self.owned() {
            // SAFETY: the owned allocation, freed once; `ptr` is replaced.
            unsafe { alloc::free(self.ptr.cast()) };
        }
        self.ptr = oomstr();
    }

    /// `git_str_grow`.
    pub fn grow(&mut self, target_size: usize) -> Result<()> {
        self.try_grow(target_size, true)
    }

    /// `git_str_grow_by`: room for `additional_size` more bytes than
    /// `size`.
    pub fn grow_by(&mut self, additional_size: usize) -> Result<()> {
        let Some(newsize) = self.size.checked_add(additional_size) else {
            errors::set_oom();
            // C leaks the buffer here; it is freed (rust/DEFECTS.md).
            self.mark_oom();
            return Err(ErrorCode::GENERIC);
        };
        self.try_grow(newsize, true)
    }

    /// C's `ENSURE_SIZE`: -1 if out of memory or if growing to `d` fails.
    #[inline]
    fn ensure(&mut self, d: usize) -> Result<()> {
        if self.is_oom() || (d > self.asize && self.grow(d).is_err()) {
            return Err(ErrorCode::GENERIC);
        }
        Ok(())
    }

    // ----- lifetime -----

    /// `git_str_dispose`: free an owned buffer and become empty.
    pub fn dispose(&mut self) {
        if self.asize > 0 && !self.ptr.is_null() && !self.is_oom() {
            // SAFETY: the owned allocation, freed once; `ptr` is replaced.
            unsafe { alloc::free(self.ptr.cast()) };
        }
        self.ptr = initstr();
        self.asize = 0;
        self.size = 0;
    }

    /// `git_str_clear`: empty the contents, keeping the allocation.
    pub fn clear(&mut self) {
        self.size = 0;
        if self.ptr.is_null() {
            self.ptr = initstr();
            self.asize = 0;
        }
        if self.owned() {
            self.write_byte(0, 0);
        }
    }

    /// `git_str_detach`: take the buffer, leaving the string empty; `None`
    /// unless owned.
    pub fn detach(&mut self) -> Option<GitCStr> {
        let p = self.detach_raw();
        // SAFETY: `p` is NULL or an owned allocation of `git__allocator`,
        // NUL-terminated within it, now the caller's.
        unsafe { GitCStr::from_raw(p) }
    }

    fn detach_raw(&mut self) -> *mut c_char {
        if self.asize == 0 || self.is_oom() {
            return ptr::null_mut();
        }
        let data = self.ptr;
        self.ptr = initstr();
        self.asize = 0;
        self.size = 0;
        data
    }

    /// `git_str_attach`: dispose, then adopt the NUL-terminated `ptr`
    /// (NULL leaves the string empty), whose allocation is `asize` bytes (0:
    /// `strlen + 1`); then make room for `asize`.
    ///
    /// # Safety
    ///
    /// `ptr` is NULL, or a NUL-terminated string in an allocation of
    /// `git__allocator` of at least `max(asize, strlen + 1)` bytes, whose
    /// ownership passes to the string.
    pub unsafe fn attach(&mut self, ptr: *mut c_char, asize: usize) -> Result<()> {
        self.dispose();
        if !ptr.is_null() {
            // SAFETY: the caller passes a NUL-terminated string.
            let len = unsafe { CStr::from_ptr(ptr) }.count_bytes();
            self.ptr = ptr;
            self.size = len;
            self.asize = if asize != 0 {
                if asize < len { len + 1 } else { asize }
            } else {
                len + 1
            };
        }
        self.ensure(asize)
    }

    /// `git_str_attach_notowned`: dispose an owned buffer, then borrow the
    /// `size` bytes at `ptr` (or become empty for 0).
    ///
    /// # Safety
    ///
    /// For `size > 0`, `ptr` is valid for reads of `size` bytes, from any
    /// thread, until the string is disposed, cleared, or attached again, and
    /// nothing writes them meanwhile.
    pub unsafe fn attach_notowned(&mut self, ptr: *const c_char, size: usize) {
        if self.is_allocated() {
            self.dispose();
        }
        if size == 0 {
            self.ptr = initstr();
            self.asize = 0;
            self.size = 0;
        } else {
            self.ptr = ptr.cast_mut();
            self.asize = 0;
            self.size = size;
        }
    }

    /// `git_str_swap`.
    pub fn swap(&mut self, other: &mut GitStr) {
        core::mem::swap(self, other);
    }

    // ----- setting and appending -----

    /// `git_str_set`: replace the contents with `data`.
    pub fn set(&mut self, data: &[u8]) -> Result<()> {
        self.set_seg(Seg::Ext(data))
    }

    pub(crate) fn set_seg(&mut self, data: Seg<'_>) -> Result<()> {
        let len = data.len();
        if len == 0 {
            self.clear();
            return Ok(());
        }
        if let Seg::Own { off: 0, .. } = data {
            // C's `data == buf->ptr`: the bytes are in place.
        } else {
            let alloclen = alloc_add(len, 1)?;
            self.ensure(alloclen)?;
            self.write_seg(0, data);
        }
        self.size = len;
        if self.asize > self.size {
            self.write_byte(len, 0);
        }
        Ok(())
    }

    /// `git_str_sets`.
    pub fn sets(&mut self, s: &[u8]) -> Result<()> {
        self.set(s)
    }

    /// `git_str_putc`.
    #[inline]
    pub fn putc(&mut self, c: u8) -> Result<()> {
        let new_size = alloc_add(self.size, 2)?;
        self.ensure(new_size)?;
        let n = self.size;
        self.write_byte(n, c);
        self.set_size_nul(n + 1);
        Ok(())
    }

    /// `git_str_putcn`: append `len` copies of `c`.
    pub fn putcn(&mut self, c: u8, len: usize) -> Result<()> {
        let new_size = alloc_add(alloc_add(self.size, len)?, 1)?;
        self.ensure(new_size)?;
        let n = self.size;
        for b in &mut self.alloc_mut()[n..n + len] {
            b.write(c);
        }
        self.set_size_nul(n + len);
        Ok(())
    }

    /// `git_str_put`: append `data`.
    #[inline]
    pub fn put(&mut self, data: &[u8]) -> Result<()> {
        self.put_seg(Seg::Ext(data))
    }

    #[inline]
    pub(crate) fn put_seg(&mut self, data: Seg<'_>) -> Result<()> {
        let len = data.len();
        if len != 0 {
            let new_size = alloc_add(alloc_add(self.size, len)?, 1)?;
            self.ensure(new_size)?;
            let n = self.size;
            self.write_seg(n, data);
            self.set_size_nul(n + len);
        }
        Ok(())
    }

    /// `git_str_puts`.
    #[inline]
    pub fn puts(&mut self, s: &[u8]) -> Result<()> {
        self.put(s)
    }

    /// `git_str_printf` for Rust callers: append formatted text, with the
    /// C's growth: first room for `2 * format_len` more bytes (`format_len`
    /// is the length of the C format string the call replaces; 0 skips
    /// this), then, if the text did not fit, exactly enough for it. Fails
    /// with -1 as C's does.
    ///
    /// Arguments are formatted with Rust's `Display`; byte strings that are
    /// not UTF-8 should be appended with [`put`](Self::put).
    pub fn printf(&mut self, format_len: usize, args: fmt::Arguments<'_>) -> Result<()> {
        self.vprintf_begin(format_len)?;
        loop {
            let n = self.size;
            let mut w = SpareWriter {
                buf: &mut self.alloc_mut()[n..],
                len: 0,
            };
            if fmt::write(&mut w, args).is_err() {
                return Err(ErrorCode::GENERIC);
            }
            let len = w.len;
            if self.owned() && len < self.asize.saturating_sub(self.size) {
                // Everything was written: count it and add the NUL.
                self.set_size_nul(n + len);
                return Ok(());
            }
            let new_size = alloc_add(alloc_add(self.size, len)?, 1)?;
            self.ensure(new_size)?;
        }
    }

    // ----- encodings -----

    /// `git_str_encode_hexstr`: append `data` as lowercase hex.
    pub fn encode_hexstr(&mut self, data: &[u8]) -> Result<()> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let new_size = alloc_add(alloc_multiply(data.len(), 2)?, 1)?;
        self.grow_by(new_size).map_err(|_| ErrorCode::GENERIC)?;
        let n = self.size;
        let out = &mut self.alloc_mut()[n..n + data.len() * 2];
        for (o, &b) in out.as_chunks_mut::<2>().0.iter_mut().zip(data) {
            o[0].write(HEX[usize::from(b >> 4)]);
            o[1].write(HEX[usize::from(b & 0x0f)]);
        }
        self.set_size_nul(n + data.len() * 2);
        Ok(())
    }

    /// `git_str_encode_base64`: append `data` in base64.
    pub fn encode_base64(&mut self, data: &[u8]) -> Result<()> {
        const ENC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let extra = data.len() % 3;
        let blocks = alloc_add(data.len() / 3 + usize::from(extra != 0), 1)?;
        let alloclen = alloc_add(alloc_multiply(blocks, 4)?, self.size)?;
        self.ensure(alloclen)?;
        let mut w = self.size;
        let buf = self.alloc_mut();
        let (full, rest) = data.split_at(data.len() - extra);
        for c in full.as_chunks::<3>().0 {
            let (a, b, cc) = (c[0], c[1], c[2]);
            buf[w].write(ENC[usize::from(a >> 2)]);
            buf[w + 1].write(ENC[usize::from((a & 0x03) << 4 | b >> 4)]);
            buf[w + 2].write(ENC[usize::from((b & 0x0f) << 2 | cc >> 6)]);
            buf[w + 3].write(ENC[usize::from(cc & 0x3f)]);
            w += 4;
        }
        if extra > 0 {
            let a = rest[0];
            let b = if extra > 1 { rest[1] } else { 0 };
            buf[w].write(ENC[usize::from(a >> 2)]);
            buf[w + 1].write(ENC[usize::from((a & 0x03) << 4 | b >> 4)]);
            buf[w + 2].write(if extra > 1 {
                ENC[usize::from((b & 0x0f) << 2)]
            } else {
                b'='
            });
            buf[w + 3].write(b'=');
            w += 4;
        }
        self.set_size_nul(w);
        Ok(())
    }

    /// `git_str_decode_base64`: append the decoding of `base64`; on invalid
    /// input the string is unchanged and the error is `invalid base64
    /// input`.
    pub fn decode_base64(&mut self, base64: &[u8]) -> Result<()> {
        const DEC: [i8; 256] = base64_decode_table();
        let len = base64.len();
        if !len.is_multiple_of(4) {
            return Err(git_error_set!(ErrorClass::INVALID, "invalid base64 input"));
        }
        let orig_size = self.size;
        let new_size = alloc_add(alloc_add(len / 4 * 3, self.size)?, 1)?;
        self.ensure(new_size)?;
        let mut w = orig_size;
        for q in base64.as_chunks::<4>().0 {
            let (a, b, c, d) = (
                DEC[usize::from(q[0])],
                DEC[usize::from(q[1])],
                DEC[usize::from(q[2])],
                DEC[usize::from(q[3])],
            );
            if a < 0 || b < 0 || c < 0 || d < 0 {
                self.size = orig_size;
                self.write_byte(orig_size, 0);
                return Err(git_error_set!(ErrorClass::INVALID, "invalid base64 input"));
            }
            let (a, b, c, d) = (a as u8, b as u8, c as u8, d as u8);
            let buf = self.alloc_mut();
            buf[w].write((a << 2) | (b & 0x30) >> 4);
            buf[w + 1].write(((b & 0x0f) << 4) | ((c & 0x3c) >> 2));
            buf[w + 2].write((c & 0x03) << 6 | (d & 0x3f));
            w += 3;
            // Keep the contents initialized as they grow.
            self.size = w;
        }
        self.set_size_nul(w);
        Ok(())
    }

    /// `git_str_encode_base85`: append `data` in git's base85.
    pub fn encode_base85(&mut self, data: &[u8]) -> Result<()> {
        const ENC: &[u8; 85] =
            b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz!#$%&()*+-;<=>?@^_`{|}~";
        let blocks = data.len().div_ceil(4);
        let alloclen = alloc_add(alloc_add(alloc_multiply(blocks, 5)?, self.size)?, 1)?;
        self.ensure(alloclen)?;
        let mut w = self.size;
        for chunk in data.chunks(4) {
            let mut acc: u32 = 0;
            for (k, &ch) in chunk.iter().enumerate() {
                acc |= u32::from(ch) << (24 - 8 * k);
            }
            let mut b85 = [0u8; 5];
            for slot in b85.iter_mut().rev() {
                *slot = ENC[(acc % 85) as usize];
                acc /= 85;
            }
            self.write_at(w, &b85);
            w += 5;
        }
        self.set_size_nul(w);
        Ok(())
    }

    /// `git_str_decode_base85`: append the `output_len` bytes `base85`
    /// decodes to; on invalid input the string is unchanged and the error
    /// is `invalid base85 input`.
    pub fn decode_base85(&mut self, base85: &[u8], output_len: usize) -> Result<()> {
        const DEC: [i8; 256] = base85_decode_table();
        let base85_len = base85.len();
        if !base85_len.is_multiple_of(5) || output_len > base85_len.wrapping_mul(4) / 5 {
            return Err(git_error_set!(ErrorClass::INVALID, "invalid base85 input"));
        }
        let orig_size = self.size;
        let new_size = alloc_add(alloc_add(output_len, self.size)?, 1)?;
        self.ensure(new_size)?;

        let mut w = orig_size;
        let mut rd = 0usize;
        let mut output_len = output_len;
        let fail = |s: &mut GitStr| {
            s.size = orig_size;
            s.write_byte(orig_size, 0);
            Err(git_error_set!(ErrorClass::INVALID, "invalid base85 input"))
        };
        while output_len > 0 {
            let mut acc: u32 = 0;
            for _ in 0..4 {
                let de = i32::from(DEC[usize::from(base85[rd])]) - 1;
                rd += 1;
                if de < 0 {
                    return fail(self);
                }
                acc = acc.wrapping_mul(85).wrapping_add(de as u32);
            }
            let de = i32::from(DEC[usize::from(base85[rd])]) - 1;
            rd += 1;
            if de < 0 {
                return fail(self);
            }
            if 0xffff_ffff / 85 < acc {
                return fail(self);
            }
            acc = acc.wrapping_mul(85);
            if 0xffff_ffff - (de as u32) < acc {
                return fail(self);
            }
            acc = acc.wrapping_add(de as u32);

            let cnt = output_len.min(4);
            output_len -= cnt;
            for _ in 0..cnt {
                acc = acc.rotate_left(8);
                self.write_byte(w, acc as u8);
                w += 1;
                self.size = w;
            }
        }
        self.set_size_nul(w);
        Ok(())
    }

    /// `git_str_decode_percent`: append `s` with `%XX` escapes decoded.
    pub fn decode_percent(&mut self, s: &[u8]) -> Result<()> {
        let new_size = alloc_add(alloc_add(self.size, s.len())?, 1)?;
        self.ensure(new_size)?;
        let hex = |c: u8| -> u8 { ((i32::from(c | 32) % 39) - 9) as u8 };
        let mut w = self.size;
        let mut pos = 0;
        while pos < s.len() {
            let b = if s[pos] == b'%'
                && s.len() > pos + 2
                && isxdigit(s[pos + 1])
                && isxdigit(s[pos + 2])
            {
                let v = (hex(s[pos + 1]) << 4).wrapping_add(hex(s[pos + 2]));
                pos += 2;
                v
            } else {
                s[pos]
            };
            self.write_byte(w, b);
            w += 1;
            self.size = w;
            pos += 1;
        }
        self.set_size_nul(w);
        Ok(())
    }

    // ----- vprintf support (the C shim drives these) -----

    /// The first step of the `git_str_vprintf` shim: make room for twice
    /// the format's length.
    pub(crate) fn vprintf_begin(&mut self, format_len: usize) -> Result<()> {
        let expected = alloc_add(alloc_multiply(format_len, 2)?, self.size)?;
        self.ensure(expected)
    }

    /// The shim's step after `vsnprintf` wrote (or would have written)
    /// `len` bytes at `size`: `Ok(true)` when they fit (and are now
    /// counted), `Ok(false)` after making room to retry.
    ///
    /// # Safety
    ///
    /// When `len >= 0` and `len + 1 <= asize - size`, `vsnprintf` has
    /// written `len` bytes and a NUL at `size`.
    pub(crate) unsafe fn vprintf_written(&mut self, len: c_int) -> Result<bool> {
        let Ok(len) = usize::try_from(len) else {
            // C frees the buffer even when it is not owned (rust/DEFECTS.md).
            self.mark_oom();
            return Err(ErrorCode::GENERIC);
        };
        if self.owned() && len < self.asize.saturating_sub(self.size) {
            self.size += len;
            return Ok(true);
        }
        let new_size = alloc_add(alloc_add(self.size, len)?, 1)?;
        self.ensure(new_size)?;
        Ok(false)
    }

    // ----- truncation -----

    /// `git_str_copy_cstr`: copy the contents, truncated, NUL-terminated,
    /// into `data` (which must not be empty). Copies nothing from a
    /// borrowed string, as C.
    pub fn copy_cstr(&self, data: &mut [u8]) -> Result<()> {
        crate::git_assert_arg!(!data.is_empty(), "datasize");
        data[0] = 0;
        if self.size == 0 || self.asize == 0 {
            return Ok(());
        }
        let b = self.as_bytes();
        let n = b.len().min(data.len() - 1);
        data[..n].copy_from_slice(&b[..n]);
        data[n] = 0;
        Ok(())
    }

    /// `git_str_consume_bytes`: drop the first `len` bytes (nothing unless
    /// `0 < len <= size`).
    pub fn consume_bytes(&mut self, len: usize) {
        if len == 0 || len > self.size || self.is_oom() || self.ptr.is_null() {
            return;
        }
        if self.owned() {
            let size = self.size;
            self.alloc_mut().copy_within(len..size, 0);
            self.set_size_nul(size - len);
        } else {
            // A borrowed buffer is not written through: the view moves
            // instead (rust/DIVERGENCES.md).
            self.ptr = self.ptr.wrapping_add(len);
            self.size -= len;
        }
    }

    /// `git_str_truncate`.
    pub fn truncate(&mut self, len: usize) {
        if len >= self.size {
            return;
        }
        self.size = len;
        if self.size < self.asize && self.owned() {
            self.write_byte(len, 0);
        }
    }

    /// `git_str_shorten`.
    pub fn shorten(&mut self, amount: usize) {
        if self.size > amount {
            self.truncate(self.size - amount);
        } else {
            self.clear();
        }
    }

    /// `git_str_truncate_at_char`.
    pub fn truncate_at_char(&mut self, separator: u8) {
        if let Some(idx) = self.find(separator) {
            self.truncate(idx);
        }
    }

    /// `git_str_rtruncate_at_char`.
    pub fn rtruncate_at_char(&mut self, separator: u8) {
        let idx = self.rfind_next(separator);
        self.truncate(idx.unwrap_or(0));
    }

    /// `git_str_find`.
    pub fn find(&self, ch: u8) -> Option<usize> {
        memchr::memchr(ch, self.as_bytes())
    }

    /// `git_str_rfind`.
    pub fn rfind(&self, ch: u8) -> Option<usize> {
        memchr::memrchr(ch, self.as_bytes())
    }

    /// `git_str_rfind_next`: the last `ch` before the trailing run of
    /// `ch`s.
    pub fn rfind_next(&self, ch: u8) -> Option<usize> {
        let b = self.as_bytes();
        let mut idx = b.len();
        while idx > 0 && b[idx - 1] == ch {
            idx -= 1;
        }
        while idx > 0 && b[idx - 1] != ch {
            idx -= 1;
        }
        idx.checked_sub(1)
    }

    /// `git_str_rtrim`: drop trailing whitespace.
    pub fn rtrim(&mut self) {
        let mut n = self.size;
        {
            let b = self.as_bytes();
            n = n.min(b.len());
            while n > 0 && isspace(b[n - 1]) {
                n -= 1;
            }
        }
        self.size = n;
        if self.asize > self.size && self.owned() {
            self.write_byte(n, 0);
        }
    }

    // ----- joining -----

    /// `git_str_join_n` over byte strings: append `segments` separated by
    /// `separator`, without doubling separators. A `None` segment is
    /// skipped, as C skips NULL, but still counts as a segment.
    pub fn join_n(&mut self, separator: u8, segments: &[Option<&[u8]>]) -> Result<()> {
        let segs: Vec<Option<Seg<'_>>> = segments.iter().map(|s| s.map(Seg::Ext)).collect();
        self.join_segs(separator, &segs)
    }

    pub(crate) fn join_segs(&mut self, separator: u8, segs: &[Option<Seg<'_>>]) -> Result<()> {
        let mut total_size = 0usize;
        if self.size > 0 && self.byte(self.size - 1) != separator {
            total_size += 1;
        }
        for seg in segs.iter().flatten() {
            let len = seg.len();
            total_size = alloc_add(total_size, len)?;
            if len == 0 || self.seg_byte(seg, len - 1) != separator {
                total_size = alloc_add(total_size, 1)?;
            }
        }
        if total_size == 0 {
            return Ok(());
        }
        total_size = alloc_add(total_size, 1)?;
        self.grow_by(total_size).map_err(|_| ErrorCode::GENERIC)?;

        let mut out = self.size;
        if self.size > 0 && self.byte(self.size - 1) != separator {
            self.write_byte(out, separator);
            out += 1;
            self.size = out;
        }
        let n = segs.len();
        for (i, seg) in segs.iter().enumerate() {
            let Some(seg) = *seg else { continue };
            let mut seg = seg;
            if out > 0 && self.byte(out - 1) == separator {
                while seg.len() > 0 && self.seg_byte(&seg, 0) == separator {
                    seg = match seg {
                        Seg::Ext(b) => Seg::Ext(&b[1..]),
                        Seg::Own { off, len } => Seg::Own {
                            off: off + 1,
                            len: len - 1,
                        },
                    };
                }
            }
            let len = seg.len();
            if len > 0 {
                self.write_seg(out, seg);
                out += len;
                self.size = out;
            }
            if i + 1 < n && out > 0 && self.byte(out - 1) != separator {
                self.write_byte(out, separator);
                out += 1;
                self.size = out;
            }
        }
        self.set_size_nul(out);
        Ok(())
    }

    #[inline]
    fn seg_byte(&self, seg: &Seg<'_>, i: usize) -> u8 {
        match *seg {
            Seg::Ext(b) => b[i],
            Seg::Own { off, .. } => self.byte(off + i),
        }
    }

    /// `git_str_join`: set the contents to `a`, `separator` and `b`,
    /// without doubling the separator (`a` may be `None`, C's NULL).
    pub fn join(&mut self, separator: u8, a: Option<&[u8]>, b: &[u8]) -> Result<()> {
        self.join_seg(separator, a.map(Seg::Ext), b)
    }

    /// `git_str_joinpath(buf, buf->ptr, b)`: the C idiom of appending a
    /// path component to the string itself.
    pub fn joinpath_self(&mut self, b: &[u8]) -> Result<()> {
        let a = if self.size == 0 {
            Seg::Ext(&[])
        } else {
            Seg::Own {
                off: 0,
                len: self.as_bytes().len(),
            }
        };
        self.join_seg(b'/', Some(a), b)
    }

    /// `git_str_joinpath`.
    pub fn joinpath(&mut self, a: &[u8], b: &[u8]) -> Result<()> {
        self.join(b'/', Some(a), b)
    }

    pub(crate) fn join_seg(&mut self, separator: u8, a: Option<Seg<'_>>, b: &[u8]) -> Result<()> {
        let strlen_a = a.as_ref().map_or(0, Seg::len);
        let mut b = b;
        let mut need_sep = 0usize;
        if separator != 0 && strlen_a > 0 {
            while b.first() == Some(&separator) {
                b = &b[1..];
            }
            let a = a.as_ref().expect("strlen_a > 0");
            if self.seg_byte(a, strlen_a - 1) != separator {
                need_sep = 1;
            }
        }
        let alloc_len = alloc_add(alloc_add(alloc_add(strlen_a, b.len())?, need_sep)?, 1)?;
        self.ensure(alloc_len)?;
        if let Some(a) = a
            && !matches!(a, Seg::Own { off: 0, .. })
            && strlen_a > 0
        {
            self.write_seg(0, a);
        }
        if need_sep == 1 {
            self.write_byte(strlen_a, separator);
        }
        self.write_at(strlen_a + need_sep, b);
        self.set_size_nul(strlen_a + b.len() + need_sep);
        Ok(())
    }

    /// `git_str_join3`: `a`, `b`, `c` separated by `separator`, without
    /// doubling it.
    pub fn join3(&mut self, separator: u8, a: &[u8], b: &[u8], c: &[u8]) -> Result<()> {
        let (mut b, mut c) = (b, c);
        let (mut sep_a, mut sep_b) = (0usize, 0usize);
        if separator != 0 {
            if !a.is_empty() {
                while b.first() == Some(&separator) {
                    b = &b[1..];
                }
                sep_a = usize::from(a[a.len() - 1] != separator);
            }
            if !a.is_empty() || !b.is_empty() {
                while c.first() == Some(&separator) {
                    c = &c[1..];
                }
            }
            if !b.is_empty() {
                sep_b = usize::from(b[b.len() - 1] != separator);
            }
        }
        let mut total = alloc_add(a.len(), sep_a)?;
        total = alloc_add(total, b.len())?;
        total = alloc_add(total, sep_b)?;
        total = alloc_add(total, c.len())?;
        total = alloc_add(total, 1)?;
        self.ensure(total)?;
        let mut t = 0;
        self.write_at(t, a);
        t += a.len();
        if sep_a == 1 {
            self.write_byte(t, separator);
            t += 1;
        }
        self.write_at(t, b);
        t += b.len();
        if sep_b == 1 {
            self.write_byte(t, separator);
            t += 1;
        }
        self.write_at(t, c);
        t += c.len();
        self.set_size_nul(t);
        Ok(())
    }

    // ----- comparisons and edits -----

    /// `git_str_cmp`: `memcmp` of the common length, then the shorter
    /// first.
    pub fn compare(&self, other: &GitStr) -> c_int {
        let (a, b) = (self.as_bytes(), other.as_bytes());
        let n = a.len().min(b.len());
        let r = if n == 0 {
            0
        } else {
            // SAFETY: both slices are readable for `n` bytes. (C's `memcmp`,
            // whose result's magnitude the C returns.)
            unsafe { libc::memcmp(a.as_ptr().cast(), b.as_ptr().cast(), n) }
        };
        if r != 0 {
            r
        } else {
            match a.len().cmp(&b.len()) {
                core::cmp::Ordering::Less => -1,
                core::cmp::Ordering::Greater => 1,
                core::cmp::Ordering::Equal => 0,
            }
        }
    }

    /// `git_str_splice`: replace `nb_to_remove` bytes at `where_` with
    /// `data`.
    pub fn splice(&mut self, where_: usize, nb_to_remove: usize, data: &[u8]) -> Result<()> {
        git_assert!(where_ <= self.size, "where <= buf->size");
        git_assert!(
            nb_to_remove <= self.size - where_,
            "nb_to_remove <= buf->size - where"
        );
        let new_size = alloc_add(self.size - nb_to_remove, data.len())?;
        let alloc_size = alloc_add(new_size, 1)?;
        self.ensure(alloc_size)?;
        let size = self.size;
        self.alloc_mut()
            .copy_within(where_ + nb_to_remove..size, where_ + data.len());
        self.write_at(where_, data);
        self.set_size_nul(new_size);
        Ok(())
    }

    /// `git_str_quote`: quote the contents as git does for paths, if they
    /// need it.
    pub fn quote(&mut self) -> Result<()> {
        const WHITESPACE: &[u8; 7] = b"abtnvfr";
        let b = self.as_bytes();
        let mut quote = b.first() == Some(&b'!');
        let mut i = 0;
        if !quote {
            while i < b.len() {
                let c = b[i] as i8;
                if c == b'"' as i8 || c == b'\\' as i8 || c < b' ' as i8 || c > b'~' as i8 {
                    quote = true;
                    break;
                }
                i += 1;
            }
        }
        if !quote {
            return Ok(());
        }
        let mut quoted = GitStr::new();
        let _ = quoted.putc(b'"');
        let _ = quoted.put(&b[..i]);
        for &byte in &b[i..] {
            let c = byte as i8;
            if (0x07..=0x0d).contains(&c) {
                let _ = quoted.putc(b'\\');
                let _ = quoted.putc(WHITESPACE[(c - 0x07) as usize]);
            } else if byte == b'"' || byte == b'\\' {
                let _ = quoted.putc(b'\\');
                let _ = quoted.putc(byte);
            } else if byte != b' ' && (c < b'!' as i8 || c > b'~' as i8) {
                // C: `git_str_printf(&quoted, "\\%03o", ...)`.
                let _ = quoted.printf(5, format_args!("\\{byte:03o}"));
            } else {
                let _ = quoted.putc(byte);
            }
        }
        let _ = quoted.putc(b'"');
        if quoted.is_oom() {
            return Err(ErrorCode::GENERIC);
        }
        self.swap(&mut quoted);
        Ok(())
    }

    /// Make a borrowed string's contents owned (so they can be edited in
    /// place), keeping them.
    fn make_owned(&mut self) -> Result<()> {
        if self.owned() || self.size == 0 || self.is_oom() {
            return Ok(());
        }
        let copy = GitStr::from_bytes(self.as_bytes())?;
        *self = copy;
        Ok(())
    }

    /// `git_str_unquote`: undo [`quote`](Self::quote).
    pub fn unquote(&mut self) -> Result<()> {
        self.rtrim();
        let invalid = || Err(git_error_set!(ErrorClass::INVALID, "invalid quoted line"));
        {
            let b = self.as_bytes();
            if b.len() < 2 || b[0] != b'"' || b[b.len() - 1] != b'"' {
                return invalid();
            }
        }
        // Borrowed contents are copied rather than written through
        // (rust/DIVERGENCES.md).
        self.make_owned()?;
        let size = self.size;
        let b = self.content_mut();
        let mut i = 0usize;
        let mut j = 1usize;
        while j < size - 1 {
            let mut ch = b[j];
            if ch == b'\\' {
                if j == size - 2 {
                    return invalid();
                }
                j += 1;
                ch = b[j];
                match ch {
                    b'"' | b'\\' => {}
                    b'a' => ch = 0x07,
                    b'b' => ch = 0x08,
                    b'f' => ch = 0x0c,
                    b'n' => ch = b'\n',
                    b'r' => ch = b'\r',
                    b't' => ch = b'\t',
                    b'v' => ch = 0x0b,
                    b'0'..=b'3' => {
                        if j == size - 3 {
                            return Err(git_error_set!(
                                ErrorClass::INVALID,
                                "truncated quoted character \\{}",
                                Bytes(&[ch])
                            ));
                        }
                        let is_oct = |c: u8| (b'0'..=b'7').contains(&c);
                        // `j + 2` may be `size`: C reads the NUL there.
                        let (c1, c2) = (b[j + 1], b.get(j + 2).copied().unwrap_or(0));
                        if !is_oct(c1) || !is_oct(c2) {
                            return Err(git_error_set!(
                                ErrorClass::INVALID,
                                "truncated quoted character \\{}",
                                Bytes(&[ch, c1, c2])
                            ));
                        }
                        ch = ((b[j] - b'0') << 6) | ((b[j + 1] - b'0') << 3) | (b[j + 2] - b'0');
                        j += 2;
                    }
                    _ => {
                        return Err(git_error_set!(
                            ErrorClass::INVALID,
                            "invalid quoted character \\{}",
                            Bytes(&[ch])
                        ));
                    }
                }
            }
            b[i] = ch;
            i += 1;
            j += 1;
        }
        self.set_size_nul(i);
        Ok(())
    }

    /// `git_str_puts_escaped`: append `s`, with each byte of `esc_chars`
    /// wrapped in `esc_prefix` and `esc_suffix`.
    pub fn puts_escaped(
        &mut self,
        s: &[u8],
        esc_chars: &[u8],
        esc_prefix: &[u8],
        esc_suffix: &[u8],
    ) -> Result<()> {
        let is_esc = |c: u8| esc_chars.contains(&c);
        let per = esc_prefix.len() + esc_suffix.len() + 1;
        let mut total = 0usize;
        for &c in s {
            total = alloc_add(total, if is_esc(c) { per } else { 1 })?;
        }
        let alloclen = alloc_add(total, 1)?;
        self.grow_by(alloclen).map_err(|_| ErrorCode::GENERIC)?;
        let mut w = self.size;
        for &c in s {
            if is_esc(c) {
                self.write_at(w, esc_prefix);
                w += esc_prefix.len();
                self.write_byte(w, c);
                w += 1;
                self.write_at(w, esc_suffix);
                w += esc_suffix.len();
            } else {
                self.write_byte(w, c);
                w += 1;
            }
            self.size = w;
        }
        self.set_size_nul(w);
        Ok(())
    }

    /// `git_str_puts_escape_regex`.
    pub fn puts_escape_regex(&mut self, s: &[u8]) -> Result<()> {
        self.puts_escaped(s, b"^.[]$()|*+?{}\\", b"\\", b"")
    }

    /// `git_str_unescape`: remove backslash escapes in place, up to the
    /// first NUL.
    pub fn unescape(&mut self) {
        if self.make_owned().is_err() {
            return;
        }
        let b = self.content_mut();
        let end = memchr::memchr(0, b).unwrap_or(b.len());
        let n = super::util::unescape(&mut b[..end]);
        if self.owned() {
            if n != end {
                self.write_byte(n, 0);
            }
            self.size = n;
        } else {
            self.size = 0;
        }
    }

    /// `git_str_crlf_to_lf`: set `self` to `src` with CRLF turned into LF.
    pub fn crlf_to_lf(&mut self, src: &[u8]) -> Result<()> {
        let Some(first) = memchr::memchr(b'\r', src) else {
            return self.set(src);
        };
        let new_size = alloc_add(src.len(), 1)?;
        self.grow(new_size).map_err(|_| ErrorCode::GENERIC)?;
        self.size = 0;
        let mut out = 0usize;
        let mut scan = 0usize;
        let mut next = Some(first);
        while let Some(nx) = next {
            if nx > scan {
                self.write_at(out, &src[scan..nx]);
                out += nx - scan;
            }
            if nx + 1 == src.len() || src[nx + 1] != b'\n' {
                self.write_byte(out, b'\r');
                out += 1;
            }
            scan = nx + 1;
            next = memchr::memchr(b'\r', &src[scan..]).map(|k| scan + k);
        }
        if scan < src.len() {
            self.write_at(out, &src[scan..]);
            out += src.len() - scan;
        }
        self.set_size_nul(out);
        Ok(())
    }

    /// `git_str_lf_to_crlf`: set `self` to `src` with LF turned into CRLF
    /// (existing CRLFs kept).
    pub fn lf_to_crlf(&mut self, src: &[u8]) -> Result<()> {
        let Some(first) = memchr::memchr(b'\n', src) else {
            return self.set(src);
        };
        let alloclen = alloc_add(alloc_add(src.len(), src.len() >> 4)?, 1)?;
        self.grow(alloclen).map_err(|_| ErrorCode::GENERIC)?;
        self.size = 0;
        let mut scan = 0usize;
        let mut next = Some(first);
        while let Some(nx) = next {
            let mut copylen = nx - scan;
            if copylen > 0 && src[nx - 1] == b'\r' {
                copylen -= 1;
            }
            let alloclen = alloc_add(copylen, 3)?;
            self.grow_by(alloclen).map_err(|_| ErrorCode::GENERIC)?;
            let n = self.size;
            self.write_at(n, &src[scan..scan + copylen]);
            self.write_byte(n + copylen, b'\r');
            self.write_byte(n + copylen + 1, b'\n');
            self.size = n + copylen + 2;
            scan = nx + 1;
            next = memchr::memchr(b'\n', &src[scan..]).map(|k| scan + k);
        }
        let n = self.size;
        self.set_size_nul(n);
        self.put(&src[scan..])
    }

    /// `git_str_common_prefix`: set the contents to the longest common
    /// prefix of `strings`.
    pub fn common_prefix(&mut self, strings: &[&[u8]]) -> Result<()> {
        self.clear();
        let Some(first) = strings.first() else {
            return Ok(());
        };
        self.sets(first)?;
        for s in &strings[1..] {
            self.common_prefix_step(s);
            if self.size == 0 {
                break;
            }
        }
        Ok(())
    }

    /// Truncate to the common prefix with `s`.
    fn common_prefix_step(&mut self, s: &[u8]) {
        let common = self
            .as_bytes()
            .iter()
            .zip(s.iter())
            .take_while(|(a, b)| a == b)
            .count();
        self.truncate(common);
    }

    // ----- text classification -----

    /// `git_str_detect_bom`: the BOM and its length.
    pub fn detect_bom(&self) -> (Bom, usize) {
        detect_bom(self.as_bytes())
    }

    /// `git_str_is_binary`.
    pub fn is_binary(&self) -> bool {
        is_binary(self.as_bytes())
    }

    /// `git_str_contains_nul`.
    pub fn contains_nul(&self) -> bool {
        memchr::memchr(0, self.as_bytes()).is_some()
    }

    /// `git_str_gather_text_stats`: the statistics, and whether the text
    /// looks binary.
    pub fn gather_text_stats(&self, skip_bom: bool) -> (TextStats, bool) {
        gather_text_stats(self.as_bytes(), skip_bom)
    }
}

/// Formats into the spare room of a buffer, counting what does not fit
/// (`vsnprintf`'s contract).
struct SpareWriter<'a> {
    buf: &'a mut [MaybeUninit<u8>],
    len: usize,
}

impl fmt::Write for SpareWriter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let b = s.as_bytes();
        if let Some(room) = self.buf.get_mut(self.len..) {
            let n = b.len().min(room.len());
            room[..n].write_copy_of_slice(&b[..n]);
        }
        self.len += b.len();
        Ok(())
    }
}

/// A borrowed `git_str` (`GIT_STR_INIT_CONST`) over bytes that outlive it;
/// it derefs to `&GitStr` for the functions that read one.
pub struct GitStrRef<'a> {
    s: GitStr,
    _data: core::marker::PhantomData<&'a [u8]>,
}

impl core::ops::Deref for GitStrRef<'_> {
    type Target = GitStr;
    fn deref(&self) -> &GitStr {
        &self.s
    }
}

/// `git_str_detect_bom` on bytes.
pub fn detect_bom(b: &[u8]) -> (Bom, usize) {
    if b.len() < 2 {
        return (Bom::None, 0);
    }
    let at = |i: usize| b.get(i).copied();
    match b[0] {
        0 if b.len() >= 4 && b[1] == 0 && b[2] == 0xfe && b[3] == 0xff => (Bom::Utf32Be, 4),
        0xef if b.len() >= 3 && b[1] == 0xbb && b[2] == 0xbf => (Bom::Utf8, 3),
        0xfe if b[1] == 0xff => (Bom::Utf16Be, 2),
        0xff if b[1] == 0xfe => {
            if b.len() >= 4 && at(2) == Some(0) && at(3) == Some(0) {
                (Bom::Utf32Le, 4)
            } else {
                (Bom::Utf16Le, 2)
            }
        }
        _ => (Bom::None, 0),
    }
}

/// `git_str_is_binary` on bytes.
pub fn is_binary(b: &[u8]) -> bool {
    let (bom, skip) = detect_bom(b);
    if bom as c_int > Bom::Utf8 as c_int {
        return true;
    }
    let mut printable: c_int = 0;
    let mut nonprintable: c_int = 0;
    for &c in &b[skip..] {
        if (c > 0x1f && c != 127) || c == 0x08 || c == 0x1b || c == 0x0c {
            printable += 1;
        } else if c == 0 {
            return true;
        } else if !isspace(c) {
            nonprintable += 1;
        }
    }
    (printable >> 7) < nonprintable
}

/// `git_str_gather_text_stats` on bytes.
pub fn gather_text_stats(b: &[u8], skip_bom: bool) -> (TextStats, bool) {
    let mut st = TextStats::default();
    let (bom, skip) = detect_bom(b);
    st.bom = bom;
    let mut scan = if skip_bom { skip } else { 0 };
    let mut end = b.len();
    if end > 0 && b[end - 1] == 0x1a {
        end -= 1;
    }
    while scan < end {
        let c = b[scan];
        scan += 1;
        if c > 0x1f && c != 0x7f {
            st.printable += 1;
        } else {
            match c {
                0 => {
                    st.nul += 1;
                    st.nonprintable += 1;
                }
                b'\n' => st.lf += 1,
                b'\r' => {
                    st.cr += 1;
                    if scan < end && b[scan] == b'\n' {
                        st.crlf += 1;
                    }
                }
                b'\t' | 0x0c | 0x0b | 0x08 | 0x1b => st.printable += 1,
                _ => st.nonprintable += 1,
            }
        }
    }
    let binary = st.cr != st.crlf || st.nul > 0 || (st.printable >> 7) < st.nonprintable;
    (st, binary)
}

const fn base64_decode_table() -> [i8; 256] {
    let mut t = [-1i8; 256];
    let enc = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut i = 0;
    while i < 64 {
        t[enc[i] as usize] = i as i8;
        i += 1;
    }
    t[b'=' as usize] = 0;
    t
}

const fn base85_decode_table() -> [i8; 256] {
    // C's table stores value + 1, so 0 means invalid there as -1 does here
    // after the `--de`.
    let mut t = [-1i8; 256];
    let enc =
        b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz!#$%&()*+-;<=>?@^_`{|}~";
    let mut i = 0;
    while i < 85 {
        t[enc[i] as usize] = (i + 1) as i8;
        i += 1;
    }
    t
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `str.c`. `git_str_printf`, `git_str_vprintf` and
/// `git_str_join_n` are variadic: they are C shims in
/// `src/util/rust_shims.c` over [`git_str__vprintf_begin`],
/// [`git_str__vprintf_written`] and [`git_str__join_n`].
///
/// Every export takes `git_str *` arguments that are valid `git_str`s (in
/// any of the states of the [module documentation](super)), that no other
/// thread accesses during the call; data arguments may point into the
/// string's own buffer where C allowed it (they are copied first, or
/// handled in place).
pub mod ffi {
    use super::*;
    use crate::util::export::{barrier, bytes, cstr_bytes, status};
    use std::borrow::Cow;

    /// The string at `buf`, for the call.
    ///
    /// # Safety
    ///
    /// `buf` is a valid `git_str` that nothing else accesses for `'a`.
    #[inline(always)]
    unsafe fn s<'a>(buf: *mut GitStr) -> &'a mut GitStr {
        // SAFETY: the caller's contract.
        unsafe { &mut *buf }
    }

    /// The `len` bytes at `data` as a segment of `buf`: its own contents
    /// if they lie within them, else a slice; copied if they overlap the
    /// allocation some other way.
    ///
    /// # Safety
    ///
    /// `data` is readable for `len` bytes (when `len > 0`), and nothing
    /// writes them during the call except through `buf`.
    #[inline(always)]
    unsafe fn seg<'a>(buf: &GitStr, data: *const c_char, len: usize) -> Cow<'a, [u8]> {
        if len == 0 || data.is_null() {
            return Cow::Borrowed(&[]);
        }
        // SAFETY: forwarded: `len` readable bytes at `data`.
        let ext = unsafe { bytes(data, len) };
        if buf.owned() {
            let start = buf.ptr as usize;
            let end = start + buf.asize;
            let (d0, d1) = (data as usize, data as usize + len);
            if d0 < end && d1 > start {
                // Within the buffer: copy before the buffer is touched.
                return Cow::Owned(ext.to_vec());
            }
        }
        Cow::Borrowed(ext)
    }

    /// The offset of `data` within `buf`'s contents, if `data..data+len`
    /// lies within them.
    fn own_offset(buf: &GitStr, data: *const c_char, len: usize) -> Option<usize> {
        let b = buf.as_bytes();
        let start = b.as_ptr() as usize;
        let d = data as usize;
        if b.is_empty() || d < start || d >= start + b.len() {
            return None;
        }
        let off = d - start;
        (off.checked_add(len)? <= b.len()).then_some(off)
    }

    /// `git_str_init`.
    ///
    /// # Safety
    ///
    /// `buf` is valid for writes of a `git_str` (its old contents are
    /// overwritten, not freed, as in C).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_init(buf: *mut GitStr, initial_size: usize) -> c_int {
        barrier(|| {
            // SAFETY: the caller passes a writable `git_str`; C does not free
            // what was there.
            unsafe { buf.write(GitStr::new()) };
            // SAFETY: just initialized.
            status(unsafe { s(buf) }.ensure(initial_size))
        })
    }

    /// `git_str_try_grow`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_try_grow(
        buf: *mut GitStr,
        target_size: usize,
        mark_oom: bool,
    ) -> c_int {
        // SAFETY: the module's contract.
        barrier(|| status(unsafe { s(buf) }.try_grow(target_size, mark_oom)))
    }

    /// `git_str_grow`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_grow(buf: *mut GitStr, target_size: usize) -> c_int {
        // SAFETY: the module's contract.
        barrier(|| status(unsafe { s(buf) }.grow(target_size)))
    }

    /// `git_str_grow_by`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_grow_by(buf: *mut GitStr, additional_size: usize) -> c_int {
        // SAFETY: the module's contract.
        barrier(|| status(unsafe { s(buf) }.grow_by(additional_size)))
    }

    /// `git_str_dispose` (NULL is allowed).
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_dispose(buf: *mut GitStr) {
        barrier(|| {
            if !buf.is_null() {
                // SAFETY: the module's contract.
                unsafe { s(buf) }.dispose()
            }
        })
    }

    /// `git_str_clear`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_clear(buf: *mut GitStr) {
        // SAFETY: the module's contract.
        barrier(|| unsafe { s(buf) }.clear())
    }

    /// `git_str_set`.
    ///
    /// # Safety
    ///
    /// See the [module](self); `data` is readable for `len` bytes (it may be
    /// the buffer's own `ptr`).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_set(
        buf: *mut GitStr,
        data: *const libc::c_void,
        len: usize,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the module's contract.
            let b = unsafe { s(buf) };
            if len == 0 || data.is_null() {
                b.clear();
                return 0;
            }
            let data = data.cast::<c_char>();
            if data == b.ptr.cast_const() {
                // C's `data == buf->ptr`: the first `len` bytes are in
                // place (the caller guarantees they are readable).
                b.size = len;
                if b.asize > len {
                    b.write_byte(len, 0);
                }
                return 0;
            }
            if let Some(off) = own_offset(b, data, len) {
                return status(b.set_seg(Seg::Own { off, len }));
            }
            // SAFETY: the caller passes `len` readable bytes.
            let d = unsafe { seg(b, data, len) };
            status(b.set(&d))
        })
    }

    /// `git_str_sets`.
    ///
    /// # Safety
    ///
    /// See the [module](self); `string` is NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_sets(buf: *mut GitStr, string: *const c_char) -> c_int {
        let len = if string.is_null() {
            0
        } else {
            // SAFETY: a non-null `string` is NUL-terminated.
            unsafe { libc::strlen(string) }
        };
        // SAFETY: forwarded.
        unsafe { git_str_set(buf, string.cast(), len) }
    }

    /// `git_str_putc`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_putc(buf: *mut GitStr, c: c_char) -> c_int {
        // SAFETY: the module's contract.
        barrier(|| status(unsafe { s(buf) }.putc(c as u8)))
    }

    /// `git_str_putcn`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_putcn(buf: *mut GitStr, c: c_char, len: usize) -> c_int {
        // SAFETY: the module's contract.
        barrier(|| status(unsafe { s(buf) }.putcn(c as u8, len)))
    }

    /// `git_str_put`.
    ///
    /// # Safety
    ///
    /// See the [module](self); `data` is readable for `len` bytes (it may
    /// lie within the buffer).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_put(
        buf: *mut GitStr,
        data: *const c_char,
        len: usize,
    ) -> c_int {
        barrier(|| {
            if len == 0 {
                return 0;
            }
            if data.is_null() {
                let _ = errors::assert_arg_failed("data");
                return -1;
            }
            // SAFETY: the module's contract.
            let b = unsafe { s(buf) };
            if let Some(off) = own_offset(b, data, len) {
                return status(b.put_seg(Seg::Own { off, len }));
            }
            // SAFETY: the caller passes `len` readable bytes.
            let d = unsafe { seg(b, data, len) };
            status(b.put(&d))
        })
    }

    /// `git_str_puts`.
    ///
    /// # Safety
    ///
    /// See the [module](self); `string` is NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_puts(buf: *mut GitStr, string: *const c_char) -> c_int {
        if string.is_null() {
            return barrier(|| errors::assert_arg_failed("string").raw());
        }
        // SAFETY: `string` is NUL-terminated.
        let len = unsafe { libc::strlen(string) };
        // SAFETY: forwarded.
        unsafe { git_str_put(buf, string, len) }
    }

    macro_rules! encode {
        ($(#[$m:meta])* $name:ident, $method:ident) => {
            $(#[$m])*
            ///
            /// # Safety
            ///
            /// See the [module](self); `data` is readable for `len` bytes.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name(buf: *mut GitStr, data: *const c_char, len: usize) -> c_int {
                barrier(|| {
                    // SAFETY: the module's contract: a valid, unaliased string.
                    let b = unsafe { s(buf) };
                    // SAFETY: the caller passes `len` readable bytes.
                    let d = unsafe { seg(b, data, len) };
                    status(b.$method(&d))
                })
            }
        };
    }

    encode!(
        /// `git_str_encode_hexstr`.
        git_str_encode_hexstr,
        encode_hexstr
    );
    encode!(
        /// `git_str_encode_base64`.
        git_str_encode_base64,
        encode_base64
    );
    encode!(
        /// `git_str_decode_base64`.
        git_str_decode_base64,
        decode_base64
    );
    encode!(
        /// `git_str_encode_base85`.
        git_str_encode_base85,
        encode_base85
    );
    encode!(
        /// `git_str_decode_percent`.
        git_str_decode_percent,
        decode_percent
    );

    /// `git_str_decode_base85`.
    ///
    /// # Safety
    ///
    /// See the [module](self); `base85` is readable for `base85_len` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_decode_base85(
        buf: *mut GitStr,
        base85: *const c_char,
        base85_len: usize,
        output_len: usize,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the module's contract: a valid, unaliased string.
            let b = unsafe { s(buf) };
            // SAFETY: the caller passes `base85_len` readable bytes.
            let d = unsafe { seg(b, base85, base85_len) };
            status(b.decode_base85(&d, output_len))
        })
    }

    /// The first half of the `git_str_vprintf` shim.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str__vprintf_begin(buf: *mut GitStr, format_len: usize) -> c_int {
        // SAFETY: the module's contract.
        barrier(|| status(unsafe { s(buf) }.vprintf_begin(format_len)))
    }

    /// The second half of the `git_str_vprintf` shim: 0 when done, 1 to
    /// format again, -1 on failure.
    ///
    /// # Safety
    ///
    /// See the [module](self); `vsnprintf` returned `len` for a write of at
    /// most `asize - size` bytes at `ptr + size`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str__vprintf_written(buf: *mut GitStr, len: c_int) -> c_int {
        barrier(|| {
            // SAFETY: the module's contract: a valid, unaliased string.
            let b = unsafe { s(buf) };
            // SAFETY: `len` is what `vsnprintf` returned for the room at
            // `ptr + size` (this function's contract).
            match unsafe { b.vprintf_written(len) } {
                Ok(true) => 0,
                Ok(false) => 1,
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_str_copy_cstr`.
    ///
    /// # Safety
    ///
    /// `data` is writable for `datasize` bytes; `buf` is NULL or a valid
    /// `git_str`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_copy_cstr(
        data: *mut c_char,
        datasize: usize,
        buf: *const GitStr,
    ) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!data.is_null(), "data", -1);
            crate::git_assert_arg!(datasize != 0, "datasize", -1);
            crate::git_assert_arg!(!buf.is_null(), "buf", -1);
            // SAFETY: `buf` is a valid `git_str`, only read here.
            let b = unsafe { &*buf };
            let src = b.as_bytes();
            // C uses `memmove`, so `data` may overlap the contents: then the
            // contents are copied out before `data` is borrowed mutably.
            let overlap = {
                let (s0, s1) = (src.as_ptr() as usize, src.as_ptr() as usize + src.len());
                let (d0, d1) = (data as usize, data as usize + datasize);
                !src.is_empty() && s0 < d1 && d0 < s1
            };
            if overlap {
                let tmp = match GitStr::from_bytes(src) {
                    Ok(t) => t,
                    Err(e) => return e.raw(),
                };
                // SAFETY: `data` is writable for `datasize` bytes, and the
                // string is not read again (its copy is).
                let out = unsafe { core::slice::from_raw_parts_mut(data.cast::<u8>(), datasize) };
                if b.asize == 0 {
                    out[0] = 0;
                    return 0;
                }
                return status(tmp.copy_cstr(out));
            }
            // SAFETY: `data` is writable for `datasize` bytes, disjoint from
            // the contents (checked above).
            let out = unsafe { core::slice::from_raw_parts_mut(data.cast::<u8>(), datasize) };
            status(b.copy_cstr(out))
        })
    }

    /// `git_str_consume_bytes`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_consume_bytes(buf: *mut GitStr, len: usize) {
        // SAFETY: the module's contract.
        barrier(|| unsafe { s(buf) }.consume_bytes(len))
    }

    /// `git_str_consume`: drop the bytes before `end`, a pointer into the
    /// contents (nothing otherwise).
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_consume(buf: *mut GitStr, end: *const c_char) {
        barrier(|| {
            // SAFETY: the module's contract.
            let b = unsafe { s(buf) };
            let start = b.ptr as usize;
            let e = end as usize;
            if e > start && e <= start + b.size {
                b.consume_bytes(e - start);
            }
        })
    }

    /// `git_str_truncate`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_truncate(buf: *mut GitStr, len: usize) {
        // SAFETY: the module's contract.
        barrier(|| unsafe { s(buf) }.truncate(len))
    }

    /// `git_str_shorten`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_shorten(buf: *mut GitStr, amount: usize) {
        // SAFETY: the module's contract.
        barrier(|| unsafe { s(buf) }.shorten(amount))
    }

    /// `git_str_truncate_at_char`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_truncate_at_char(buf: *mut GitStr, separator: c_char) {
        // SAFETY: the module's contract.
        barrier(|| unsafe { s(buf) }.truncate_at_char(separator as u8))
    }

    /// `git_str_rtruncate_at_char`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_rtruncate_at_char(buf: *mut GitStr, separator: c_char) {
        // SAFETY: the module's contract.
        barrier(|| unsafe { s(buf) }.rtruncate_at_char(separator as u8))
    }

    /// `git_str_swap`.
    ///
    /// # Safety
    ///
    /// Both are valid `git_str`s (they may be the same one).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_swap(str_a: *mut GitStr, str_b: *mut GitStr) {
        barrier(|| {
            // SAFETY: both are valid; `ptr::swap` allows them to be equal.
            unsafe { ptr::swap(str_a, str_b) }
        })
    }

    /// `git_str_detach`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_detach(buf: *mut GitStr) -> *mut c_char {
        // SAFETY: the module's contract.
        barrier(|| unsafe { s(buf) }.detach_raw())
    }

    /// `git_str_attach`.
    ///
    /// # Safety
    ///
    /// See the [module](self) and [`GitStr::attach`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_attach(
        buf: *mut GitStr,
        ptr: *mut c_char,
        asize: usize,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the module's contract: a valid, unaliased string.
            let b = unsafe { s(buf) };
            // SAFETY: `attach`'s contract, forwarded to the caller.
            status(unsafe { b.attach(ptr, asize) })
        })
    }

    /// `git_str_attach_notowned`.
    ///
    /// # Safety
    ///
    /// See the [module](self) and [`GitStr::attach_notowned`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_attach_notowned(
        buf: *mut GitStr,
        ptr: *const c_char,
        size: usize,
    ) {
        barrier(|| {
            // SAFETY: the module's contract: a valid, unaliased string.
            let b = unsafe { s(buf) };
            // SAFETY: `attach_notowned`'s contract, forwarded to the caller.
            unsafe { b.attach_notowned(ptr, size) }
        })
    }

    /// The Rust half of the `git_str_join_n` shim: `segments` holds `nbuf`
    /// pointers (NULL or NUL-terminated; they may point into the buffer).
    ///
    /// # Safety
    ///
    /// See the [module](self); `segments` holds `nbuf` such pointers.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str__join_n(
        buf: *mut GitStr,
        separator: c_char,
        nbuf: c_int,
        segments: *const *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the module's contract.
            let b = unsafe { s(buf) };
            let n = usize::try_from(nbuf).unwrap_or(0);
            let ptrs: &[*const c_char] = if n == 0 {
                &[]
            } else {
                // SAFETY: the caller passes `nbuf` pointers.
                unsafe { core::slice::from_raw_parts(segments, n) }
            };
            let mut copies: Vec<Option<Cow<'_, [u8]>>> = Vec::with_capacity(n);
            let mut owns: Vec<Option<usize>> = Vec::with_capacity(n);
            for &p in ptrs {
                if p.is_null() {
                    copies.push(None);
                    owns.push(None);
                } else if let Some(off) = own_offset(b, p, 1) {
                    // Within the contents: C uses the rest of the original
                    // contents from there.
                    copies.push(None);
                    owns.push(Some(off));
                } else {
                    // SAFETY: a non-null segment is NUL-terminated.
                    let bytes = unsafe { cstr_bytes(p) };
                    // SAFETY: readable, as just measured.
                    copies.push(Some(unsafe { seg(b, p, bytes.len()) }));
                    owns.push(None);
                }
            }
            let size = b.as_bytes().len();
            let segs: Vec<Option<Seg<'_>>> = copies
                .iter()
                .zip(&owns)
                .map(|(c, o)| match (c, o) {
                    (Some(c), _) => Some(Seg::Ext(c)),
                    (None, Some(off)) => Some(Seg::Own {
                        off: *off,
                        len: size - off,
                    }),
                    (None, None) => None,
                })
                .collect();
            status(b.join_segs(separator as u8, &segs))
        })
    }

    /// `git_str_join`.
    ///
    /// # Safety
    ///
    /// See the [module](self); `str_a` is NULL or NUL-terminated (it may
    /// lie within the buffer); `str_b` is NUL-terminated and must not lie
    /// within the buffer's contents.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_join(
        buf: *mut GitStr,
        separator: c_char,
        str_a: *const c_char,
        str_b: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the module's contract.
            let b = unsafe { s(buf) };
            if b.size > 0 {
                let start = b.ptr as usize;
                let p = str_b as usize;
                crate::git_assert_arg!(
                    p < start || p >= start + b.size,
                    "str_b < buf->ptr || str_b >= buf->ptr + buf->size",
                    -1
                );
            }
            // SAFETY: `str_a` is NULL or NUL-terminated.
            let a_bytes = unsafe { cstr_bytes(str_a) };
            // SAFETY: `str_b` is NUL-terminated.
            let b_bytes = unsafe { cstr_bytes(str_b) };
            let a_len = a_bytes.len();
            let a_copy;
            let a = if str_a.is_null() {
                None
            } else if let Some(off) = own_offset(b, str_a, a_len) {
                // Within the contents (C's `offset_a`).
                Some(Seg::Own { off, len: a_len })
            } else {
                // SAFETY: readable, as just measured.
                a_copy = unsafe { seg(b, str_a, a_len) };
                Some(Seg::Ext(&a_copy))
            };
            // SAFETY: readable, as just measured.
            let bb = unsafe { seg(b, str_b, b_bytes.len()) };
            status(b.join_seg(separator as u8, a, &bb))
        })
    }

    /// `git_str_join3`.
    ///
    /// # Safety
    ///
    /// See the [module](self); the three are NUL-terminated and must not
    /// lie within the buffer's contents.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_join3(
        buf: *mut GitStr,
        separator: c_char,
        str_a: *const c_char,
        str_b: *const c_char,
        str_c: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the module's contract.
            let b = unsafe { s(buf) };
            let start = b.ptr as usize;
            let outside = |p: *const c_char| (p as usize) < start || (p as usize) >= start + b.size;
            git_assert!(
                outside(str_a),
                "str_a < buf->ptr || str_a >= buf->ptr + buf->size",
                -1
            );
            git_assert!(
                outside(str_b),
                "str_b < buf->ptr || str_b >= buf->ptr + buf->size",
                -1
            );
            git_assert!(
                outside(str_c),
                "str_c < buf->ptr || str_c >= buf->ptr + buf->size",
                -1
            );
            // The three are NUL-terminated strings outside the contents
            // (copied if they overlap the rest of the allocation).
            // SAFETY: NUL-terminated strings.
            let (la, lb, lc) = unsafe { strlen3(str_a, str_b, str_c) };
            // SAFETY: `la` readable bytes, as just measured.
            let a = unsafe { seg(b, str_a, la) };
            // SAFETY: as for `a`.
            let bb = unsafe { seg(b, str_b, lb) };
            // SAFETY: as for `a`.
            let c = unsafe { seg(b, str_c, lc) };
            status(b.join3(separator as u8, &a, &bb, &c))
        })
    }

    /// `git_str_rtrim`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_rtrim(buf: *mut GitStr) {
        // SAFETY: the module's contract.
        barrier(|| unsafe { s(buf) }.rtrim())
    }

    /// `git_str_cmp`.
    ///
    /// # Safety
    ///
    /// Both are valid `git_str`s.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_cmp(a: *const GitStr, b: *const GitStr) -> c_int {
        barrier(|| {
            // SAFETY: a valid `git_str`, only read.
            let a = unsafe { &*a };
            // SAFETY: as `a`.
            let b = unsafe { &*b };
            a.compare(b)
        })
    }

    /// `git_str_splice`.
    ///
    /// # Safety
    ///
    /// See the [module](self); `data` is readable for `nb_to_insert` bytes
    /// (it may lie within the buffer: it is copied first).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_splice(
        buf: *mut GitStr,
        where_: usize,
        nb_to_remove: usize,
        data: *const c_char,
        nb_to_insert: usize,
    ) -> c_int {
        barrier(|| {
            git_assert!(!buf.is_null(), "buf", -1);
            // SAFETY: the module's contract: a valid, unaliased string.
            let b = unsafe { s(buf) };
            // SAFETY: the caller passes `nb_to_insert` readable bytes.
            let d = unsafe { seg(b, data, nb_to_insert) };
            status(b.splice(where_, nb_to_remove, &d))
        })
    }

    /// `git_str_quote`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_quote(buf: *mut GitStr) -> c_int {
        // SAFETY: the module's contract.
        barrier(|| status(unsafe { s(buf) }.quote()))
    }

    /// `git_str_unquote`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_unquote(buf: *mut GitStr) -> c_int {
        // SAFETY: the module's contract.
        barrier(|| status(unsafe { s(buf) }.unquote()))
    }

    /// `git_str_puts_escaped`.
    ///
    /// # Safety
    ///
    /// See the [module](self); `string`, `esc_prefix` and `esc_suffix` are
    /// NULL or NUL-terminated, `esc_chars` NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_puts_escaped(
        buf: *mut GitStr,
        string: *const c_char,
        esc_chars: *const c_char,
        esc_prefix: *const c_char,
        esc_suffix: *const c_char,
    ) -> c_int {
        barrier(|| {
            if string.is_null() {
                return 0;
            }
            // SAFETY: the module's contract: a valid, unaliased string.
            let b = unsafe { s(buf) };
            // SAFETY: `string` is NUL-terminated (and not NULL here).
            let l = unsafe { libc::strlen(string) };
            // SAFETY: `l` readable bytes, as just measured.
            let st = unsafe { seg(b, string, l) };
            // SAFETY: NUL-terminated.
            let ec = unsafe { cstr_bytes(esc_chars) };
            // SAFETY: NULL or NUL-terminated.
            let ep = unsafe { cstr_bytes(esc_prefix) };
            // SAFETY: NULL or NUL-terminated.
            let es = unsafe { cstr_bytes(esc_suffix) };
            status(b.puts_escaped(&st, ec, ep, es))
        })
    }

    /// `git_str_unescape`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_unescape(buf: *mut GitStr) {
        // SAFETY: the module's contract.
        barrier(|| unsafe { s(buf) }.unescape())
    }

    /// The lengths of three NUL-terminated strings.
    ///
    /// # Safety
    ///
    /// Each is NUL-terminated.
    unsafe fn strlen3(
        a: *const c_char,
        b: *const c_char,
        c: *const c_char,
    ) -> (usize, usize, usize) {
        // SAFETY: the caller passes NUL-terminated strings.
        let la = unsafe { libc::strlen(a) };
        // SAFETY: as above.
        let lb = unsafe { libc::strlen(b) };
        // SAFETY: as above.
        let lc = unsafe { libc::strlen(c) };
        (la, lb, lc)
    }

    /// The contents of `src` for an operation on another string `tgt`.
    ///
    /// # Safety
    ///
    /// `src` is a valid `git_str`.
    unsafe fn src_bytes<'a>(tgt: &GitStr, src: *const GitStr) -> Cow<'a, [u8]> {
        // SAFETY: `src` is valid; its contents are readable for the call.
        let sb = unsafe { (*src).as_bytes() };
        // SAFETY: as above.
        unsafe { seg(tgt, sb.as_ptr().cast(), sb.len()) }
    }

    /// `git_str_crlf_to_lf`.
    ///
    /// # Safety
    ///
    /// Both are valid `git_str`s.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_crlf_to_lf(tgt: *mut GitStr, src: *const GitStr) -> c_int {
        barrier(|| {
            git_assert!(!core::ptr::eq(tgt, src), "tgt != src", -1);
            // SAFETY: a valid string, distinct from `src` (checked).
            let t = unsafe { s(tgt) };
            // SAFETY: `src` is a valid string.
            let sb = unsafe { src_bytes(t, src) };
            status(t.crlf_to_lf(&sb))
        })
    }

    /// `git_str_lf_to_crlf`.
    ///
    /// # Safety
    ///
    /// Both are valid `git_str`s.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_lf_to_crlf(tgt: *mut GitStr, src: *const GitStr) -> c_int {
        barrier(|| {
            git_assert!(!core::ptr::eq(tgt, src), "tgt != src", -1);
            // SAFETY: a valid string, distinct from `src` (checked).
            let t = unsafe { s(tgt) };
            // SAFETY: `src` is a valid string.
            let sb = unsafe { src_bytes(t, src) };
            status(t.lf_to_crlf(&sb))
        })
    }

    /// `git_str_common_prefix`.
    ///
    /// # Safety
    ///
    /// See the [module](self); `strings` is NULL or holds `count` pointers
    /// to NUL-terminated strings, of which only those up to the point where
    /// the prefix becomes empty are read (as C reads them).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_common_prefix(
        buf: *mut GitStr,
        strings: *const *const c_char,
        count: usize,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the module's contract.
            let b = unsafe { s(buf) };
            b.clear();
            if strings.is_null() || count == 0 {
                return 0;
            }
            for i in 0..count {
                if i > 0 && b.size == 0 {
                    break;
                }
                // SAFETY: entry `i < count` is readable (and, as C reads
                // them one by one, only those reached are).
                let p = unsafe { strings.wrapping_add(i).read() };
                // SAFETY: NULL (read as empty) or NUL-terminated.
                let l = unsafe { cstr_bytes(p) }.len();
                // SAFETY: `l` readable bytes, as just measured; copied if
                // they overlap the buffer.
                let sb = unsafe { seg(b, p, l) };
                if i == 0 {
                    if let Err(e) = b.sets(&sb) {
                        return e.raw();
                    }
                } else {
                    b.common_prefix_step(&sb);
                }
            }
            0
        })
    }

    /// `git_str_is_binary`.
    ///
    /// # Safety
    ///
    /// `buf` is a valid `git_str`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_is_binary(buf: *const GitStr) -> c_int {
        // SAFETY: a valid `git_str`, only read.
        barrier(|| c_int::from(unsafe { &*buf }.is_binary()))
    }

    /// `git_str_contains_nul`.
    ///
    /// # Safety
    ///
    /// `buf` is a valid `git_str`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_contains_nul(buf: *const GitStr) -> c_int {
        // SAFETY: a valid `git_str`, only read.
        barrier(|| c_int::from(unsafe { &*buf }.contains_nul()))
    }

    /// `git_str_detect_bom`.
    ///
    /// # Safety
    ///
    /// `bom` is writable; `buf` is a valid `git_str`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_detect_bom(bom: *mut Bom, buf: *const GitStr) -> c_int {
        barrier(|| {
            // SAFETY: a valid `git_str`, only read.
            let (b, n) = unsafe { &*buf }.detect_bom();
            // SAFETY: `bom` is writable.
            unsafe { bom.write(b) };
            n as c_int
        })
    }

    /// `git_str_gather_text_stats`.
    ///
    /// # Safety
    ///
    /// `stats` is writable; `buf` is a valid `git_str`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_gather_text_stats(
        stats: *mut TextStats,
        buf: *const GitStr,
        skip_bom: bool,
    ) -> bool {
        barrier(|| {
            // SAFETY: a valid `git_str`, only read.
            let (st, bin) = unsafe { &*buf }.gather_text_stats(skip_bom);
            // SAFETY: `stats` is writable.
            unsafe { stats.write(st) };
            bin
        })
    }
}

crate::c_layout_test! {
    headers: ["str.h"],
    types: [
        GitStr => "git_str" { ptr, asize, size },
        TextStats => "git_str_text_stats" { bom, nul, cr, lf, crlf, printable, nonprintable },
        Bom => "git_str_bom_t" {},
    ],
    asserts: ["GIT_STR_BOM_UTF32_BE == 5"],
}
