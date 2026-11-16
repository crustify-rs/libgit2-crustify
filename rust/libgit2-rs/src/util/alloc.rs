//! Port of `src/util/alloc.c`: libgit2's global allocator, `git__allocator`,
//! and the allocation helpers over it (PORTING.md §4.3).
//!
//! C reads `git__allocator` directly (`alloc.h` inlines `git__malloc`,
//! `git__realloc` and `git__free`), and `GIT_OPT_SET_ALLOCATOR` and the
//! tests' failing allocators (`cl_alloc_limit`) replace it. Until the library
//! is initialized it is the failing allocator.
//!
//! Memory that C may free, `realloc` or keep must come from this allocator.
//! Rust code gets it through:
//!
//! - [`malloc`], [`realloc`], [`free`], [`calloc`], [`strdup`]…: the C
//!   helpers, raw pointers, setting the out-of-memory error on failure as
//!   `git__malloc` does;
//! - [`GitAlloc`]: the same family as the standard `Allocator` (ffibox's
//!   `CAllocator`), so `Vec<T, GitAlloc>` / `Box<T, GitAlloc>` hold memory C
//!   frees with `git__free`, and as the storage policy of ffibox's owners
//!   (`CBox<Foo, GitAlloc>`, [`GitCStr`], `CVec<T, GitAlloc>`).
//!
//! ```
//! use libgit2_rs::util::alloc::{GitAlloc, GIT_ALLOC};
//! # let _g = libgit2_rs::util::test_support::init();
//! let mut v: Vec<u32, GitAlloc> = Vec::new_in(GIT_ALLOC);
//! v.try_reserve(16).expect("allocation"); // fails with the OOM error set
//! v.push(1);
//! ```

use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicPtr, Ordering};

use ffibox::{CAllocator, CLenAlloc, CLenDrop, CStrBox};

use super::allocators::{failalloc, stdalloc};
use super::errors::{self, Result};

/// `gmalloc`: allocate `n` bytes.
pub type MallocFn = unsafe extern "C" fn(n: usize, file: *const c_char, line: c_int) -> *mut c_void;
/// `grealloc`: C's `realloc`.
pub type ReallocFn =
    unsafe extern "C" fn(ptr: *mut c_void, size: usize, file: *const c_char, line: c_int) -> *mut c_void;
/// `gfree`: C's `free`.
pub type FreeFn = unsafe extern "C" fn(ptr: *mut c_void);

/// C's `git_allocator` (`include/git2/sys/alloc.h`): mirrored (kind 3).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GitAllocator {
    /// Allocate `n` bytes.
    pub gmalloc: Option<MallocFn>,
    /// Resize an allocation, as `realloc`.
    pub grealloc: Option<ReallocFn>,
    /// Free an allocation; NULL is a no-op.
    pub gfree: Option<FreeFn>,
}

/// The global allocator's storage: three function pointers with the layout
/// of `git_allocator`, as atomics, so that Rust's accesses never race with a
/// concurrent replacement. (C reads the fields with plain loads; replacing
/// the allocator while another thread allocates is a contract violation on
/// the C side, as before.)
#[repr(C)]
pub struct AllocatorCell {
    gmalloc: AtomicPtr<c_void>,
    grealloc: AtomicPtr<c_void>,
    gfree: AtomicPtr<c_void>,
}

impl AllocatorCell {
    const fn new(a: GitAllocator) -> AllocatorCell {
        AllocatorCell {
            gmalloc: AtomicPtr::new(fn_to_ptr_malloc(a.gmalloc)),
            grealloc: AtomicPtr::new(fn_to_ptr_realloc(a.grealloc)),
            gfree: AtomicPtr::new(fn_to_ptr_free(a.gfree)),
        }
    }

