//! Port of `src/util/allocators/stdalloc.c`: the C library's `malloc`,
//! `realloc` and `free`, the default allocator.

use crate::util::cabi::Out;
use core::ffi::{c_char, c_int, c_void};

use crate::util::alloc::GitAllocator;

/// The standard allocator.
pub const ALLOCATOR: GitAllocator = GitAllocator {
    gmalloc: Some(stdalloc_malloc),
    grealloc: Some(stdalloc_realloc),
    gfree: Some(stdalloc_free),
};

/// `malloc` (safe: any size is allowed, and the file is not read).
extern "C" fn stdalloc_malloc(len: usize, _file: *const c_char, _line: c_int) -> *mut c_void {
    // SAFETY: `malloc` may be called with any size.
    unsafe { libc::malloc(len) }
}

/// `realloc`.
///
/// # Safety
///
/// `ptr` is NULL or a live allocation of this allocator (C's `malloc`),
/// which the call may free.
unsafe extern "C" fn stdalloc_realloc(
    ptr: *mut c_void,
    size: usize,
    _file: *const c_char,
    _line: c_int,
) -> *mut c_void {
    // SAFETY: `ptr` is NULL or a live `malloc` block (this fn's contract).
    unsafe { libc::realloc(ptr, size) }
}

/// `free`.
///
/// # Safety
///
/// `ptr` is NULL or a live allocation of this allocator, not used again.
unsafe extern "C" fn stdalloc_free(ptr: *mut c_void) {
    // SAFETY: `ptr` is NULL or a live `malloc` block, not used again (this
    // fn's contract).
    unsafe { libc::free(ptr) }
}

/// `git_stdalloc_init_allocator`: fill `allocator` with the standard one.
///
/// # Safety
///
/// `allocator` is valid for a write of a `git_allocator`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_stdalloc_init_allocator(
    mut allocator: Out<'_, GitAllocator>,
) -> c_int {
    allocator.write(ALLOCATOR);
    0
}
