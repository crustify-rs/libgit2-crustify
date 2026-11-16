//! `src/libgit2/ignore.c` (C that stays C): `git_index_add_all`'s check
//! that a pathspec names no ignored file. Safe wrapper:
//! `crate::index::foreign`.

use core::ffi::{c_int, c_void};

use crate::util::vector::GitVector;

unsafe extern "C" {
    /// `git_ignore__check_pathspec_for_exact_ignores`.
    pub fn git_ignore__check_pathspec_for_exact_ignores(
        repo: *mut c_void,
        pathspec: *mut GitVector,
        no_fnmatch: bool,
    ) -> c_int;
}