    /// The current allocator.
    pub fn get(&self) -> GitAllocator {
        let m = self.gmalloc.load(Ordering::Acquire);
        let r = self.grealloc.load(Ordering::Acquire);
        let f = self.gfree.load(Ordering::Acquire);
        // SAFETY: the cells only ever hold NULL or a pointer made from a
        // function of the matching type (`new` and `set` are the only
        // writers), and a function pointer and a data pointer have the same
        // size and representation on the platforms the port supports.
        unsafe {
            GitAllocator {
                gmalloc: core::mem::transmute::<*mut c_void, Option<MallocFn>>(m),
                grealloc: core::mem::transmute::<*mut c_void, Option<ReallocFn>>(r),
                gfree: core::mem::transmute::<*mut c_void, Option<FreeFn>>(f),
            }
        }
    }

    fn set(&self, a: GitAllocator) {
        self.gmalloc.store(fn_to_ptr_malloc(a.gmalloc), Ordering::Release);
        self.grealloc.store(fn_to_ptr_realloc(a.grealloc), Ordering::Release);
        self.gfree.store(fn_to_ptr_free(a.gfree), Ordering::Release);
    }

    #[inline]
    fn malloc_fn(&self) -> Option<MallocFn> {
        let m = self.gmalloc.load(Ordering::Acquire);
        // SAFETY: as in `get`.
        unsafe { core::mem::transmute::<*mut c_void, Option<MallocFn>>(m) }
    }

    #[inline]
    fn realloc_fn(&self) -> Option<ReallocFn> {
        let r = self.grealloc.load(Ordering::Acquire);
        // SAFETY: as in `get`.
        unsafe { core::mem::transmute::<*mut c_void, Option<ReallocFn>>(r) }
    }

    #[inline]
    fn free_fn(&self) -> Option<FreeFn> {
        let f = self.gfree.load(Ordering::Acquire);
        // SAFETY: as in `get`.
        unsafe { core::mem::transmute::<*mut c_void, Option<FreeFn>>(f) }
    }
}

const fn fn_to_ptr_malloc(f: Option<MallocFn>) -> *mut c_void {
    match f {
        Some(f) => f as *mut c_void,
        None => ptr::null_mut(),
    }
}
const fn fn_to_ptr_realloc(f: Option<ReallocFn>) -> *mut c_void {
    match f {
        Some(f) => f as *mut c_void,
        None => ptr::null_mut(),
    }
}
const fn fn_to_ptr_free(f: Option<FreeFn>) -> *mut c_void {
    match f {
        Some(f) => f as *mut c_void,
        None => ptr::null_mut(),
    }
}

/// libgit2's global allocator, which C reads directly. It fails every
/// allocation until the library is initialized.
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub static git__allocator: AllocatorCell = AllocatorCell::new(failalloc::ALLOCATOR);

/// The `file` argument the Rust passes to the allocator's functions.
const RUST_FILE: &CStr = c"libgit2-rs";

/// The current global allocator.
pub fn current() -> GitAllocator {
    git__allocator.get()
}

/// Allocate `len` bytes with the global allocator (`git__malloc`). On
/// failure, NULL with the out-of-memory error set.
///
/// The memory is uninitialized; C may free it with `git__free`.
#[inline]
pub fn malloc(len: usize) -> *mut c_void {
    let p = match git__allocator.malloc_fn() {
        // SAFETY: the installed allocator is sound to call with any size
        // (`set_allocator`'s contract); the file name is a static string.
        Some(f) => unsafe { f(len, RUST_FILE.as_ptr(), 0) },
        None => ptr::null_mut(),
    };
    if p.is_null() {
        errors::set_oom();
    }
    p
}

/// Resize `ptr` to `size` bytes (`git__realloc`). On failure, NULL with the
/// out-of-memory error set, and `ptr` still allocated.
///
/// # Safety
///
/// `ptr` is NULL or a live allocation of the current global allocator,
/// which this call may free.
#[inline]
pub unsafe fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    let p = match git__allocator.realloc_fn() {
        // SAFETY: the caller passes NULL or a live allocation of this
        // allocator.
        Some(f) => unsafe { f(ptr, size, RUST_FILE.as_ptr(), 0) },
        None => ptr::null_mut(),
    };
    if p.is_null() {
        errors::set_oom();
    }
    p
}

