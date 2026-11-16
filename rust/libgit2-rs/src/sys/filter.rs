//! `src/libgit2/filter.c` (C that stays C): the filter lists of
//! `git_repository_hashfile`, and the global init.

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr;

use crate::repository::GitRepository;
use crate::repository::handles::check;
use crate::util::errors::Result;

unsafe extern "C" {
    fn git_filter_list_load(
        filters: *mut *mut c_void,
        repo: *mut c_void,
        blob: *mut c_void,
        path: *const c_char,
        mode: c_int,
        flags: u32,
    ) -> c_int;
    fn git_filter_list_free(filters: *mut c_void);
    /// `git_filter_global_init`.
    pub fn git_filter_global_init() -> c_int;
}

/// `GIT_FILTER_TO_ODB`.
pub const GIT_FILTER_TO_ODB: c_int = 1;
/// `GIT_FILTER_DEFAULT`.
pub const GIT_FILTER_DEFAULT: c_uint = 0;

/// An owned filter list (NULL when no filter applies).
pub struct FilterList(*mut c_void);

impl FilterList {
    /// No filters.
    pub const fn none() -> FilterList {
        FilterList(ptr::null_mut())
    }

    /// `git_filter_list_load(&fl, repo, NULL, path, mode, flags)`.
    pub fn load(repo: &GitRepository, path: &CStr, mode: c_int, flags: u32) -> Result<FilterList> {
        let mut fl = ptr::null_mut();
        // SAFETY: a live repository, a NUL-terminated path, no blob, a
        // local out-parameter.
        check(unsafe {
            git_filter_list_load(
                &raw mut fl,
                repo.as_ptr(),
                ptr::null_mut(),
                path.as_ptr(),
                mode,
                flags,
            )
        })?;
        Ok(FilterList(fl))
    }

    /// The `git_filter_list *` (NULL for none).
    pub fn as_ptr(&self) -> *mut c_void {
        self.0
    }
}

impl Drop for FilterList {
    fn drop(&mut self) {
        // SAFETY: NULL or the owned list, freed once.
        unsafe { git_filter_list_free(self.0) }
    }
}
