//! `src/libgit2/remote.c` (C that stays C).

use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr;

use crate::repository::GitRepository;
use crate::repository::handles::check;
use crate::util::errors::Result;

unsafe extern "C" {
    fn git_remote_create(
        out: *mut *mut c_void,
        repo: *mut c_void,
        name: *const c_char,
        url: *const c_char,
    ) -> c_int;
    fn git_remote_free(remote: *mut c_void);
}

/// `git_remote_create` of a remote that is freed at once.
pub fn create(repo: &GitRepository, name: &CStr, url: &CStr) -> Result<()> {
    let mut r = ptr::null_mut();
    // SAFETY: a live repository, NUL-terminated strings, a local out.
    check(unsafe { git_remote_create(&mut r, repo.as_ptr(), name.as_ptr(), url.as_ptr()) })?;
    // SAFETY: the remote C returned to us, freed once.
    unsafe { git_remote_free(r) };
    Ok(())
}
