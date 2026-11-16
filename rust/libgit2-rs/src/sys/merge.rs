//! `src/libgit2/merge.c` (C that stays C).

use core::ffi::{c_int, c_void};

use crate::oid::Oid;
use crate::repository::GitRepository;

/// `git_repository_mergehead_foreach_cb`.
pub type MergeheadCb = unsafe extern "C" fn(oid: *const Oid, payload: *mut c_void) -> c_int;

unsafe extern "C" {
    fn git_repository_mergehead_foreach(
        repo: *mut c_void,
        cb: MergeheadCb,
        payload: *mut c_void,
    ) -> c_int;
}

/// `git_repository_mergehead_foreach`.
///
/// # Safety
///
/// `cb` is sound to call with `payload` for each merge head.
pub unsafe fn mergehead_foreach(
    repo: &GitRepository,
    cb: MergeheadCb,
    payload: *mut c_void,
) -> c_int {
    // SAFETY: a live repository and the caller's contract.
    unsafe { git_repository_mergehead_foreach(repo.as_ptr(), cb, payload) }
}