/// Free `ptr` (`git__free`).
///
/// # Safety
///
/// `ptr` is NULL or a live allocation of the current global allocator, not
/// used again.
#[inline]
pub unsafe fn free(ptr: *mut c_void) {
    if let Some(f) = git__allocator.free_fn() {
        // SAFETY: the caller passes NULL or a live allocation of this
        // allocator.
        unsafe { f(ptr) }
    }
}

/// `git__calloc`: `nelem * elsize` zeroed bytes; NULL with the
/// out-of-memory error on overflow or failure.
pub fn calloc(nelem: usize, elsize: usize) -> *mut c_void {
    let Some(size) = nelem.checked_mul(elsize) else {
        errors::set_oom();
        return ptr::null_mut();
    };
    let p = malloc(size);
    if !p.is_null() {
        // SAFETY: `p` is a fresh allocation of `size` bytes.
        unsafe { ptr::write_bytes(p.cast::<u8>(), 0, size) };
    }
    p
}

/// `git__reallocarray`: resize to `nelem * elsize` bytes; NULL with the
/// out-of-memory error on overflow or failure.
///
/// # Safety
///
/// As [`realloc`].
pub unsafe fn reallocarray(ptr: *mut c_void, nelem: usize, elsize: usize) -> *mut c_void {
    let Some(size) = nelem.checked_mul(elsize) else {
        errors::set_oom();
        return core::ptr::null_mut();
    };
    // SAFETY: forwarded.
    unsafe { realloc(ptr, size) }
}

/// `git__mallocarray`.
pub fn mallocarray(nelem: usize, elsize: usize) -> *mut c_void {
    // SAFETY: NULL is always a valid `ptr` for `reallocarray`.
    unsafe { reallocarray(ptr::null_mut(), nelem, elsize) }
}

/// `git__substrdup`: a NUL-terminated copy of `s` in the global allocator.
pub fn substrdup(s: &[u8]) -> *mut c_char {
    let Some(size) = s.len().checked_add(1) else {
        errors::set_oom();
        return ptr::null_mut();
    };
    let p = malloc(size).cast::<u8>();
    if !p.is_null() {
        // SAFETY: `p` is a fresh allocation of `s.len() + 1` bytes, disjoint
        // from `s`.
        unsafe {
            ptr::copy_nonoverlapping(s.as_ptr(), p, s.len());
            p.add(s.len()).write(0);
        }
    }
    p.cast()
}

/// `git__strdup`.
pub fn strdup(s: &CStr) -> *mut c_char {
    substrdup(s.to_bytes())
}

/// `git__strndup`: a copy of at most `n` bytes of `s`, stopping at a NUL.
pub fn strndup(s: &[u8], n: usize) -> *mut c_char {
    let s = &s[..s.len().min(n)];
    let len = memchr::memchr(0, s).unwrap_or(s.len());
    substrdup(&s[..len])
}

/// Replace the global allocator (`git_allocator_setup`); `None` installs the
/// default one (the standard allocator).
///
/// # Safety
///
/// Every function of `allocator` must be set, behave as `malloc`, `realloc`
/// and `free` (returning memory aligned to at least 8 bytes), and be sound
/// to call from any thread for as long as it is installed. No allocation may
/// be in progress, and memory allocated by the previous allocator must not
/// be freed by the new one.
pub unsafe fn set_allocator(allocator: Option<&GitAllocator>) {
    match allocator {
        Some(a) => git__allocator.set(*a),
        None => setup_default(),
    }
}

fn setup_default() {
    git__allocator.set(stdalloc::ALLOCATOR);
}

/// `git_allocator_global_init`: install the default allocator, unless one
/// was installed before initialization.
pub fn global_init() -> Result<()> {
    let installed = git__allocator.gmalloc.load(Ordering::Acquire);
    if installed != fn_to_ptr_malloc(failalloc::ALLOCATOR.gmalloc) {
        return Ok(());
    }
    setup_default();
    Ok(())
}

// ---------------------------------------------------------------------------
// The allocator family for Rust's owners
// ---------------------------------------------------------------------------

