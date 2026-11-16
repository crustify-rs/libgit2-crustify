//! `src/libgit2/attrcache.c` (C that stays C).

use core::ffi::{c_int, c_void};

use crate::repository::GitRepository;

unsafe extern "C" {
    fn git_attr_cache_flush(repo: *mut c_void) -> c_int;
}

/// `git_attr_cache_flush(repo)`: drops the repository's attribute cache
/// (C swaps `repo->attrcache` to NULL atomically).
pub fn flush(repo: &GitRepository) -> c_int {
    // SAFETY: a live repository; C reaches only its `attrcache` field, an
    // atomic pointer.
    unsafe { git_attr_cache_flush(repo.as_ptr()) }
}
