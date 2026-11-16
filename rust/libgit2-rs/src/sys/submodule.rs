//! `src/libgit2/submodule.c` (C that stays C): `git_index_add_bypath` of a
//! directory that is a submodule. Safe wrappers: `crate::index::foreign`.

use core::ffi::{c_char, c_int, c_void};

unsafe extern "C" {
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
