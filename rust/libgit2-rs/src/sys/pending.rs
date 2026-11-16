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

// PENDING(l-pack): `src/libgit2/pack.c`. `git_delta_read_header_fromstream`
// reads a delta's header through a pack stream (`git_packfile_stream *`,
// opaque here).
unsafe extern "C" {
    /// `git_packfile_stream_read`. PENDING(l-pack)
    pub fn git_packfile_stream_read(obj: *mut c_void, buffer: *mut c_void, len: usize) -> isize;
}
