//! `src/libgit2/submodule.c` (C that stays C): the repository's submodule
//! cache and the calls of `git_repository_reinit_filesystem`.

use core::ffi::{c_char, c_int, c_void};

use crate::repository::GitRepository;

/// `git_submodule_cb`.
pub type SubmoduleCb =
    unsafe extern "C" fn(sm: *mut c_void, name: *const c_char, payload: *mut c_void) -> c_int;

unsafe extern "C" {
    fn git_submodule_cache_init(out: *mut *mut c_void, repo: *mut c_void) -> c_int;
    fn git_submodule_cache_free(cache: *mut c_void) -> c_int;
    fn git_submodule_foreach(repo: *mut c_void, cb: SubmoduleCb, payload: *mut c_void) -> c_int;
    fn git_submodule_open(repo: *mut *mut c_void, sm: *mut c_void) -> c_int;
}

/// `git_submodule_cache_init(&repo->submodule_cache, repo)`.
pub fn cache_init(repo: &GitRepository) -> c_int {
    // SAFETY: a live repository; the out-parameter is its own
    // `submodule_cache` field (an atomic pointer, so C's write through it
    // is allowed while `repo` is shared).
    unsafe { git_submodule_cache_init(repo.submodule_cache_slot(), repo.as_ptr()) }
}

/// `git_submodule_cache_free(cache)`.
///
/// # Safety
///
/// `cache` is NULL or a cache from `git_submodule_cache_init` that nothing
/// uses any more.
pub unsafe fn cache_free(cache: *mut c_void) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { git_submodule_cache_free(cache) }
}

/// `git_submodule_foreach`.
///
/// # Safety
///
/// `cb` is sound to call with `payload` for each submodule of `repo`.
pub unsafe fn foreach(repo: &GitRepository, cb: SubmoduleCb, payload: *mut c_void) -> c_int {
    // SAFETY: a live repository and the caller's contract.
    unsafe { git_submodule_foreach(repo.as_ptr(), cb, payload) }
}

/// `git_submodule_open`: the submodule's repository (owned by the caller).
///
/// # Safety
///
/// `sm` is a live `git_submodule` (as C passes to a foreach callback).
pub unsafe fn open(out: &mut *mut c_void, sm: *mut c_void) -> c_int {
    // SAFETY: the caller's contract and a local out-parameter.
    unsafe { git_submodule_open(out, sm) }
}
