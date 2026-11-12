//! `src/util/allocators/win32_leakcheck.c`: [`allocator`] keeps its contract, checked through
//! Rust assertions: on a build without `GIT_DEBUG_LEAKCHECK_WIN32`, as every variant is, it
//! fails with the class `GIT_ERROR_INVALID`, not the error code C's stub passes as the class,
//! and the thread's last error is the error it returns, whatever the thread held before.
//!
//! No C test covers the module, so none is carried over. Every test calls C, so each is
//! ignored under Miri.

use std::ffi::CStr;

use libgit2::libgit2::src::util::allocators::win32_leakcheck::allocator;
use libgit2::libgit2::src::util::errors::{ErrorClass, clear, last, set_str};

const MESSAGE: &CStr = c"leakcheck memory allocator not available";

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn this_build_has_no_leakcheck_allocator() {
    let error = allocator().unwrap_err();
    assert_eq!(error.class(), Some(ErrorClass::Invalid));
    assert_eq!(error.message(), MESSAGE);
}

/// C's stub passes the error code `GIT_EINVALID` (-21) as the class, a class the header
/// names nowhere; the wrapper reports `GIT_ERROR_INVALID`, in the returned error and in the
/// thread's last error alike.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_stubs_error_code_class_is_corrected() {
    let error = allocator().unwrap_err();
    assert_eq!(error.raw_class(), ErrorClass::Invalid.raw());
    let thread = last();
    assert_eq!(thread.raw_class(), ErrorClass::Invalid.raw());
    assert_eq!(
        thread, error,
        "the thread's last error is the corrected one"
    );
}

/// The call replaces whatever error the thread held, cleared or set, and repeats
/// identically.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn each_call_replaces_the_threads_error() {
    clear();
    let first = allocator().unwrap_err();
    assert_eq!(last(), first);

    set_str(ErrorClass::Odb, c"an older error").unwrap();
    let second = allocator().unwrap_err();
    assert_eq!(second, first);
    assert_eq!(last(), second);
}
