//! Port of `src/util/allocators/debugalloc.c`: an allocator that records
//! each allocation's size before it, so that `realloc` always moves the
//! block (and poisons the old one with `0xfd`), and that fails
//! zero-sized allocations. Installed when built with
//! `GIT_DEBUG_STRICT_ALLOC`.

use crate::util::cabi::Out;
use core::ffi::{c_char, c_int, c_void};
use core::mem::size_of;

use crate::util::alloc::GitAllocator;

/// The debug allocator.
pub const ALLOCATOR: GitAllocator = GitAllocator {
    gmalloc: Some(debugalloc_malloc),
    grealloc: Some(debugalloc_realloc),
    gfree: Some(debugalloc_free),
};

const HDR: usize = size_of::<usize>();

unsafe extern "C" fn debugalloc_malloc(
    len: usize,
    _file: *const c_char,
    _line: c_int,
) -> *mut c_void {
    let Some(total) = len.checked_add(HDR) else {
        return core::ptr::null_mut();
    };
    if len == 0 {
        return core::ptr::null_mut();
    }
    // SAFETY: `malloc` may be called with any size.
    let ptr = unsafe { libc::malloc(total) }.cast::<u8>();
    if ptr.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: `ptr` holds `total >= HDR` bytes; the header is written
    // unaligned, as `malloc`'s alignment need not cover a `usize` in general.
    unsafe { ptr.cast::<usize>().write_unaligned(len) };
    ptr.wrapping_add(HDR).cast()
}

unsafe extern "C" fn debugalloc_realloc(
    ptr: *mut c_void,
    len: usize,
    _file: *const c_char,
    _line: c_int,
) -> *mut c_void {
    let ptr = ptr.cast::<u8>();
    if len == 0 && ptr.is_null() {
        return core::ptr::null_mut();
    }
    if len == 0 {
        // SAFETY: a non-null `ptr` comes from this allocator, `HDR` bytes
        // into its block (the allocator's contract).
        unsafe { libc::free(ptr.wrapping_sub(HDR).cast()) };
        return core::ptr::null_mut();
    }
    let Some(total) = len.checked_add(HDR) else {
        return core::ptr::null_mut();
    };
    // SAFETY: `malloc` may be called with any size.
    let newptr = unsafe { libc::malloc(total) }.cast::<u8>();
    if newptr.is_null() {
        return core::ptr::null_mut();
    }
    if !ptr.is_null() {
        // `ptr` comes from this allocator (its contract): its block starts
        // `HDR` bytes before it, with its length, and holds that many bytes
        // after it. `newptr` is a fresh block of `len + HDR` bytes.
        let block = ptr.wrapping_sub(HDR);
        // SAFETY: the block's header.
        let original_len = unsafe { block.cast::<usize>().read_unaligned() };
        // SAFETY: `min(len, original_len)` bytes are readable at `ptr` and
        // writable after the new block's header; the blocks are distinct.
        unsafe {
            core::ptr::copy_nonoverlapping(ptr, newptr.wrapping_add(HDR), len.min(original_len))
        };
        // SAFETY: the old block's `original_len + HDR` bytes, poisoned.
        unsafe { core::ptr::write_bytes(block, 0xfd, original_len + HDR) };
        // SAFETY: the old block, freed once.
        unsafe { libc::free(block.cast()) };
    }
    // SAFETY: `newptr` holds `len + HDR` bytes; the header is written
    // unaligned, as in `debugalloc_malloc`.
    unsafe { newptr.cast::<usize>().write_unaligned(len) };
    newptr.wrapping_add(HDR).cast()
}

unsafe extern "C" fn debugalloc_free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: a non-null `ptr` comes from this allocator, `HDR` bytes into
    // its block.
    unsafe { libc::free(ptr.cast::<u8>().wrapping_sub(HDR).cast()) }
}

/// `git_debugalloc_init_allocator`: fill `allocator` with the debug one.
///
/// # Safety
///
/// `allocator` is valid for a write of a `git_allocator`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_debugalloc_init_allocator(
    mut allocator: Out<'_, GitAllocator>,
) -> c_int {
    allocator.write(ALLOCATOR);
    0
}

#[cfg(test)]
#[allow(clippy::multiple_unsafe_ops_per_block)] // each block's SAFETY covers its sequence
mod tests {
    use super::*;

    #[test]
    fn debugalloc_moves_and_keeps_contents() {
        // SAFETY: the allocator's own functions, used as their contract says.
        unsafe {
            assert!(debugalloc_malloc(0, core::ptr::null(), 0).is_null());
            let p = debugalloc_malloc(4, core::ptr::null(), 0).cast::<u8>();
            p.copy_from(b"abcd".as_ptr(), 4);
            let q = debugalloc_realloc(p.cast(), 8, core::ptr::null(), 0).cast::<u8>();
            assert_eq!(core::slice::from_raw_parts(q, 4), b"abcd");
            let r = debugalloc_realloc(q.cast(), 2, core::ptr::null(), 0).cast::<u8>();
            assert_eq!(core::slice::from_raw_parts(r, 2), b"ab");
            assert!(debugalloc_realloc(r.cast(), 0, core::ptr::null(), 0).is_null());
            debugalloc_free(core::ptr::null_mut());
        }
    }
}
