//! `src/libgit2/revparse.c` (C that stays C).

use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr;

use crate::repository::GitRepository;
use crate::repository::handles::{Object, Reference};

unsafe extern "C" {
    fn git_revparse_ext(
        object_out: *mut *mut c_void,
        reference_out: *mut *mut c_void,
        repo: *mut c_void,
        spec: *const c_char,
    ) -> c_int;
}

/// `git_revparse_ext`: the C status, the object, and the reference (each
/// owned, or `None`).
pub fn ext<'r>(
    repo: &'r GitRepository,
    spec: &CStr,
) -> (c_int, Option<Object<'r>>, Option<Reference<'r>>) {
    let (mut o, mut r) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: a live repository, a NUL-terminated spec, local outs.
    let e = unsafe { git_revparse_ext(&raw mut o, &raw mut r, repo.as_ptr(), spec.as_ptr()) };
    // SAFETY: what C wrote is NULL or owned by us.
    let o = unsafe { Object::from_raw(o) };
    // SAFETY: as above.
    let r = unsafe { Reference::from_raw(r.cast()) };
    (e, o, r)
}

use core::ffi::c_uint;

use super::pending::l_revwalk::git_object;

/// `git_revspec` (public header, mirrored).
#[repr(C)]
#[derive(Debug)]
pub struct git_revspec {
    /// The left element; the caller frees it.
    pub from: *mut git_object,
    /// The right element; the caller frees it.
    pub to: *mut git_object,
    /// `git_revspec_t` flags.
    pub flags: c_uint,
}

/// `GIT_REVSPEC_MERGE_BASE`.
pub const GIT_REVSPEC_MERGE_BASE: c_uint = 1 << 2;

// The repository is a Rust type C sees as opaque.
#[allow(improper_ctypes)]
unsafe extern "C" {
    /// `git_revparse`.
    pub fn git_revparse(
        revspec: *mut git_revspec,
        repo: *mut GitRepository,
        spec: *const c_char,
    ) -> c_int;
}

crate::c_layout_test! {
    headers: ["git2/revparse.h"],
    types: [
        git_revspec => "git_revspec" { from, to, flags },
    ],
    asserts: [
        "GIT_REVSPEC_SINGLE == 1",
        "GIT_REVSPEC_RANGE == 2",
        "GIT_REVSPEC_MERGE_BASE == 4",
    ],
}
