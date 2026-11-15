//! Two borrows formed from a raw pointer: one through `<*const T>::as_ref`, which ffimap's std
//! table counts as `raw_ptr.casted_to.std.borrow`, and one through `AtomicI32::from_ptr`, which
//! it does not.
use core::sync::atomic::AtomicI32;

/// A shared reference to the integer `p` addresses.
///
/// # Safety
///
/// `p` is null or addresses an `i32` live, and unwritten, for `'a`.
pub unsafe fn plain<'a>(p: *const i32) -> Option<&'a i32> {
    // SAFETY: the caller's.
    unsafe { p.as_ref() }
}

/// A shared reference to the integer `p` addresses, as an atomic.
///
/// # Safety
///
/// `p` addresses an aligned `i32` live for `'a`, which every concurrent access reaches
/// atomically.
pub unsafe fn atomic<'a>(p: *mut i32) -> &'a AtomicI32 {
    // SAFETY: the caller's.
    unsafe { AtomicI32::from_ptr(p) }
}