/// The global allocator as an ffibox allocation family (`malloc` /
/// `realloc` / `free` through `git__allocator`), aligned to 8 bytes. Every
/// failure sets the out-of-memory error, as `git__malloc` does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct GitAllocFamily;

// SAFETY: `c_drop_len` frees the buffer once with `git__free`, which ignores
// the length, and touches nothing else.
unsafe impl CLenDrop for GitAllocFamily {
    #[inline]
    unsafe fn c_drop_len(&self, ptr: *mut u8, _byte_len: usize) {
        // SAFETY: the caller passes a live allocation of this family that it
        // owns, freed once.
        unsafe { free(ptr.cast()) }
    }
}

// SAFETY: `c_alloc_len` returns a fresh allocation of `byte_len` bytes from
// the global allocator or `None`; the allocator's contract (`set_allocator`)
// is 8-byte alignment; `c_realloc_len` is the allocator's `realloc`, which
// keeps the buffer on failure; `c_drop_len` releases either without reading.
unsafe impl CLenAlloc for GitAllocFamily {
    const ALIGN: usize = 8;

    #[inline]
    fn c_alloc_len(&self, byte_len: usize) -> Option<NonNull<u8>> {
        NonNull::new(malloc(byte_len).cast())
    }

    #[inline]
    unsafe fn c_realloc_len(&self, ptr: *mut u8, _old_len: usize, new_len: usize) -> Option<NonNull<u8>> {
        // SAFETY: the caller passes a live allocation of this family.
        NonNull::new(unsafe { realloc(ptr.cast(), new_len) }.cast())
    }
}

/// libgit2's global allocator as the standard `Allocator`, and as ffibox's
/// storage policy: `Vec<T, GitAlloc>`, `Box<T, GitAlloc>`,
/// `CBox<Foo, GitAlloc>`, `CVec<T, GitAlloc>`, [`GitCStr`].
pub type GitAlloc = CAllocator<GitAllocFamily>;

/// The [`GitAlloc`] value (it is zero-sized).
pub const GIT_ALLOC: GitAlloc = CAllocator::new(GitAllocFamily);

/// An owned NUL-terminated string in libgit2's allocator, which C frees
/// with `git__free`.
pub type GitCStr = CStrBox<GitAlloc>;

/// The exports of `alloc.c`.
pub mod ffi {
    use super::*;
    use crate::util::export::{barrier, bytes};

