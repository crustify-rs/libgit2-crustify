//! Tests of the error state: the Rust API and the exports.

// These tests call the exports in sequences, as C does; the SAFETY comment
// of each block covers the calls in it.
#![allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]

use core::ffi::{CStr, c_int};
use core::fmt;
use libgit2_rs::util::errors::{self, Bytes, Error, ErrorClass, ErrorCode, GitError, ffi};
use libgit2_rs::util::export::barrier;
use libgit2_rs::util::test_support;
use libgit2_rs::{git_assert, git_assert_arg, git_error_set};

/// What `git_error_last` shows, as C reads it.
fn c_last() -> (c_int, Vec<u8>) {
    // SAFETY: as C: read the returned error before the next error call.
    unsafe {
        let e: *const GitError = ffi::git_error_last();
        ((*e).klass, CStr::from_ptr((*e).message).to_bytes().to_vec())
    }
}

#[test]
fn set_clear_last() {
    let _g = test_support::init();
    errors::clear();
    assert_eq!(c_last(), (0, b"no error".to_vec()));
    assert!(!errors::exists());
    assert_eq!(
        errors::set_str(ErrorClass::CONFIG, "bad config"),
        ErrorCode::GENERIC
    );
    assert_eq!(c_last(), (7, b"bad config".to_vec()));
    assert!(errors::exists());
    errors::set_oom();
    assert_eq!(c_last(), (1, b"Out of memory".to_vec()));
    errors::clear();
    assert_eq!(c_last(), (0, b"no error".to_vec()));
    // SAFETY: as C.
    unsafe {
        assert_eq!(
            ffi::git_error_set_str(ErrorClass::TAG.0, c"from C".as_ptr()),
            0
        );
        assert_eq!(c_last(), (13, b"from C".to_vec()));
        assert_eq!(
            ffi::git_error_set_str(ErrorClass::TAG.0, core::ptr::null()),
            -1
        );
        assert_eq!(c_last(), (3, b"invalid argument: 'string'".to_vec()));
        ffi::git_error_set_oom();
        assert!(ffi::git_error_exists());
        ffi::git_error_clear();
        assert!(!ffi::git_error_exists());
        // Any class an int can hold.
        assert_eq!(ffi::git_error_set_str(1234, c"odd".as_ptr()), 0);
        assert_eq!(c_last(), (1234, b"odd".to_vec()));
    }
}

#[test]
fn formatted_and_os_errors() {
    let _g = test_support::init();
    let r: errors::Result<()> = Err(git_error_set!(ErrorClass::NET, "code {} at {:#x}", 7, 255));
    assert_eq!(r, Err(ErrorCode::GENERIC));
    assert_eq!(c_last(), (12, b"code 7 at 0xff".to_vec()));

    errors::set_errno(libc::ENOENT);
    let _ = git_error_set!(ErrorClass::OS, "failed to open '{}'", Bytes(b"/x\xff"));
    let expect = b"failed to open '/x\xff': No such file or directory";
    if cfg!(miri) {
        // Miri's `strerror_r` appends " (os error 2)" to glibc's text.
        assert!(c_last().1.starts_with(expect));
    } else {
        assert_eq!(c_last(), (2, expect.to_vec()));
    }
    assert_eq!(errors::errno(), 0, "errno is reset");
    let _ = git_error_set!(ErrorClass::OS, "no errno");
    assert_eq!(c_last(), (2, b"no errno: ".to_vec()));

    // The shim's entry point: NULL format, OS class.
    // SAFETY: as the shim calls it.
    unsafe {
        ffi::git_error__set_formatted(ErrorClass::OS.0, core::ptr::null(), 0, libc::EACCES);
        assert!(c_last().1.starts_with(b"Permission denied"));
        ffi::git_error__set_formatted(ErrorClass::OS.0, core::ptr::null(), 0, 0);
        assert_eq!(c_last(), (2, b"".to_vec()));
        ffi::git_error__set_formatted(ErrorClass::REGEX.0, c"abcdef".as_ptr(), 3, 0);
        assert_eq!(c_last(), (8, b"abc".to_vec()));
    }
    // Bytes outside a message is lossy.
    assert_eq!(Bytes(b"a\xffb").to_string(), "a\u{fffd}b");
}

#[test]
fn reentrant_display_is_harmless() {
    let _g = test_support::init();
    struct Sneaky;
    impl fmt::Display for Sneaky {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            // An argument that itself sets an error while being formatted.
            let _ = errors::set_str(ErrorClass::SSL, "inner");
            let _ = git_error_set!(ErrorClass::SSL, "inner {}", 2);
            f.write_str("sneaky")
        }
    }
    let _ = git_error_set!(ErrorClass::FILTER, "outer {} {}", Sneaky, Bytes(b"\xfe"));
    assert_eq!(c_last(), (24, b"outer sneaky \xfe".to_vec()));
}

