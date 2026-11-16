//! `src/libgit2/pathspec.c` (C that stays C): the pathspecs of
//! `git_index_add_all`, `git_index_remove_all` and `git_index_update_all`
//! (safe wrappers: `crate::index::foreign`), and what a revwalk limited by
//! a pathspec reads of it and calls.

use core::ffi::{c_char, c_int, c_uint};

use crate::strarray::GitStrArray;
use crate::util::pool::GitPool;
use crate::util::thread::GitRefcount;
use crate::util::vector::GitVector;

/// `git_pathspec` (`pathspec.h`), mirrored: the index embeds one on its
/// stack, as the C did.
#[repr(C)]
pub struct GitPathspec {
    /// `rc`.
    pub rc: GitRefcount,
    /// `prefix`.
    pub prefix: *mut c_char,
    /// `pathspec`: the parsed patterns.
    pub pathspec: GitVector,
    /// `pool`.
    pub pool: GitPool,
}

/// `git_pathspec` under the revwalk's name.
#[allow(non_camel_case_types)]
pub type git_pathspec = GitPathspec;

/// `git_attr_fnmatch` (attr_file.h): the fields read here, a prefix.
#[repr(C)]
#[derive(Debug)]
#[allow(non_camel_case_types)]
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
    /// `git_pathspec__init`.
    pub fn git_pathspec__init(ps: *mut GitPathspec, paths: *const GitStrArray) -> c_int;
    /// `git_pathspec__clear`.
    pub fn git_pathspec__clear(ps: *mut GitPathspec);
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
        GitPathspec => "git_pathspec" { rc, prefix, pathspec, pool },
        git_attr_fnmatch => "git_attr_fnmatch" { pattern, length, containing_dir, containing_dir_length, flags },
    ],
    asserts: ["GIT_ATTR_FNMATCH_HASWILD == (1U << 5)"],
}
