//! `src/libgit2/submodule.c` (C that stays C): the repository's submodule
//! cache, the calls of `git_repository_reinit_filesystem`, and
//! `git_index_add_bypath` of a directory that is a submodule (safe
//! wrappers of those: `crate::index::foreign`).

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
    /// `git_submodule_lookup`.
    pub fn git_submodule_lookup(
        out: *mut *mut c_void,
        repo: *mut c_void,
        name: *const c_char,
    ) -> c_int;
    /// `git_submodule_add_to_index`.
    pub fn git_submodule_add_to_index(sm: *mut c_void, write_index: c_int) -> c_int;
    /// `git_submodule_free`.
    pub fn git_submodule_free(sm: *mut c_void);
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
    // SAFETY: `cache` is NULL (which the C accepts) or a cache from
    // `git_submodule_cache_init` that nothing uses any more (this
    // function's contract), freed once.
    unsafe { git_submodule_cache_free(cache) }
}

/// `git_submodule_foreach`.
///
/// # Safety
///
/// `cb` is sound to call with `payload` for each submodule of `repo`.
pub unsafe fn foreach(repo: &GitRepository, cb: SubmoduleCb, payload: *mut c_void) -> c_int {
    // SAFETY: a live repository (C reads its paths and config); `cb` may be
    // called with `payload` for each submodule (this function's
    // contract).
    unsafe { git_submodule_foreach(repo.as_ptr(), cb, payload) }
}

/// `git_submodule_open`: the submodule's repository (owned by the caller).
///
/// # Safety
///
/// `sm` is a live `git_submodule` (as C passes to a foreach callback).
pub unsafe fn open(out: &mut *mut c_void, sm: *mut c_void) -> c_int {
    // SAFETY: `sm` is a live submodule (this function's contract); `out` is
    // an exclusive reference, writable.
    unsafe { git_submodule_open(out, sm) }
}
