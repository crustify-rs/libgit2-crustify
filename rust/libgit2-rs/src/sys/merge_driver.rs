//! `src/libgit2/merge_driver.c` (C that stays C).

use core::ffi::c_int;

unsafe extern "C" {
    /// `git_merge_driver_global_init`.
    pub fn git_merge_driver_global_init() -> c_int;
}
