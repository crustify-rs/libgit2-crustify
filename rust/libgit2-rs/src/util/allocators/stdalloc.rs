//! Port of `src/util/allocators/stdalloc.c`: the C library's `malloc`,
//! `realloc` and `free`, the default allocator.

use core::ffi::{c_char, c_int, c_void};

use crate::util::alloc::GitAllocator;

/// The standard allocator.
pub const ALLOCATOR: GitAllocator = GitAllocator {
    gmalloc: Some(stdalloc_malloc),
    grealloc: Some(stdalloc_realloc),
    gfree: Some(stdalloc_free),
};

unsafe extern "C" fn stdalloc_malloc(len: usize, _file: *const c_char, _line: c_int) -> *mut c_void {
    // SAFETY: `malloc` may be called with any size.
    unsafe { libc::malloc(len) }
}

unsafe extern "C" fn stdalloc_realloc(ptr: *mut c_void, size: usize, _file: *const c_char, _line: c_int) -> *mut c_void {
    // SAFETY: the allocator's contract: `ptr` is NULL or from this allocator.
    unsafe { libc::realloc(ptr, size) }
}

unsafe extern "C" fn stdalloc_free(ptr: *mut c_void) {
    // SAFETY: the allocator's contract: `ptr` is NULL or from this allocator.
    unsafe { libc::free(ptr) }
}

/// `git_stdalloc_init_allocator`: fill `allocator` with the standard one.
///
/// # Safety
///
/// `allocator` is valid for a write of a `git_allocator`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_stdalloc_init_allocator(allocator: *mut GitAllocator) -> c_int {
    // SAFETY: the caller passes a writable `git_allocator`.
    unsafe { allocator.write(ALLOCATOR) };
    0
}
