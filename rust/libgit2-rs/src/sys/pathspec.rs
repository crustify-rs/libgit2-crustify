//! `src/libgit2/pathspec.c` (C that stays C): the pathspecs of
//! `git_index_add_all`, `git_index_remove_all` and `git_index_update_all`.
//! Safe wrappers: `crate::index::foreign`.

use core::ffi::{c_char, c_int};

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
    headers: ["pathspec.h"],
    types: [
        GitPathspec => "git_pathspec" { rc, prefix, pathspec, pool },
    ],
}
