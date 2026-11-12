//! Equivalence of [`allocator`] with `git_win32_leakcheck_init_allocator`.
//!
//! Every variant builds the stub of `src/util/allocators/win32_leakcheck.c`, so both sides
//! fail. Each side runs the same sequence on a fresh thread of its own, so that each starts
//! without thread-local error state: a call with no error set, a call over an older error,
//! and an allocation through libgit2's installed allocator after them. After each step
//! they are compared through the mapping the safe API applies: C's negative return is
//! `Err`, the value C leaves unwritten is no `Allocator`, and the error the call left,
//! returned by the Rust and read by `git_error_last` on both sides, has the same message.
//!
//! Documented divergence, outside the comparison: C's stub passes the error code
//! `GIT_EINVALID` (-21) where `git_error_set` takes a class, and the wrapper reports the
//! class `GIT_ERROR_INVALID` instead, in its result and in the thread's last error. The
//! sequence checks each side's class against its own contract, and the module's
//! `unit_tests` cover the correction.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::thread;

use libgit2::ffi;
use libgit2::libgit2::src::util::alloc::StrFree;
use libgit2::libgit2::src::util::allocators::win32_leakcheck::allocator;
use libgit2::libgit2::src::util::errors::{ErrorClass, last, set_str};

/// What one step observed: whether the call failed, whether it built an allocator, and
/// the message of the thread's last error after it; each side checks its class itself.
#[derive(Debug, PartialEq, Eq)]
struct Step {
    failed: bool,
    built: bool,
    message: CString,
}

/// What the call left as the thread's last error, through `git_error_last`.
fn c_last() -> (c_int, CString) {
    // SAFETY: no arguments; the caller has initialised libgit2, so the thread state is
    // looked up under its live TLS key.
    let error = unsafe { ffi::git_error_last() };
    assert!(!error.is_null());
    // SAFETY: `git_error_last` returns a static error or this thread's error state, live
    // and unchanged until the next libgit2 call on this thread; it is copied out now.
    let error = unsafe { error.read() };
    // SAFETY: the copy's message is the NUL-terminated buffer it pointed at, still live,
    // since no libgit2 call has run since.
    let message = unsafe { CStr::from_ptr(error.message) };
    (error.klass, message.to_owned())
}

unsafe extern "C" fn sentinel_malloc(_: usize, _: *const c_char, _: c_int) -> *mut c_void {
    unreachable!("the sentinel is never installed")
}

unsafe extern "C" fn sentinel_realloc(
    _: *mut c_void,
    _: usize,
    _: *const c_char,
    _: c_int,
) -> *mut c_void {
    unreachable!("the sentinel is never installed")
}

unsafe extern "C" fn sentinel_free(_: *mut c_void) {
    unreachable!("the sentinel is never installed")
}

/// The raw call on a value holding sentinel members, which the stub must leave as they are.
fn c_step() -> Step {
    let sentinel = ffi::git_allocator {
        gmalloc: Some(sentinel_malloc),
        grealloc: Some(sentinel_realloc),
        gfree: Some(sentinel_free),
    };
    let mut raw = sentinel;
    // SAFETY: `raw` is a live `git_allocator` this frame owns exclusively, which C only
    // writes or leaves untouched and does not retain; libgit2 is initialised.
    let rc = unsafe { ffi::git_win32_leakcheck_init_allocator(&raw mut raw) };
    let unwritten = raw.gmalloc.map(|f| f as usize) == sentinel.gmalloc.map(|f| f as usize)
        && raw.grealloc.map(|f| f as usize) == sentinel.grealloc.map(|f| f as usize)
        && raw.gfree.map(|f| f as usize) == sentinel.gfree.map(|f| f as usize);
    let (klass, message) = c_last();
    assert_eq!(rc, ffi::GIT_ERROR, "C returns -1");
    assert_eq!(
        klass,
        ffi::GIT_EINVALID,
        "C passes GIT_EINVALID as the class"
    );
    Step {
        failed: rc < 0,
        built: rc >= 0 && !unwritten,
        message,
    }
}

fn rust_step() -> Step {
    let result = allocator();
    let (klass, message) = c_last();
    assert_eq!(
        klass,
        ffi::GIT_ERROR_INVALID.cast_signed(),
        "the corrected class"
    );
    if let Err(error) = &result {
        assert_eq!(error.class(), Some(ErrorClass::Invalid));
        assert_eq!(
            error.message(),
            message.as_c_str(),
            "the result is the last error"
        );
        assert_eq!(&last(), error);
    }
    Step {
        failed: result.is_err(),
        built: result.is_ok(),
        message,
    }
}

/// The sequence, on a fresh thread: each step's observation.
fn run(step: fn() -> Step) -> Vec<Step> {
    thread::spawn(move || {
        // Initialises libgit2 for the raw side too, and holds it for the process.
        drop(last());
        let mut steps = vec![step()];
        set_str(ErrorClass::Odb, c"an older error").unwrap();
        steps.push(step());
        // libgit2's installed allocator is still the one it was.
        let copy = StrFree::dup(c"still allocates").unwrap();
        assert_eq!(copy.as_bytes(), b"still allocates");
        steps.push(step());
        steps
    })
    .join()
    .unwrap()
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn rust_fails_as_c_does_step_by_step() {
    let c = run(c_step);
    let rust = run(rust_step);
    for (i, (c, rust)) in c.iter().zip(&rust).enumerate() {
        assert_eq!(c, rust, "step {i}");
        assert!(c.failed && !c.built);
        assert_eq!(
            c.message.as_c_str(),
            c"leakcheck memory allocator not available"
        );
    }
    assert_eq!(c.len(), rust.len());
}
