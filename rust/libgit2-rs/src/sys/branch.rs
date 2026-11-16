//! `src/libgit2/branch.c` (C that stays C).

use core::ffi::{c_int, c_void};

use crate::repository::handles::Reference;

unsafe extern "C" {
    fn git_branch_is_checked_out(branch: *const c_void) -> c_int;
}

/// `git_branch_is_checked_out` (1, 0, or an error code).
pub fn is_checked_out(branch: &Reference) -> c_int {
    // SAFETY: a live reference.
    unsafe { git_branch_is_checked_out(branch.as_ptr()) }
}