    /// `git__calloc`.
    ///
    /// # Safety
    ///
    /// None beyond the installed allocator's own contract.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__calloc(nelem: usize, elsize: usize) -> *mut c_void {
        barrier(|| calloc(nelem, elsize))
    }

    /// `git__reallocarray`.
    ///
    /// # Safety
    ///
    /// `ptr` is NULL or a live allocation of the global allocator.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__reallocarray(ptr: *mut c_void, nelem: usize, elsize: usize) -> *mut c_void {
        // SAFETY: forwarded.
        barrier(|| unsafe { reallocarray(ptr, nelem, elsize) })
    }

    /// `git__mallocarray`.
    ///
    /// # Safety
    ///
    /// None.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__mallocarray(nelem: usize, elsize: usize) -> *mut c_void {
        barrier(|| mallocarray(nelem, elsize))
    }

    /// `git__strdup`.
    ///
    /// # Safety
    ///
    /// `str` is a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__strdup(str: *const c_char) -> *mut c_char {
        // SAFETY: the caller passes a NUL-terminated string.
        barrier(|| strdup(unsafe { CStr::from_ptr(str) }))
    }

    /// `git__strndup`: copy at most `n` bytes, stopping at a NUL.
    ///
    /// # Safety
    ///
    /// `str` is readable up to its NUL or `n` bytes, whichever comes first.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__strndup(str: *const c_char, n: usize) -> *mut c_char {
        barrier(|| {
            // SAFETY: `strnlen` reads at most `n` bytes, and stops at a NUL,
            // within what the caller guarantees readable.
            let len = unsafe { libc::strnlen(str, n) };
            // SAFETY: the `len` bytes before the NUL (or the limit) are readable.
            substrdup(unsafe { bytes(str, len) })
        })
    }

    /// `git__substrdup`: copy exactly `n` bytes and a NUL.
    ///
    /// # Safety
    ///
    /// `str` is readable for `n` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__substrdup(str: *const c_char, n: usize) -> *mut c_char {
        // SAFETY: the caller passes `n` readable bytes.
        barrier(|| substrdup(unsafe { bytes(str, n) }))
    }

    /// `git_allocator_global_init`.
    ///
    /// # Safety
    ///
    /// Called during library initialization, before any allocation.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_allocator_global_init() -> c_int {
        barrier(|| crate::util::export::status(global_init()))
    }

    /// `git_allocator_setup` (`GIT_OPT_SET_ALLOCATOR`): copy `*allocator`
    /// into the global allocator, or install the default one for NULL.
    ///
    /// # Safety
    ///
    /// `allocator` is NULL or a valid `git_allocator`, under
    /// [`set_allocator`]'s contract.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_allocator_setup(allocator: *mut GitAllocator) -> c_int {
        barrier(|| {
            // SAFETY: the caller passes NULL or a valid allocator, and
            // vouches for its functions.
            unsafe { set_allocator(allocator.as_ref()) };
            0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::errors::ErrorClass;
    use crate::util::test_support;

    #[test]
    fn helpers_copy_and_terminate() {
        let _g = test_support::init();
        let p = strdup(c"hello");
        // SAFETY: `p` is a NUL-terminated copy, freed once.
        unsafe {
            assert_eq!(CStr::from_ptr(p), c"hello");
            free(p.cast());
        }
        let p = strndup(b"hello\0world", 20);
        // SAFETY: as above.
        unsafe {
            assert_eq!(CStr::from_ptr(p), c"hello");
            free(p.cast());
        }
        let p = strndup(b"hello", 3);
        // SAFETY: as above.
        unsafe {
            assert_eq!(CStr::from_ptr(p), c"hel");
            free(p.cast());
        }
        let p = calloc(4, 4).cast::<u8>();
        // SAFETY: 16 zeroed bytes, freed once.
        unsafe {
            assert_eq!(core::slice::from_raw_parts(p, 16), &[0u8; 16]);
            free(p.cast());
        }
    }

    #[test]
    fn overflow_is_oom() {
        let _g = test_support::init();
        crate::util::errors::clear();
        assert!(calloc(usize::MAX, 2).is_null());
        assert_eq!(crate::util::errors::last_class(), Some(ErrorClass::NOMEMORY));
        assert!(mallocarray(usize::MAX / 2, 3).is_null());
    }

    #[test]
    fn vec_in_git_alloc() {
        let _g = test_support::init();
        let mut v: Vec<u64, GitAlloc> = Vec::new_in(GIT_ALLOC);
        for i in 0..1000 {
            v.push(i);
        }
        assert_eq!(v.iter().sum::<u64>(), 999 * 1000 / 2);
        assert!(v.try_reserve(usize::MAX / 16).is_err());
    }

    #[test]
    fn limited_allocator_fails_with_oom() {
        let g = test_support::exclusive();
        test_support::with_alloc_limit(&g, 10, || {
            crate::util::errors::clear();
            assert!(malloc(11).is_null());
            assert_eq!(crate::util::errors::last_class(), Some(ErrorClass::NOMEMORY));
            let p = malloc(10);
            assert!(!p.is_null());
            assert!(malloc(1).is_null());
            // SAFETY: `p` came from the limited allocator, which is installed.
            unsafe { free(p) };
            let p = malloc(1);
            assert!(!p.is_null());
            // SAFETY: as above.
            unsafe { free(p) };
        });
    }
}

crate::c_layout_test! {
    headers: ["alloc.h"],
    types: [
        GitAllocator => "git_allocator" { gmalloc, grealloc, gfree },
        AllocatorCell => "git_allocator" {},
    ],
}
