//! Port of `src/util/allocators/failalloc.c`: the allocator that fails
//! every allocation, installed until the library is initialized.

use core::ffi::{c_char, c_int, c_void};

use crate::util::alloc::GitAllocator;

/// The failing allocator.
pub const ALLOCATOR: GitAllocator = GitAllocator {
    gmalloc: Some(git_failalloc_malloc),
    grealloc: Some(git_failalloc_realloc),
    gfree: Some(git_failalloc_free),
};

/// Always NULL.
///
/// # Safety
///
/// None: it reads none of its arguments.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_failalloc_malloc(_len: usize, _file: *const c_char, _line: c_int) -> *mut c_void {
    core::ptr::null_mut()
}

/// Always NULL.
///
/// # Safety
///
/// None: it reads none of its arguments.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_failalloc_realloc(
    _ptr: *mut c_void,
    _size: usize,
    _file: *const c_char,
    _line: c_int,
) -> *mut c_void {
    core::ptr::null_mut()
}

/// Does nothing.
///
/// # Safety
///
/// None: it reads none of its arguments.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_failalloc_free(_ptr: *mut c_void) {}