#[test]
fn assert_macros() {
    let _g = test_support::init();
    fn f(x: Option<u8>) -> errors::Result<u8> {
        git_assert_arg!(x.is_some(), "x");
        git_assert!(x != Some(0), "x != 0");
        Ok(x.unwrap())
    }
    assert_eq!(f(None), Err(ErrorCode::GENERIC));
    assert_eq!(c_last(), (3, b"invalid argument: 'x'".to_vec()));
    assert_eq!(f(Some(0)), Err(ErrorCode::GENERIC));
    assert_eq!(
        c_last(),
        (35, b"unrecoverable internal error: 'x != 0'".to_vec())
    );
    assert_eq!(f(Some(3)), Ok(3));
    fn g(x: c_int) -> c_int {
        git_assert_arg!(x > 0, "x > 0", -7);
        0
    }
    assert_eq!(g(0), -7);
}

#[test]
fn save_and_restore() {
    let _g = test_support::init();
    let _ = errors::set_str(ErrorClass::INDEX, "saved message");
    let s = errors::save();
    errors::clear();
    errors::restore(s);
    assert_eq!(c_last(), (10, b"saved message".to_vec()));

    // The exports.
    let mut e: *mut GitError = core::ptr::null_mut();
    // SAFETY: as C.
    unsafe {
        assert_eq!(ffi::git_error_save(&mut e), 0);
        ffi::git_error_clear();
        assert_eq!(ffi::git_error_restore(e), 0);
        assert_eq!(c_last(), (10, b"saved message".to_vec()));
        ffi::git_error_set_oom();
        assert_eq!(ffi::git_error_save(&mut e), 0);
        ffi::git_error_clear();
        assert_eq!(ffi::git_error_restore(e), 0);
        assert_eq!(c_last(), (1, b"Out of memory".to_vec()));
        // Saving no error, then restoring it, restores "no error" (which
        // then counts as an error set, as in C).
        ffi::git_error_clear();
        assert_eq!(ffi::git_error_save(&mut e), 0);
        assert_eq!(ffi::git_error_restore(e), 0);
        assert!(ffi::git_error_exists());
        assert_eq!(c_last(), (0, b"no error".to_vec()));
        assert_eq!(ffi::git_error_restore(core::ptr::null_mut()), -1);
        assert_eq!(c_last(), (3, b"invalid argument: 'error'".to_vec()));
        ffi::git_error_free(core::ptr::null_mut());
    }
}

#[test]
fn rich_error_and_callbacks() {
    let _g = test_support::init();
    let _ = errors::set_str(ErrorClass::REFERENCE, "ref not found");
    let e = Error::from(ErrorCode::NOT_FOUND);
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);
    assert_eq!(e.class(), ErrorClass::REFERENCE);
    assert_eq!(e.message(), "ref not found");
    assert_eq!(e.to_string(), "ref not found");
    assert!(format!("{e:?}").contains("GIT_ENOTFOUND"));
    assert_eq!(ErrorCode::from_raw(0), None);
    assert_eq!(ErrorCode::check(5), Ok(5));
    assert_eq!(ErrorCode::check(-3), Err(ErrorCode::NOT_FOUND));
    assert_eq!(ErrorCode::check_zero(1).map_err(ErrorCode::raw), Err(1));

    errors::clear();
    assert_eq!(errors::set_after_callback(-5, "foreach"), -5);
    assert_eq!(c_last(), (26, b"foreach callback returned -5".to_vec()));
    let _ = errors::set_str(ErrorClass::TREE, "kept");
    assert_eq!(errors::set_after_callback(3, "walk"), 3);
    assert_eq!(c_last(), (14, b"kept".to_vec()));
}

#[test]
fn errors_are_per_thread() {
    let _g = test_support::init();
    let _ = errors::set_str(ErrorClass::ODB, "main thread");
    std::thread::spawn(|| {
        assert!(!errors::exists());
        let _ = errors::set_str(ErrorClass::ODB, "other thread");
        assert_eq!(c_last().1, b"other thread");
    })
    .join()
    .unwrap();
    assert_eq!(c_last().1, b"main thread");
}

#[test]
fn barrier_turns_panics_into_errors() {
    let _g = test_support::init();
    let r: c_int = barrier(|| panic!("boom"));
    assert_eq!(r, -1);
    assert_eq!(c_last().0, ErrorClass::INTERNAL.0);
    let p: *mut u8 = barrier(|| panic!("boom"));
    assert!(p.is_null());
    assert_eq!(barrier(|| 5), 5);
}

#[test]
fn system_errors() {
    let _g = test_support::init();
    // SAFETY: as C.
    unsafe {
        ffi::git_error_system_set(libc::EEXIST);
        assert_eq!(ffi::git_error_system_last(), libc::EEXIST);
        ffi::git_error_clear();
        assert_eq!(
            ffi::git_error_system_last(),
            0,
            "git_error_clear resets errno"
        );
    }
}
