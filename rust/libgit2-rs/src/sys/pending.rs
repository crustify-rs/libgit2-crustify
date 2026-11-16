//! Functions of modules that another work package is porting concurrently,
//! called through their C symbols until both have landed (PORTING.md §3).
//! Each entry carries a `// PENDING(<wp>)` comment.

use core::ffi::c_void;

// PENDING(pool): `src/util/pool.c` (in PORTING.md §1's list; not yet
// ported). `git_futils_mkdir_relative`'s path cache allocates from the
// caller's (C-owned) pool.
unsafe extern "C" {
    /// `git_pool_malloc`. PENDING(pool)
    pub fn git_pool_malloc(pool: *mut c_void, items: usize) -> *mut c_void;
}

// PENDING(l-config, l-odb, l-pack, l-refs, l-index, l-obj, l-commit): what
// the repository (WP l-repo) calls of them.
pub mod l_repo;
