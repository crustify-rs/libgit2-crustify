//! `src/libgit2/diff_driver.c` (C that stays C).

use core::ffi::c_void;

unsafe extern "C" {
    fn git_diff_driver_registry_free(reg: *mut c_void);
}

/// `git_diff_driver_registry_free`.
///
/// # Safety
///
/// `reg` is NULL or a registry that nothing uses any more.
pub unsafe fn registry_free(reg: *mut c_void) {
    // SAFETY: `reg` is NULL (which the C accepts) or a registry that
    // nothing uses any more (this function's contract), freed once.
    unsafe { git_diff_driver_registry_free(reg) }
}
