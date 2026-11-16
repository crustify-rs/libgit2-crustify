//! `src/libgit2/pathspec.c` (stays C): what a revwalk limited by a
//! pathspec reads of it and calls.

use core::ffi::{c_char, c_uint};

use crate::util::pool::GitPool;
use crate::util::thread::GitRefcount;
use crate::util::vector::GitVector;

/// `struct git_pathspec` (mirrored). C allocates it; the revwalk only
/// reads `prefix` and `pathspec`.
#[repr(C)]
#[derive(Debug)]
pub struct git_pathspec {
    /// `rc`.
    pub rc: GitRefcount,
    /// `prefix`: the common non-wildcard prefix, or NULL.
    pub prefix: *mut c_char,
    /// `pathspec`: a vector of `git_attr_fnmatch *`.
    pub pathspec: GitVector,
    /// `pool`.
    pub pool: GitPool,
}

/// `git_attr_fnmatch` (attr_file.h): the fields read here, a prefix.
#[repr(C)]
#[derive(Debug)]
pub struct git_attr_fnmatch {
    /// `pattern`.
    pub pattern: *mut c_char,
    /// `length`.
    pub length: usize,
    /// `containing_dir`.
    pub containing_dir: *mut c_char,
    /// `containing_dir_length`.
    pub containing_dir_length: usize,
    /// `flags`.
    pub flags: c_uint,
}

/// `GIT_ATTR_FNMATCH_HASWILD`.
pub const GIT_ATTR_FNMATCH_HASWILD: c_uint = 1 << 5;

unsafe extern "C" {
    /// `git_pathspec__match`.
    pub fn git_pathspec__match(
        vspec: *const GitVector,
        path: *const c_char,
        disable_fnmatch: bool,
        casefold: bool,
        matched_pathspec: *mut *const c_char,
        matched_at: *mut usize,
    ) -> bool;
}

crate::c_layout_test! {
    headers: ["pathspec.h", "attr_file.h"],
    types: [
        git_pathspec => "git_pathspec" { rc, prefix, pathspec, pool },
        git_attr_fnmatch => "git_attr_fnmatch" { pattern, length, containing_dir, containing_dir_length, flags },
    ],
    asserts: ["GIT_ATTR_FNMATCH_HASWILD == (1U << 5)"],
}
