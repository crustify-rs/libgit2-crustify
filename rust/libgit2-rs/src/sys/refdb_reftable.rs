//! `src/libgit2/refdb_reftable.c` (C that stays C).

use core::ffi::c_int;

unsafe extern "C" {
    /// `git_reftable_global_init`.
    pub fn git_reftable_global_init() -> c_int;
}
