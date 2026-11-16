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
        self.asize > 0 && !self.ptr.is_null() && !self.is_oom() && self.ptr != initstr()
    }

    /// Whether the buffer is an allocation this string owns (and frees).
    #[inline]
    pub fn owns_buffer(&self) -> bool {
        self.owned()
    }

    /// The contents; empty for an out-of-memory string.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        if self.size == 0 || self.ptr.is_null() || self.is_oom() || self.ptr == initstr() {
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

    /// Check that a `Seg::Own` lies within the contents: the consumers of
    /// segments read it with raw copies, so they must not trust it.
    #[inline(always)]
    fn check_seg(&self, seg: Option<&Seg<'_>>) -> Result<()> {
        if let Some(&Seg::Own { off, len }) = seg
            && off
                .checked_add(len)
                .is_none_or(|end| end > self.as_bytes().len())
        {
            return Err(errors::assert_failed("segment within the contents"));
        }
        Ok(())
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
        // (Evaluated once: nothing below changes it before the realloc.)
        let owned = self.owned();
        let capacity = if owned { self.asize } else { 0 };
        if capacity == 0 && self.size != 0 {
            let _ = errors::set_str(ErrorClass::INVALID, "cannot grow a borrowed buffer");
            return Err(ErrorCode::INVALID);
        }
        let target = if target_size == 0 {
            self.size
        } else {
            target_size
        };
        if target <= capacity {
            return Ok(());
        }
        let (mut new_size, old) = if !owned {
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

    /// The allocated bytes after the contents and their NUL
    /// (`asize - size - 1` of them); empty unless the string owns its
    /// buffer. The string reads none of them, so safe code may write
    /// anything there; [`set_len`](Self::set_len) commits written bytes.
    pub fn spare_capacity_mut(&mut self) -> &mut [MaybeUninit<u8>] {
        if !self.owned() || self.size.saturating_add(1) >= self.asize {
            return &mut [];
        }
        let spare = self
            .ptr
            .cast::<MaybeUninit<u8>>()
            .wrapping_add(self.size + 1);
        // SAFETY: an owned buffer has `asize` bytes, of which the string
        // uses `size` and the NUL after them; the rest (at `spare`) is
        // ours to write through `&mut self`, and nothing reads it.
        unsafe { core::slice::from_raw_parts_mut(spare, self.asize - self.size - 1) }
    }

    /// The room after the contents, the NUL's byte included (C's
    /// `ptr + size`, `asize - size` bytes); empty unless the string owns
    /// its buffer.
    ///
    /// # Safety
    ///
    /// Byte 0 is the string's NUL: until it is written back (or
    /// [`set_len`](Self::set_len) moves the end and a NUL is written there,
    /// or [`terminate`](Self::terminate) runs), the string must not be read
    /// or handed on.
    pub(crate) unsafe fn spare_with_nul_mut(&mut self) -> &mut [MaybeUninit<u8>] {
        let (p, n) = self.spare_ptr();
        if n == 0 {
            return &mut [];
        }
        // SAFETY: `p` and `n` are the owned allocation's bytes from `size`
        // on, ours through `&mut self`; the caller restores the NUL.
        unsafe { core::slice::from_raw_parts_mut(p.cast::<MaybeUninit<u8>>(), n) }
    }

    /// C's view of the room after the contents: `ptr + size` and
    /// `asize - size` bytes, the NUL's slot included (NULL and 0 unless
    /// the string owns its buffer). Writing through the pointer is the
    /// caller's `unsafe` step; overwriting the NUL leaves the string
    /// unterminated until [`set_len`](Self::set_len) and
    /// [`terminate`](Self::terminate) (or a growth) restore it.
    pub(crate) fn spare_ptr(&mut self) -> (*mut u8, usize) {
        if !self.owned() || self.size >= self.asize {
            return (ptr::null_mut(), 0);
        }
        (
            self.ptr.cast::<u8>().wrapping_add(self.size),
            self.asize - self.size,
        )
    }

    /// Set the length to `len` (`buf->size = len`) after writing the bytes
    /// up to it through [`spare_ptr`](Self::spare_ptr) or
    /// [`spare_capacity_mut`](Self::spare_capacity_mut). No NUL is written.
    ///
    /// # Safety
    ///
    /// The string owns its buffer, `len <= asize`, and the first `len`
    /// bytes are initialized. Unless the NUL at `len` is known to be there,
    /// the string is outside its invariant (C's zstream does this, with
    /// `len == asize` even): call [`terminate`](Self::terminate) or grow it
    /// before it is read.
    pub unsafe fn set_len(&mut self, len: usize) {
        debug_assert!(self.owned() && len <= self.asize);
        self.size = len;
    }

    /// Write the NUL after the contents, growing by one byte if the
    /// contents fill the buffer (the state [`set_len`](Self::set_len) may
    /// leave). Nothing to do for a string that does not own its buffer.
    pub fn terminate(&mut self) -> Result<()> {
        if !self.owned() {
            return Ok(());
        }
        if self.size >= self.asize {
            // `try_grow` writes the NUL at `size` after growing.
            return self.grow(self.size.saturating_add(1));
        }
        let n = self.size;
        self.write_byte(n, 0);
        Ok(())
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
        if self.is_oom() || (d > self.capacity_owned() && self.grow(d).is_err()) {
            return Err(ErrorCode::GENERIC);
        }
        Ok(())
    }

    /// `asize` if the buffer is owned, else 0 (what can be written).
    #[inline(always)]
    fn capacity_owned(&self) -> usize {
        if self.owned() { self.asize } else { 0 }
    }

    // ----- lifetime -----

    /// `git_str_dispose`: free an owned buffer and become empty.
    pub fn dispose(&mut self) {
        if self.owned() {
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
        if !self.owned() {
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
        self.check_seg(Some(&data))?;
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
        let buf = self.alloc_mut();
        buf[n].write(c);
        buf[n + 1].write(0);
        self.size = n + 1;
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
        let len = data.len();
        let n = self.size;
        // Fast path: an owned buffer with room for `data` and the NUL (what
        // `ENSURE_SIZE` lets through without growing). Empty data is a
        // no-op, as in C.
        if len != 0 && self.owned() && len < self.asize && n < self.asize - len {
            let buf = self.alloc_mut();
            buf[n..n + len].write_copy_of_slice(data);
            buf[n + len].write(0);
            self.size = n + len;
            return Ok(());
        }
        if len != 0 && n == 0 && !self.owned() && !self.is_oom() {
            return self.put_first(data);
        }
        self.put_seg(Seg::Ext(data))
    }

    /// `put` of non-empty `data` into an empty string that owns no buffer
    /// (the first `put` into `GIT_STR_INIT`, as when a `git_buf` is
    /// filled): the steps of `ENSURE_SIZE` and `git_str_try_grow` for that
    /// state alone, with their results and errors.
    #[inline(never)]
    fn put_first(&mut self, data: &[u8]) -> Result<()> {
        let len = data.len();
        // `ENSURE_SIZE(size + len + 1)`, with `size == 0`.
        let target = alloc_add(len, 1)?;
        // `try_grow` of a buffer it does not own: the target, rounded up
        // to 8; one that wraps is out of memory.
        let new_size = target.wrapping_add(7) & !7;
        if new_size == 0 {
            self.mark_oom();
            errors::set_oom();
            return Err(ErrorCode::GENERIC);
        }
        // SAFETY: NULL asks for a new allocation.
        let p = unsafe { alloc::realloc(ptr::null_mut(), new_size) }.cast::<c_char>();
        if p.is_null() {
            self.mark_oom();
            return Err(ErrorCode::GENERIC);
        }
        self.asize = new_size;
        self.ptr = p;
        let buf = self.alloc_mut();
        buf[..len].write_copy_of_slice(data);
        buf[len].write(0);
        self.size = len;
        Ok(())
    }

    #[inline]
    pub(crate) fn put_seg(&mut self, data: Seg<'_>) -> Result<()> {
        self.check_seg(Some(&data))?;
        let len = data.len();
        if len != 0 {
            let new_size = alloc_add(alloc_add(self.size, len)?, 1)?;
            self.ensure(new_size)?;
            let n = self.size;
            let buf = self.alloc_mut();
            match data {
                Seg::Ext(b) => {
                    buf[n..n + len].write_copy_of_slice(b);
                }
                Seg::Own { off, len } => buf.copy_within(off..off + len, n),
            }
            buf[n + len].write(0);
            self.size = n + len;
        }
        Ok(())
    }

    /// Append `len` bytes from `data` when they fit the allocation (no
    /// growth); `false` (nothing done) otherwise. The hot path of
    /// `git_str_put`, without a panic or an error path. As no reallocation
    /// happens, `data` may lie in the buffer itself: the copy is a
    /// `memmove`, as C's.
    ///
    /// # Safety
    ///
    /// `data` is readable and initialized for `len` bytes, and no reference
    /// covers them.
    #[inline(always)]
    pub(crate) unsafe fn try_put_fast(&mut self, data: *const u8, len: usize) -> bool {
        if !self.owned() || self.size >= self.asize || len >= self.asize - self.size {
            return false;
        }
        let at = self.ptr.cast::<u8>().wrapping_add(self.size);
        // SAFETY: the string owns `asize` bytes at `ptr` (owned), and
        // `size + len < asize`, so `at..at + len + 1` is within them;
        // `data` is readable for `len` bytes; `copy` allows them to
        // overlap.
        unsafe { ptr::copy(data, at, len) };
        // SAFETY: the byte after them, within the allocation (as above).
        unsafe { at.wrapping_add(len).write(0) };
        self.size += len;
        true
    }

    /// Append `c` when it fits the allocation; `false` (nothing done)
    /// otherwise. The hot path of `git_str_putc`.
    #[inline(always)]
    pub(crate) fn try_putc_fast(&mut self, c: u8) -> bool {
        if !self.owned() || self.size >= self.asize || self.asize - self.size < 2 {
            return false;
        }
        let at = self.ptr.cast::<u8>().wrapping_add(self.size);
        // SAFETY: the string owns `asize` bytes at `ptr`, and `size + 2 <=
        // asize`: both bytes are within them.
        unsafe { at.write(c) };
        // SAFETY: as above.
        unsafe { at.wrapping_add(1).write(0) };
        self.size += 1;
        true
    }

    /// Replace the contents with `len` bytes from `data` when they fit the
    /// allocation (no growth); `false` (nothing done) otherwise. The hot
    /// path of `git_str_set`. `data` may lie in the buffer (C's `data ==
    /// buf->ptr` included: the bytes stay, the size becomes `len`).
    ///
    /// # Safety
    ///
    /// As [`try_put_fast`](Self::try_put_fast).
    #[inline(always)]
    pub(crate) unsafe fn try_set_fast(&mut self, data: *const u8, len: usize) -> bool {
        if !self.owned() || len == 0 || len >= self.asize {
            return false;
        }
        let p = self.ptr.cast::<u8>();
        // SAFETY: the string owns `asize > len` bytes at `ptr`; `data` is
        // readable for `len` bytes; `copy` allows them to overlap.
        unsafe { ptr::copy(data, p, len) };
        // SAFETY: byte `len < asize`, within the allocation.
        unsafe { p.wrapping_add(len).write(0) };
        self.size = len;
        true
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
        with_segs(
            segments.len(),
            |i| segments[i].map(Seg::Ext),
            |segs| self.join_segs(separator, segs),
        )
    }

    /// `git_str_join_n` over the segments `segs`.
    pub(crate) fn join_segs(&mut self, separator: u8, segs: &[Option<Seg<'_>>]) -> Result<()> {
        for seg in segs {
            self.check_seg(seg.as_ref())?;
        }
        if self.is_oom() {
            // C reads a stale byte here, then fails in `git_str_grow_by`.
            return Err(ErrorCode::GENERIC);
        }
        let size = self.size;
        let total_size = {
            let contents = self.as_bytes();
            let mut total: Option<usize> =
                Some(usize::from(size > 0 && contents.last() != Some(&separator)));
            for seg in segs.iter().flatten() {
                let (len, last) = match *seg {
                    Seg::Ext(b) => (b.len(), b.last()),
                    Seg::Own { off, len } => (len, contents.get((off + len).wrapping_sub(1))),
                };
                let extra = usize::from(len == 0 || last != Some(&separator));
                total = total
                    .and_then(|t| t.checked_add(len))
                    .and_then(|t| t.checked_add(extra));
            }
            match total {
                Some(t) => t,
                None => return Err(errors::oom()),
            }
        };
        if total_size == 0 {
            return Ok(());
        }
        let total_size = alloc_add(total_size, 1)?;
        self.grow_by(total_size).map_err(|_| ErrorCode::GENERIC)?;

        // C's second pass, over raw pointers (a measured hot path). After
        // `grow_by`, the buffer is owned with room for `size + total_size`
        // bytes, which pass 1 counted; each write is also checked against
        // `asize` below, so soundness does not rest on that count.
        let asize = self.asize;
        let base = self.ptr.cast::<u8>();
        let mut out = size;
        let mut last = if size > 0 {
            // SAFETY: `size - 1 < size < asize`: a byte of the contents.
            unsafe { base.wrapping_add(size - 1).read() }
        } else {
            0
        };
        let room = |out: usize, len: usize| {
            assert!(
                out.checked_add(len).is_some_and(|e| e < asize),
                "join_n accounting"
            );
        };
        if size > 0 && last != separator {
            room(out, 1);
            // SAFETY: `out < asize` (checked): within the allocation.
            unsafe { base.wrapping_add(out).write(separator) };
            out += 1;
            last = separator;
        }
        let n = segs.len();
        for (i, seg) in segs.iter().enumerate() {
            let Some(seg) = *seg else { continue };
            let (mut from, mut len) = match seg {
                Seg::Ext(b) => (b.as_ptr(), b.len()),
                // The original contents, below `size <= out`: never written
                // by this loop.
                Seg::Own { off, len } => (base.wrapping_add(off).cast_const(), len),
            };
            if out > 0 && last == separator {
                // SAFETY: `from` is readable for `len` bytes (an external
                // slice, or contents within the allocation).
                while len > 0 && unsafe { from.read() } == separator {
                    from = from.wrapping_add(1);
                    len -= 1;
                }
            }
            if len > 0 {
                room(out, len);
                // SAFETY: `from` is readable for `len` bytes; the target
                // `out..out + len` is within the allocation (checked); they
                // may overlap only as `memmove` allows.
                unsafe { ptr::copy(from, base.wrapping_add(out), len) };
                out += len;
                // SAFETY: the byte just written.
                last = unsafe { base.wrapping_add(out - 1).read() };
            }
            if i + 1 < n && out > 0 && last != separator {
                room(out, 1);
                // SAFETY: `out < asize` (checked).
                unsafe { base.wrapping_add(out).write(separator) };
                out += 1;
                last = separator;
            }
        }
        room(out, 0);
        // SAFETY: `out < asize` (checked).
        unsafe { base.wrapping_add(out).write(0) };
        self.size = out;
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
        self.check_seg(a.as_ref())?;
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
        let total = strlen_a + need_sep + b.len();
        // A measured hot path: raw copies into the allocation, which
        // `ensure` made at least `total + 1 <= asize` bytes (checked again).
        assert!(self.owned() && total < self.asize, "join accounting");
        let base = self.ptr.cast::<u8>();
        match a {
            Some(Seg::Ext(a)) => {
                // SAFETY: `a` (outside the allocation) is readable for
                // `strlen_a <= total` bytes, written at the start.
                unsafe { ptr::copy_nonoverlapping(a.as_ptr(), base, strlen_a) };
            }
            Some(Seg::Own { off, len }) if off != 0 => {
                // SAFETY: `off..off + len` lies within the contents, `0..len`
                // within the allocation; `copy` allows them to overlap.
                unsafe { ptr::copy(base.wrapping_add(off), base, len) };
            }
            _ => {}
        }
        if need_sep == 1 {
            // SAFETY: `strlen_a < total < asize`.
            unsafe { base.wrapping_add(strlen_a).write(separator) };
        }
        // SAFETY: `b` (outside the allocation) is readable for its length,
        // written at `strlen_a + need_sep`, ending at `total < asize`.
        unsafe {
            ptr::copy_nonoverlapping(b.as_ptr(), base.wrapping_add(strlen_a + need_sep), b.len())
        };
        // SAFETY: `total < asize`.
        unsafe { base.wrapping_add(total).write(0) };
        self.size = total;
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

/// Collect `n` segments from `seg_at` (on the stack for up to 8), and
/// run `f` on them.
pub(crate) fn with_segs<'s, R>(
    n: usize,
    seg_at: impl Fn(usize) -> Option<Seg<'s>>,
    f: impl FnOnce(&[Option<Seg<'s>>]) -> Result<R>,
) -> Result<R> {
    const STACK: usize = 8;
    if n <= STACK {
        let mut arr: [Option<Seg<'s>>; STACK] = [None; STACK];
        for (i, slot) in arr[..n].iter_mut().enumerate() {
            *slot = seg_at(i);
        }
        f(&arr[..n])
    } else {
        let mut v: Vec<Option<Seg<'s>>> = Vec::new();
        try_reserve(&mut v, n)?;
        v.extend((0..n).map(seg_at));
        f(&v)
    }
}

/// `Vec::try_reserve_exact`, failing with the out-of-memory error.
pub(crate) fn try_reserve<T>(v: &mut Vec<T>, n: usize) -> Result<()> {
    v.try_reserve_exact(n).map_err(|_| errors::oom())
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
///
/// A data argument "readable for `n` bytes" must also be *initialized* for
/// those bytes. In particular, `git_str_set(buf, buf->ptr, len)` with `len >
/// size` (C's "in place" case) is allowed only when the bytes up to `len`
/// were written: Rust later reads the contents as bytes, where C would read
/// indeterminate `char`s.
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

    /// Where the `len` bytes at `data` lie, for an operation that appends
    /// or sets them: outside the buffer, within its contents, or overlapping
    /// its allocation some other way (then copied).
    enum Src<'a> {
        Seg(Seg<'a>),
        Copy(Vec<u8>),
    }

    impl Src<'_> {
        #[inline(always)]
        fn seg(&self) -> Seg<'_> {
            match self {
                Src::Seg(s) => *s,
                Src::Copy(v) => Seg::Ext(v),
            }
        }
    }

    /// Classify the `len` bytes at `data` against `buf` (see [`Src`]).
    ///
    /// # Safety
    ///
    /// As [`seg`].
    #[inline(always)]
    unsafe fn src<'a>(buf: &GitStr, data: *const c_char, len: usize) -> Src<'a> {
        if len == 0 || data.is_null() {
            return Src::Seg(Seg::Ext(&[]));
        }
        if buf.owned() {
            let start = buf.ptr as usize;
            let d = data as usize;
            if d < start + buf.asize && d.wrapping_add(len) > start {
                let size = buf.size.min(buf.asize);
                if d >= start && d - start + len <= size {
                    return Src::Seg(Seg::Own {
                        off: d - start,
                        len,
                    });
                }
                // SAFETY: the caller passes `len` readable bytes.
                return Src::Copy(unsafe { bytes(data, len) }.to_vec());
            }
        }
        // SAFETY: the caller passes `len` readable bytes, which lie outside
        // the buffer's allocation.
        Src::Seg(Seg::Ext(unsafe { bytes(data, len) }))
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
        // SAFETY: the module's contract.
        let b = unsafe { s(buf) };
        // Hot path (no panic is possible): room enough, no aliasing.
        // SAFETY: a non-null `data` is readable for `len` bytes.
        if !data.is_null() && unsafe { b.try_set_fast(data.cast(), len) } {
            return 0;
        }
        // SAFETY: forwarded.
        unsafe { set_slow(b, data, len) }
    }

    /// `git_str_set` when the fast path does not apply (out of line).
    ///
    /// # Safety
    ///
    /// As `git_str_set`.
    #[cold]
    #[inline(never)]
    unsafe fn set_slow(b: &mut GitStr, data: *const libc::c_void, len: usize) -> c_int {
        barrier(|| {
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
            // SAFETY: the caller passes `len` readable bytes.
            let d = unsafe { src(b, data, len) };
            status(b.set_seg(d.seg()))
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
        // SAFETY: the module's contract.
        let b = unsafe { s(buf) };
        // SAFETY: a non-null `string` is readable for `len` bytes.
        if !string.is_null() && unsafe { b.try_set_fast(string.cast(), len) } {
            return 0;
        }
        // SAFETY: forwarded.
        unsafe { set_slow(b, string.cast(), len) }
    }

    /// `git_str_putc`.
    ///
    /// # Safety
    ///
    /// See the [module](self).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_str_putc(buf: *mut GitStr, c: c_char) -> c_int {
        let ch = c as u8;
        // SAFETY: the module's contract.
        let b = unsafe { s(buf) };
        // Hot path (no panic is possible).
        if b.try_putc_fast(ch) {
            return 0;
        }
        putc_slow(b, ch)
    }

    #[cold]
    #[inline(never)]
    fn putc_slow(b: &mut GitStr, ch: u8) -> c_int {
        barrier(|| status(b.putc(ch)))
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
        if len == 0 {
            return 0;
        }
        // SAFETY: the module's contract.
        let b = unsafe { s(buf) };
        // Hot path (no panic is possible): room enough, no aliasing.
        // SAFETY: a non-null `data` is readable for `len` bytes.
        if !data.is_null() && unsafe { b.try_put_fast(data.cast(), len) } {
            return 0;
        }
        // SAFETY: forwarded.
        unsafe { put_slow(b, data, len) }
    }

    /// `git_str_put` when the fast path does not apply: out of line, so
    /// that the hot path needs no stack frame for the barrier.
    ///
    /// # Safety
    ///
    /// As `git_str_put`.
    #[cold]
    #[inline(never)]
    unsafe fn put_slow(b: &mut GitStr, data: *const c_char, len: usize) -> c_int {
        barrier(|| {
            if data.is_null() {
                let _ = errors::assert_arg_failed("data");
                return -1;
            }
            // SAFETY: the caller passes `len` readable bytes.
            let d = unsafe { src(b, data, len) };
            status(b.put_seg(d.seg()))
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
        if len == 0 {
            return 0;
        }
        // SAFETY: the module's contract.
        let b = unsafe { s(buf) };
        // SAFETY: `string` is readable for `len` bytes.
        if unsafe { b.try_put_fast(string.cast(), len) } {
            return 0;
        }
        // SAFETY: forwarded.
        unsafe { put_slow(b, string, len) }
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
            // C: `data[0] = 0`, then (unless the string is empty or not
            // allocated) `memmove` of `min(size, datasize - 1)` bytes and a
            // NUL. `data` may overlap the contents, so this is done with
            // raw copies: no reference to either, no allocation.
            // SAFETY: `buf` is a valid `git_str`; the reference covers the
            // struct only, which `data` does not overlap.
            let g = unsafe { &*buf };
            let (ptr, asize, size) = (g.ptr, g.asize, g.size);
            let d = data.cast::<u8>();
            // First, as C (which matters when `data` is the contents).
            // SAFETY: `data` is writable for `datasize >= 1` bytes.
            unsafe { d.write(0) };
            if size == 0 || asize == 0 || ptr == oomstr() || ptr.is_null() {
                return 0;
            }
            let n = size.min(asize).min(datasize - 1);
            // SAFETY: the contents are readable for `size` bytes and `data`
            // writable for `datasize`; `copy` allows them to overlap.
            unsafe { ptr::copy(ptr.cast::<u8>(), d, n) };
            // SAFETY: `n < datasize`.
            unsafe { d.wrapping_add(n).write(0) };
            0
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
        // SAFETY: the module's contract.
        if unsafe { join_n_fast(buf, separator as u8, nbuf, segments) } {
            return 0;
        }
        // SAFETY: forwarded.
        unsafe { join_n_slow(buf, separator, nbuf, segments) }
    }

    /// `git_str_join_n`'s hot path, C's two passes over raw pointers: up to
    /// 8 segments, each outside the allocation or starting within the
    /// contents, and a result that fits the allocation. `false` (nothing
    /// done) when it does not apply. No panic; every write is clamped to
    /// the allocation, so soundness does not rest on pass 1's count.
    ///
    /// # Safety
    ///
    /// As `git_str__join_n`.
    #[inline(always)]
    unsafe fn join_n_fast(
        buf: *mut GitStr,
        sep: u8,
        nbuf: c_int,
        segments: *const *const c_char,
    ) -> bool {
        const MAX: usize = 8;
        let Ok(n) = usize::try_from(nbuf) else {
            return false;
        };
        if n == 0 || n > MAX {
            return false;
        }
        // SAFETY: the module's contract.
        let b = unsafe { s(buf) };
        if !b.owned() || b.size >= b.asize {
            return false;
        }
        let (base, asize, size) = (b.ptr.cast::<u8>(), b.asize, b.size);
        let (lo, hi) = (base as usize, base as usize + asize);
        // Pass 1: lengths (`usize::MAX` for NULL) and the total.
        let mut ptrs = [core::ptr::null::<u8>(); MAX];
        let mut lens = [usize::MAX; MAX];
        let last_of = |p: *const u8, len: usize| {
            // SAFETY: `p` is readable for `len > 0` bytes (callers).
            unsafe { p.wrapping_add(len - 1).read() }
        };
        let mut total = usize::from(size > 0 && last_of(base, size) != sep);
        for i in 0..n {
            // SAFETY: the caller passes `nbuf` pointers.
            let p = unsafe { segments.wrapping_add(i).read() }.cast::<u8>();
            if p.is_null() {
                continue;
            }
            let pa = p as usize;
            let len = if size > 0 && pa >= lo && pa < lo + size {
                // C: the rest of the original contents.
                size - (pa - lo)
            } else {
                // SAFETY: a non-null segment is NUL-terminated.
                let len = unsafe { libc::strlen(p.cast()) };
                if pa < hi && pa + len + 1 > lo {
                    return false;
                }
                len
            };
            ptrs[i] = p;
            lens[i] = len;
            total += len + usize::from(len == 0 || last_of(p, len) != sep);
        }
        if total == 0 {
            return true;
        }
        if total >= asize - size {
            return false;
        }
        // Pass 2, as C. Writes go at `out >= size`; contents segments are
        // read below `size`, so they are not overwritten.
        let limit = asize - 1;
        let mut out = size;
        let mut last = if size > 0 { last_of(base, size) } else { 0 };
        if size > 0 && last != sep {
            // SAFETY: `out = size < limit`.
            unsafe { base.wrapping_add(out).write(sep) };
            out += 1;
            last = sep;
        }
        for i in 0..n {
            let (mut p, mut len) = (ptrs[i], lens[i]);
            if len == usize::MAX {
                continue;
            }
            if out > 0 && last == sep {
                // SAFETY: `p` is readable for `len` bytes.
                while len > 0 && unsafe { p.read() } == sep {
                    p = p.wrapping_add(1);
                    len -= 1;
                }
            }
            let len = len.min(limit - out);
            if len > 0 {
                // SAFETY: `p` is readable for `len` bytes; the target lies
                // within `out..limit`; `copy` allows a contents segment to
                // overlap (it does not: it lies below `size <= out`).
                unsafe { ptr::copy(p, base.wrapping_add(out), len) };
                out += len;
                last = last_of(base, out);
            }
            if i + 1 < n && out > 0 && last != sep && out < limit {
                // SAFETY: `out < limit`.
                unsafe { base.wrapping_add(out).write(sep) };
                out += 1;
                last = sep;
            }
        }
        // SAFETY: `out <= limit < asize`.
        unsafe { base.wrapping_add(out).write(0) };
        b.size = out;
        true
    }

    /// `git_str__join_n` when the fast path does not apply (out of line).
    ///
    /// # Safety
    ///
    /// As `git_str__join_n`.
    #[cold]
    #[inline(never)]
    unsafe fn join_n_slow(
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
            // A segment that starts within the contents is the rest of the
            // original contents from there (C's special case).
            let contents = b.as_bytes();
            let (start, size) = (contents.as_ptr() as usize, contents.len());
            let inside =
                |p: *const c_char| size > 0 && (p as usize) >= start && (p as usize) < start + size;
            let (a0, a1) = if b.owned() {
                (b.ptr as usize, b.ptr as usize + b.asize)
            } else {
                (0, 0)
            };
            let mut stack: [Option<Seg<'_>>; 8] = [None; 8];
            let mut heap: Vec<Option<Seg<'_>>> = Vec::new();
            let segs: &mut [Option<Seg<'_>>] = if n <= stack.len() {
                &mut stack[..n]
            } else {
                if try_reserve(&mut heap, n).is_err() {
                    return ErrorCode::GENERIC.raw();
                }
                heap.resize(n, None);
                &mut heap
            };
            let mut overlap = false;
            for (slot, &p) in segs.iter_mut().zip(ptrs) {
                if p.is_null() {
                    continue;
                }
                if inside(p) {
                    let off = p as usize - start;
                    *slot = Some(Seg::Own {
                        off,
                        len: size - off,
                    });
                    continue;
                }
                // SAFETY: a non-null segment is NUL-terminated and readable
                // for the call (one that overlaps the allocation outside
                // the contents is copied below, before the buffer changes).
                let x = unsafe { cstr_bytes(p) };
                let (d0, d1) = (x.as_ptr() as usize, x.as_ptr() as usize + x.len() + 1);
                overlap |= d0 < a1 && d1 > a0;
                *slot = Some(Seg::Ext(x));
            }
            if !overlap {
                return status(b.join_segs(separator as u8, segs));
            }
            // Rare: copy the segments that overlap the allocation (with
            // fallible allocations).
            let mut copies: Vec<Option<Vec<u8>>> = Vec::new();
            if try_reserve(&mut copies, n).is_err() {
                return ErrorCode::GENERIC.raw();
            }
            for sg in segs.iter() {
                let c = match sg {
                    Some(Seg::Ext(x))
                        if (x.as_ptr() as usize) < a1 && x.as_ptr() as usize + x.len() + 1 > a0 =>
                    {
                        let mut v = Vec::new();
                        if try_reserve(&mut v, x.len()).is_err() {
                            return ErrorCode::GENERIC.raw();
                        }
                        v.extend_from_slice(x);
                        Some(v)
                    }
                    _ => None,
                };
                copies.push(c);
            }
            let mut fixed: Vec<Option<Seg<'_>>> = Vec::new();
            if try_reserve(&mut fixed, n).is_err() {
                return ErrorCode::GENERIC.raw();
            }
            fixed.extend(segs.iter().zip(&copies).map(|(sg, c)| match c {
                Some(c) => Some(Seg::Ext(c)),
                None => *sg,
            }));
            status(b.join_segs(separator as u8, &fixed))
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
        // SAFETY: the module's contract; NULL or NUL-terminated strings.
        if unsafe { join_fast(buf, separator as u8, str_a, str_b) } {
            return 0;
        }
        // SAFETY: forwarded.
        unsafe { join_slow(buf, separator, str_a, str_b) }
    }

    /// `git_str_join`'s hot path: the result fits the allocation, and the
    /// arguments lie outside it (or `str_a` within the contents, the
    /// `git_str_joinpath(buf, buf->ptr, …)` idiom). Raw copies, no panic;
    /// `false` (nothing done) when it does not apply.
    ///
    /// # Safety
    ///
    /// As `git_str_join`.
    #[inline(always)]
    unsafe fn join_fast(
        buf: *mut GitStr,
        sep: u8,
        str_a: *const c_char,
        str_b: *const c_char,
    ) -> bool {
        // SAFETY: the module's contract.
        let b = unsafe { s(buf) };
        if !b.owned() || b.size >= b.asize || str_b.is_null() {
            return false;
        }
        let (base, asize, size) = (b.ptr.cast::<u8>(), b.asize, b.size);
        let (lo, hi) = (base as usize, base as usize + asize);
        // SAFETY: `str_b` is NUL-terminated.
        let mut lb = unsafe { libc::strlen(str_b) };
        let pb = str_b as usize;
        if pb < hi && pb + lb + 1 > lo {
            return false;
        }
        let (la, a_off) = if str_a.is_null() {
            (0, None)
        } else {
            // SAFETY: a non-null `str_a` is NUL-terminated.
            let la = unsafe { libc::strlen(str_a) };
            let pa = str_a as usize;
            if pa < hi && pa + la + 1 > lo {
                // Within the allocation: only the contents case is fast.
                if pa < lo || pa - lo + la > size {
                    return false;
                }
                (la, Some(pa - lo))
            } else {
                (la, None)
            }
        };
        let mut from_b = str_b.cast::<u8>();
        let mut need_sep = 0usize;
        if sep != 0 && la > 0 {
            // SAFETY: `from_b` stays within `str_b`'s `lb` bytes.
            while lb > 0 && unsafe { from_b.read() } == sep {
                from_b = from_b.wrapping_add(1);
                lb -= 1;
            }
            let last_a = match a_off {
                Some(off) => base.wrapping_add(off + la - 1),
                None => str_a.cast::<u8>().wrapping_add(la - 1),
            };
            // SAFETY: the last byte of `str_a` (contents or external).
            if unsafe { last_a.read() } != sep {
                need_sep = 1;
            }
        }
        let total = la + need_sep + lb;
        if total >= asize {
            return false;
        }
        match a_off {
            Some(0) => {}
            Some(off) => {
                // SAFETY: `off..off + la` lies within the contents, the
                // target `0..la` within the allocation; `copy` allows overlap.
                unsafe { ptr::copy(base.wrapping_add(off), base, la) };
            }
            None if la > 0 => {
                // SAFETY: `str_a` (outside the allocation) is readable for
                // `la` bytes; `la <= total < asize`.
                unsafe { ptr::copy_nonoverlapping(str_a.cast::<u8>(), base, la) };
            }
            None => {}
        }
        if need_sep == 1 {
            // SAFETY: `la < total < asize`.
            unsafe { base.wrapping_add(la).write(sep) };
        }
        // SAFETY: `from_b` (outside the allocation) is readable for `lb`
        // bytes, written below `total < asize`.
        unsafe { ptr::copy_nonoverlapping(from_b, base.wrapping_add(la + need_sep), lb) };
        // SAFETY: `total < asize`.
        unsafe { base.wrapping_add(total).write(0) };
        b.size = total;
        true
    }

    /// `git_str_join` when the fast path does not apply (out of line).
    ///
    /// # Safety
    ///
    /// As `git_str_join`.
    #[cold]
    #[inline(never)]
    unsafe fn join_slow(
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
            let (a_len, b_len) = (a_bytes.len(), b_bytes.len());
            // The usual case: neither lies in the buffer's allocation.
            let alloc = if b.owned() {
                b.ptr as usize..b.ptr as usize + b.asize
            } else {
                0..0
            };
            let clear = |x: &[u8]| {
                let (d0, d1) = (x.as_ptr() as usize, x.as_ptr() as usize + x.len());
                x.is_empty() || d1 <= alloc.start || d0 >= alloc.end
            };
            if clear(b_bytes) {
                if clear(a_bytes) {
                    let a = (!str_a.is_null()).then_some(Seg::Ext(a_bytes));
                    return status(b.join_seg(separator as u8, a, b_bytes));
                }
                // The `git_str_joinpath(buf, buf->ptr, …)` idiom: `str_a`
                // within the contents (C's `offset_a`).
                let contents = b.as_bytes();
                let start = contents.as_ptr() as usize;
                let off = (str_a as usize).wrapping_sub(start);
                if (str_a as usize) >= start && off + a_len <= contents.len() {
                    let a = Some(Seg::Own { off, len: a_len });
                    return status(b.join_seg(separator as u8, a, b_bytes));
                }
            }
            // `str_a` within the contents is C's `offset_a` case; either is
            // copied if it overlaps the allocation otherwise.
            // SAFETY: `a_len` readable bytes, as just measured.
            let a = (!str_a.is_null()).then(|| unsafe { src(b, str_a, a_len) });
            // SAFETY: `b_len` readable bytes, as just measured.
            let bb = unsafe { src(b, str_b, b_len) };
            let bb_copy;
            let bb: &[u8] = match bb.seg() {
                Seg::Ext(x) => x,
                Seg::Own { off, len } => {
                    // Only past the contents' end, which the assertion
                    // excludes; copy for safety.
                    bb_copy = b.as_bytes()[off..off + len].to_vec();
                    &bb_copy
                }
            };
            let a = a.as_ref().map(Src::seg);
            status(b.join_seg(separator as u8, a, bb))
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

// In-place access for the filesystem layer (WP u3): writing path bytes in
// place, and reading into the spare room (`fs_path`, `futils`).
impl GitStr {
    /// The contents of an owned buffer, for writing in place (empty for
    /// any other state: a borrowed buffer is never written).
    #[inline]
    pub fn as_mut_bytes(&mut self) -> &mut [u8] {
        self.content_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `put` into an empty string that owns nothing: one allocation of the
    /// size `try_grow` picks, as the general path makes it.
    #[test]
    fn put_first_allocation() {
        let _g = crate::util::test_support::init();
        for n in [1usize, 6, 7, 8, 100] {
            let data = vec![b'z'; n];
            let mut a = GitStr::new();
            a.put(&data).unwrap();
            let mut b = GitStr::new();
            b.put_seg(Seg::Ext(&data)).unwrap();
            assert_eq!(a.as_bytes(), &data[..]);
            assert_eq!((a.len(), a.capacity()), (b.len(), b.capacity()));
            assert_eq!(a.capacity(), (n + 1 + 7) & !7);
        }
    }

    /// `put`'s fast path (room in an owned buffer) and its boundary with
    /// the growing path: the contents and the NUL are those of C.
    #[test]
    fn put_fast_path_and_growth() {
        let _g = crate::util::test_support::init();
        let mut s = GitStr::with_capacity(8).unwrap();
        let cap = s.capacity();
        s.put(b"").unwrap();
        assert_eq!(s.as_bytes(), b"");
        // Exactly fills the buffer, NUL included: no growth.
        let fill = vec![b'a'; cap - 1];
        s.put(&fill).unwrap();
        assert_eq!(s.capacity(), cap);
        assert_eq!(s.as_bytes(), &fill[..]);
        // One more byte must grow.
        s.put(b"b").unwrap();
        assert!(s.capacity() > cap);
        assert_eq!(s.len(), cap);
        assert_eq!(s.as_bytes()[cap - 1], b'b');
        let nul = s.as_ptr().wrapping_add(s.len());
        // SAFETY: the owned buffer holds the NUL after the contents.
        assert_eq!(unsafe { *nul }, 0);
        // A borrowed string is never written in place.
        let data = b"xyz";
        let mut b = GitStr::new();
        // SAFETY: `data` outlives `b` and is not written.
        unsafe { b.attach_notowned(data.as_ptr().cast(), data.len()) };
        // (`ENSURE_SIZE` returns -1 whatever the growth failed with.)
        assert_eq!(b.put(b"q").unwrap_err(), ErrorCode::GENERIC);
        assert_eq!(b.as_bytes(), b"xyz");
    }

    /// F-5: segments of the string's own contents are checked, whatever
    /// the crate-internal caller passes.
    #[test]
    fn own_segments_are_bounds_checked() {
        let _g = crate::util::test_support::init();
        let mut s = GitStr::with_capacity(64).unwrap();
        s.puts(b"abc").unwrap();
        let bad = [
            Seg::Own {
                off: 1,
                len: 1 << 20,
            },
            Seg::Own { off: 4, len: 1 },
            Seg::Own {
                off: usize::MAX,
                len: 2,
            },
        ];
        for seg in bad {
            assert!(s.put_seg(seg).is_err());
            assert!(s.set_seg(seg).is_err());
            assert!(s.join_seg(0, Some(seg), b"").is_err());
            assert!(s.join_segs(b'/', &[Some(seg)]).is_err());
            assert_eq!(s.as_bytes(), b"abc");
        }
        assert_eq!(
            errors::last_message().unwrap(),
            b"unrecoverable internal error: 'segment within the contents'"
        );
        // In bounds, they work.
        s.put_seg(Seg::Own { off: 0, len: 3 }).unwrap();
        assert_eq!(s.as_bytes(), b"abcabc");
    }

    /// F-1: `git_str__initstr` with a capacity is never taken as owned.
    #[test]
    fn initstr_is_never_owned() {
        let _g = crate::util::test_support::init();
        // SAFETY: an invalid state C cannot make, built to check that the
        // string does not trust it: nothing is read, written or freed.
        let mut s = unsafe { GitStr::from_raw_parts(initstr(), 8, 0) };
        assert!(!s.owns_buffer());
        assert!(s.as_bytes().is_empty());
        s.puts(b"x").unwrap();
        assert_eq!(s.as_bytes(), b"x");
    }
}
