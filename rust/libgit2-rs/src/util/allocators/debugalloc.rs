//! Port of `src/util/allocators/debugalloc.c`: an allocator that records
//! each allocation's size before it, so that `realloc` always moves the
//! block (and poisons the old one with `0xfd`), and that fails
//! zero-sized allocations. Installed when built with
//! `GIT_DEBUG_STRICT_ALLOC`.

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

unsafe extern "C" fn debugalloc_malloc(len: usize, _file: *const c_char, _line: c_int) -> *mut c_void {
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
    // SAFETY: `ptr` holds `total >= HDR` bytes; the header may be unaligned
    // for a `usize` in general, so it is written unaligned.
    unsafe {
        ptr.cast::<usize>().write_unaligned(len);
        ptr.add(HDR).cast()
    }
}

unsafe extern "C" fn debugalloc_realloc(ptr: *mut c_void, len: usize, _file: *const c_char, _line: c_int) -> *mut c_void {
    let ptr = ptr.cast::<u8>();
    if len == 0 && ptr.is_null() {
        return core::ptr::null_mut();
    }
    if len == 0 {
        // SAFETY: a non-null `ptr` comes from this allocator, `HDR` bytes
        // into its block (the allocator's contract).
        unsafe { libc::free(ptr.sub(HDR).cast()) };
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
        // SAFETY: `ptr` comes from this allocator: its block starts `HDR`
        // bytes before it with its length, and holds that many bytes after
        // it. `newptr` is a fresh block of `len + HDR` bytes.
        unsafe {
            let block = ptr.sub(HDR);
            let original_len = block.cast::<usize>().read_unaligned();
            core::ptr::copy_nonoverlapping(ptr, newptr.add(HDR), len.min(original_len));
            core::ptr::write_bytes(block, 0xfd, original_len + HDR);
            libc::free(block.cast());
        }
    }
    // SAFETY: `newptr` holds `len + HDR` bytes.
    unsafe {
        newptr.cast::<usize>().write_unaligned(len);
        newptr.add(HDR).cast()
    }
}

unsafe extern "C" fn debugalloc_free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: a non-null `ptr` comes from this allocator, `HDR` bytes into
    // its block.
    unsafe { libc::free(ptr.cast::<u8>().sub(HDR).cast()) }
}

/// `git_debugalloc_init_allocator`: fill `allocator` with the debug one.
///
/// # Safety
///
/// `allocator` is valid for a write of a `git_allocator`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_debugalloc_init_allocator(allocator: *mut GitAllocator) -> c_int {
    // SAFETY: the caller passes a writable `git_allocator`.
    unsafe { allocator.write(ALLOCATOR) };
    0
}

#[cfg(test)]
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
