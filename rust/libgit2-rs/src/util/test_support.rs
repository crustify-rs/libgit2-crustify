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
    TestGuard {
        _g: ALLOCATOR_LOCK.read().unwrap_or_else(|e| e.into_inner()),
    }
}

/// Initialize the library, and wait until no other test runs: for tests
/// that replace the global allocator.
pub fn exclusive() -> ExclusiveGuard {
    ensure_init();
    ExclusiveGuard {
        _g: ALLOCATOR_LOCK.write().unwrap_or_else(|e| e.into_inner()),
    }
}

// ---------------------------------------------------------------------------
// A limited allocator (`tests/clar/clar_libgit2_alloc.c`)
// ---------------------------------------------------------------------------

static BYTES_AVAILABLE: AtomicUsize = AtomicUsize::new(0);

/// The blocks the limited allocator handed out, and their sizes. A block
/// is a plain `malloc` one (no header), so a pointer allocated before the
/// limit (by the default allocator, `malloc`) can be told apart and freed
/// as such, and a block allocated under the limit can be freed by the
/// default allocator after it.
static BLOCKS: std::sync::Mutex<Option<std::collections::HashMap<usize, usize>>> =
    std::sync::Mutex::new(None);

fn blocks<R>(f: impl FnOnce(&mut std::collections::HashMap<usize, usize>) -> R) -> R {
    let mut g = BLOCKS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Default::default))
}

unsafe extern "C" fn cl_malloc(len: usize, _file: *const c_char, _line: c_int) -> *mut c_void {
    if len > BYTES_AVAILABLE.load(Ordering::SeqCst) {
        return core::ptr::null_mut();
    }
    // SAFETY: `malloc` may be called with any size (0 is made 1, so that
    // the block is distinct, as clar's allocator's are).
    let p = unsafe { libc::malloc(len.max(1)) };
    if p.is_null() {
        return core::ptr::null_mut();
    }
    BYTES_AVAILABLE.fetch_sub(len, Ordering::SeqCst);
    blocks(|b| b.insert(p as usize, len));
    p
}

unsafe extern "C" fn cl_free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    if let Some(len) = blocks(|b| b.remove(&(ptr as usize))) {
        BYTES_AVAILABLE.fetch_add(len, Ordering::SeqCst);
    }
    // SAFETY: `ptr` is a `malloc` block: one of ours, or one the default
    // allocator (`malloc`) made before the limit; freed once.
    unsafe { libc::free(ptr) };
}

unsafe extern "C" fn cl_realloc(
    ptr: *mut c_void,
    size: usize,
    file: *const c_char,
    line: c_int,
) -> *mut c_void {
    if ptr.is_null() {
        // SAFETY: `cl_malloc` may be called with any size.
        return unsafe { cl_malloc(size, file, line) };
    }
    let Some(old) = blocks(|b| b.get(&(ptr as usize)).copied()) else {
        // A block from before the limit: moved by `realloc`, uncounted.
        // SAFETY: a live `malloc` block of the default allocator.
        return unsafe { libc::realloc(ptr, size) };
    };
    // SAFETY: `cl_malloc` may be called with any size.
    let new = unsafe { cl_malloc(size, file, line) };
    if !new.is_null() {
        // SAFETY: `ptr` holds `old` bytes, `new` at least `size`; they are
        // distinct blocks.
        unsafe {
            core::ptr::copy_nonoverlapping(ptr.cast::<u8>(), new.cast::<u8>(), old.min(size))
        };
        // SAFETY: `ptr` is freed once.
        unsafe { cl_free(ptr) };
    }
    new
}

/// Run `f` with an allocator that fails once more than `bytes` bytes are
/// live (`cl_alloc_limit`), then reinstall the default one
/// (`cl_alloc_reset`).
///
/// Memory may cross the boundary either way: the limited allocator makes
/// plain `malloc` blocks and recognises its own, so memory allocated before
/// `f` may be freed or resized inside it, and memory allocated inside `f`
/// may be freed after it. (This relies on the default allocator being
/// installed on entry, which the exclusive guard ensures in the port's
/// tests: nothing else replaces it.)
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
    let limited = GitAllocator {
        gmalloc: Some(cl_malloc),
        grealloc: Some(cl_realloc),
        gfree: Some(cl_free),
    };
    // SAFETY: the limited allocator behaves as `malloc`/`realloc`/`free`
    // with 8-byte alignment; the exclusive guard keeps every other test,
    // and so every other allocation, out until `Reset` reinstalls the
    // default allocator.
    unsafe { alloc::set_allocator(Some(&limited)) };
    let _reset = Reset;
    f()
}
