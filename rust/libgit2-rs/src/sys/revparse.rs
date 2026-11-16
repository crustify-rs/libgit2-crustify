//! `src/libgit2/revparse.c` (stays C): what the revwalk calls.

use core::ffi::{c_char, c_int, c_uint};

use super::pending::l_revwalk::{git_object, git_repository};

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

/// `GIT_REVSPEC_SINGLE`.
pub const GIT_REVSPEC_SINGLE: c_uint = 1 << 0;
/// `GIT_REVSPEC_RANGE`.
pub const GIT_REVSPEC_RANGE: c_uint = 1 << 1;
/// `GIT_REVSPEC_MERGE_BASE`.
pub const GIT_REVSPEC_MERGE_BASE: c_uint = 1 << 2;

unsafe extern "C" {
    /// `git_revparse`.
    pub fn git_revparse(
        revspec: *mut git_revspec,
        repo: *mut git_repository,
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
