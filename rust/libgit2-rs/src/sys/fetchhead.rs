//! `src/libgit2/fetchhead.c` (C that stays C): reading `FETCH_HEAD`.

use core::ffi::{c_char, c_int, c_uint, c_void};

use crate::oid::Oid;
use crate::repository::GitRepository;

/// `git_repository_fetchhead_foreach_cb`.
pub type FetchheadCb = unsafe extern "C" fn(
    ref_name: *const c_char,
    remote_url: *const c_char,
    oid: *const Oid,
    is_merge: c_uint,
    payload: *mut c_void,
) -> c_int;

unsafe extern "C" {
    fn git_repository_fetchhead_foreach(
        repo: *mut c_void,
        cb: FetchheadCb,
        payload: *mut c_void,
    ) -> c_int;
}

/// `git_repository_fetchhead_foreach`.
///
/// # Safety
///
/// `cb` is sound to call with `payload` for each line of `FETCH_HEAD`.
pub unsafe fn fetchhead_foreach(
    repo: &GitRepository,
    cb: FetchheadCb,
    payload: *mut c_void,
) -> c_int {
    // SAFETY: a live repository (C reads its git directory's path and the
    // file) and the caller's contract.
    unsafe { git_repository_fetchhead_foreach(repo.as_ptr(), cb, payload) }
}
