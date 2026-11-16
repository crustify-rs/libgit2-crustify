//! Helpers for the port's tests (unit tests, integration tests, and those
//! of later work packages). Not part of the API.
//!
//! - [`init`] initializes the library once per process and returns a guard
//!   that every test holds while it runs. With the `link-c` feature (and not
//!   under Miri) it calls the C `git_libgit2_init`, so the C that stays is
//!   initialized too; otherwise it initializes the Rust subsystems only
//!   (allocator and error state), which is all that Rust-only tests, and
//!   Miri, need.
//! - [`exclusive`] is the guard for a test that replaces the global
//!   allocator ([`with_alloc_limit`]): it waits for every other test to
//!   finish, and blocks them meanwhile.
//!
//! A test that reaches C is marked `#[cfg_attr(miri, ignore)]`.
//!
//! ```
//! # use libgit2_rs::util::test_support;
//! let _g = test_support::init();
//! // … the test …
//! ```

use core::ffi::{c_char, c_int, c_void};
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Once, RwLock, RwLockReadGuard, RwLockWriteGuard};

use super::alloc::{self, GitAllocator};

static INIT: Once = Once::new();
static ALLOCATOR_LOCK: RwLock<()> = RwLock::new(());

/// Held by a test while it runs (see [`init`]).
pub struct TestGuard {
    _g: RwLockReadGuard<'static, ()>,
}

/// Held by a test that replaces the global allocator (see [`exclusive`]).
pub struct ExclusiveGuard {
    _g: RwLockWriteGuard<'static, ()>,
}

#[cfg(all(feature = "link-c", not(miri)))]
fn init_library() {
    unsafe extern "C" {
        fn git_libgit2_init() -> c_int;
    }
    // SAFETY: `git_libgit2_init` may be called at any time; the count it
    // takes is never released, so the library stays initialized for the
    // process.
    let r = unsafe { git_libgit2_init() };
    assert!(r > 0, "git_libgit2_init failed: {r}");
}

#[cfg(not(all(feature = "link-c", not(miri))))]
fn init_library() {
    unsafe extern "C" fn alloc_init() -> c_int {
        crate::util::export::status(alloc::global_init())
    }
    unsafe extern "C" fn error_init() -> c_int {
        crate::util::export::status(crate::util::errors::global_init())
    }
    // SAFETY: the two init functions are the Rust subsystems' own, sound to
    // call during initialization.
    let r = unsafe { super::runtime::init(&[alloc_init, error_init]) };
    assert!(r > 0, "runtime init failed: {r}");
}

fn ensure_init() {
    INIT.call_once(init_library);
}

/// Initialize the library (once per process), and hold the guard that
/// keeps [`exclusive`] tests out while this one runs.
pub fn init() -> TestGuard {
    ensure_init();
    TestGuard { _g: ALLOCATOR_LOCK.read().unwrap_or_else(|e| e.into_inner()) }
}

/// Initialize the library, and wait until no other test runs: for tests
/// that replace the global allocator.
pub fn exclusive() -> ExclusiveGuard {
    ensure_init();
    ExclusiveGuard { _g: ALLOCATOR_LOCK.write().unwrap_or_else(|e| e.into_inner()) }
}

// ---------------------------------------------------------------------------
// A limited allocator (`tests/clar/clar_libgit2_alloc.c`)
// ---------------------------------------------------------------------------

static BYTES_AVAILABLE: AtomicUsize = AtomicUsize::new(0);
const HDR: usize = core::mem::size_of::<usize>();

unsafe extern "C" fn cl_malloc(len: usize, _file: *const c_char, _line: c_int) -> *mut c_void {
    if len > BYTES_AVAILABLE.load(Ordering::SeqCst) {
        return core::ptr::null_mut();
    }
    let Some(total) = len.checked_add(HDR) else {
        return core::ptr::null_mut();
    };
    // SAFETY: `malloc` may be called with any size.
    let p = unsafe { libc::malloc(total) }.cast::<u8>();
    if p.is_null() {
        return core::ptr::null_mut();
    }
    BYTES_AVAILABLE.fetch_sub(len, Ordering::SeqCst);
    // SAFETY: `p` holds `HDR + len` bytes; the header is written unaligned.
    unsafe {
        p.cast::<usize>().write_unaligned(len);
        p.add(HDR).cast()
    }
}

unsafe extern "C" fn cl_free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: `ptr` comes from `cl_malloc`: its block starts `HDR` bytes
    // before it, with the length.
    unsafe {
        let block = ptr.cast::<u8>().sub(HDR);
        let len = block.cast::<usize>().read_unaligned();
        libc::free(block.cast());
        BYTES_AVAILABLE.fetch_add(len, Ordering::SeqCst);
    }
}

unsafe extern "C" fn cl_realloc(ptr: *mut c_void, size: usize, file: *const c_char, line: c_int) -> *mut c_void {
    let copy = if ptr.is_null() {
        0
    } else {
        // SAFETY: as in `cl_free`.
        unsafe { ptr.cast::<u8>().sub(HDR).cast::<usize>().read_unaligned() }
    };
    // SAFETY: `cl_malloc` may be called with any size.
    let new = unsafe { cl_malloc(size, file, line) };
    if !new.is_null() && !ptr.is_null() {
        // SAFETY: `ptr` holds `copy` bytes, `new` at least `size`; they are
        // distinct blocks. `ptr` is then freed once.
        unsafe {
            core::ptr::copy_nonoverlapping(ptr.cast::<u8>(), new.cast::<u8>(), copy.min(size));
            cl_free(ptr);
        }
    }
    new
}

/// Run `f` with an allocator that fails once more than `bytes` bytes are
/// live (`cl_alloc_limit`), then reinstall the default one
/// (`cl_alloc_reset`). Memory allocated inside `f` must be freed inside it.
pub fn with_alloc_limit<R>(_g: &ExclusiveGuard, bytes: usize, f: impl FnOnce() -> R) -> R {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            // SAFETY: the exclusive guard keeps other tests out, so no
            // allocation is in progress.
            unsafe { alloc::set_allocator(None) };
        }
    }
    BYTES_AVAILABLE.store(bytes, Ordering::SeqCst);
    let limited = GitAllocator { gmalloc: Some(cl_malloc), grealloc: Some(cl_realloc), gfree: Some(cl_free) };
    // SAFETY: the limited allocator behaves as `malloc`/`realloc`/`free`
    // with 8-byte alignment; the exclusive guard keeps every other test,
    // and so every other allocation, out until `Reset` reinstalls the
    // default allocator.
    unsafe { alloc::set_allocator(Some(&limited)) };
    let _reset = Reset;
    f()
}
