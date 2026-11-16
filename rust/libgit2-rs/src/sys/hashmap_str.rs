//! `git_hashset_str` (`src/util/hashmap_str.h`, header-only C that stays
//! C): the C-owned sets that `git_futils_mkdir_relative`'s callers
//! (`checkout.c`) pass are read and extended with the C's own inline
//! functions, through two thin shims in `src/util/rust_shims.c`, so that
//! the set's layout and hashing stay exactly the C's.

use core::ffi::{c_char, c_int, c_void};

unsafe extern "C" {
    /// `git_hashset_str_contains`, out of line.
    pub fn git_hashset_str__contains(set: *mut c_void, key: *const c_char) -> c_int;
    /// `git_hashset_str_add`, out of line.
    pub fn git_hashset_str__add(set: *mut c_void, key: *const c_char) -> c_int;
}
