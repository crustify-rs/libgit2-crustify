//! String arguments that lie in the call's own output buffer (unsafe-r1
//! U-1, T-1): the exports read them where C does (after the output is
//! sanitized or cleared) and never keep a reference to them across a write
//! of the buffer. Pure Rust paths, so Miri checks the aliasing.

// C-style test code: each test drives the exports under one contract.
#![allow(
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks
)]

use core::ffi::{c_char, c_int};

use libgit2_rs::__test::buf::GitBuf;
use libgit2_rs::__test::util::alloc;
use libgit2_rs::__test::util::str::GitStr;
use libgit2_rs::__test::util::test_support;

/// `git_buf`'s layout, to build and read one as C does.
#[repr(C)]
struct RawBuf {
    ptr: *mut c_char,
    reserved: usize,
    size: usize,
}

unsafe extern "C" {
    fn git_config_parse_path(out: *mut GitBuf, value: *const c_char) -> c_int;
}

/// A `git_buf` owning a copy of `s` (with room for its NUL).
fn buf_of(s: &core::ffi::CStr) -> RawBuf {
    let p = alloc::strdup(s);
    assert!(!p.is_null());
    RawBuf {
        ptr: p,
        reserved: s.to_bytes().len() + 1,
        size: s.to_bytes().len(),
    }
}

/// The bytes of a `git_buf`, then free it.
unsafe fn take(b: RawBuf) -> Vec<u8> {
    let v = if b.ptr.is_null() {
        Vec::new()
    } else {
        unsafe { core::slice::from_raw_parts(b.ptr.cast::<u8>(), b.size) }.to_vec()
    };
    unsafe { alloc::free(b.ptr.cast()) };
    v
}

#[test]
fn parse_path_value_at_the_start_of_out() {
    let _g = test_support::init();
    let mut b = buf_of(c"abcde");
    let value = b.ptr.cast_const();
    // C sanitizes `out` (writing its first byte to 0) before it reads
    // `value`: the value is "", and so is the result.
    let rc = unsafe { git_config_parse_path((&raw mut b).cast(), value) };
    assert_eq!(rc, 0);
    assert_eq!(unsafe { take(b) }, b"");
}

#[test]
fn parse_path_value_inside_out() {
    let _g = test_support::init();
    let mut b = buf_of(c"xabcde");
    let value = b.ptr.cast_const().wrapping_add(1);
    // The sanitize clobbers only byte 0: the value is "abcde".
    let rc = unsafe { git_config_parse_path((&raw mut b).cast(), value) };
    assert_eq!(rc, 0);
    assert_eq!(unsafe { take(b) }, b"abcde");
}

unsafe extern "C" {
    fn git__getenv(out: *mut GitStr, name: *const c_char) -> c_int;
}

#[test]
#[cfg_attr(miri, ignore = "getenv reads the process environment")]
fn getenv_name_inside_out() {
    let _g = test_support::init();
    // SAFETY: the test's own environment variable, set before any thread
    // that reads the environment.
    unsafe { std::env::set_var("LIBGIT2_RS_ARGS_IN_OUTPUT", "value") };
    let mut s = GitStr::new();
    s.puts(b"xLIBGIT2_RS_ARGS_IN_OUTPUT").unwrap();
    let name = s.as_ptr().wrapping_add(1);
    // C clears `out` (byte 0) and then reads `name`, from byte 1 on.
    let rc = unsafe { git__getenv(&raw mut s, name) };
    assert_eq!(rc, 0);
    assert_eq!(s.as_bytes(), b"value");
}

#[cfg(all(feature = "link-c", not(miri)))]
mod blob_filter {
    //! `git_blob_filter` with `path` in `out`'s own buffer: C reads `path`
    //! only to load the filters, before `out` is written.
    use super::*;
    use std::path::{Path, PathBuf};

    #[repr(C)]
    struct Oid([u8; 33]);

    unsafe extern "C" {
        fn git_repository_open(out: *mut *mut c_void, path: *const c_char) -> c_int;
        fn git_repository_free(repo: *mut c_void);
        fn git_oid_from_string(out: *mut Oid, s: *const c_char, t: c_int) -> c_int;
        fn git_blob_lookup(out: *mut *mut c_void, repo: *mut c_void, id: *const Oid) -> c_int;
        fn git_blob_free(blob: *mut c_void);
        fn git_blob_filter(
            out: *mut GitBuf,
            blob: *mut c_void,
            path: *const c_char,
            opts: *const c_void,
        ) -> c_int;
    }
    use core::ffi::c_void;

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            let t = to.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                copy_dir(&e.path(), &t);
            } else {
                std::fs::copy(e.path(), &t).unwrap();
            }
        }
    }

    #[test]
    fn path_in_out_buffer() {
        let _g = test_support::init();
        // A scratch copy: the shared fixture is never opened.
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/resources/testrepo.git");
        let dir: PathBuf =
            std::env::temp_dir().join(format!("args_in_output_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        copy_dir(&src, &dir);
        let c = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
        let mut repo = core::ptr::null_mut();
        let mut id = Oid([0; 33]);
        let mut blob = core::ptr::null_mut();
        unsafe {
            assert_eq!(git_repository_open(&raw mut repo, c.as_ptr()), 0);
            assert_eq!(
                git_oid_from_string(
                    &raw mut id,
                    c"a8233120f6ad708f843d861ce2b7228ec4e3dec6".as_ptr(),
                    1
                ),
                0
            );
            assert_eq!(git_blob_lookup(&raw mut blob, repo, &raw const id), 0);
            // The reference result, with a separate path.
            let mut want = buf_of(c"");
            assert_eq!(
                git_blob_filter(
                    (&raw mut want).cast(),
                    blob,
                    c"README".as_ptr(),
                    core::ptr::null()
                ),
                0
            );
            let want = take(want);
            // `path` at byte 1 of `out` (byte 0 is what the sanitize writes).
            let mut b = buf_of(c"xREADME");
            let path = b.ptr.cast_const().wrapping_add(1);
            assert_eq!(
                git_blob_filter((&raw mut b).cast(), blob, path, core::ptr::null()),
                0
            );
            assert_eq!(take(b), want);
            git_blob_free(blob);
            git_repository_free(repo);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
