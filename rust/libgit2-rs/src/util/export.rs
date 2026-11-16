//! Helpers for the `extern "C"` exports of every module (PORTING.md §4.5):
//! the panic barrier, status conversion, and the few raw-pointer
//! conversions that an export makes in one justified place.
//!
//! An export is a thin shim:
//!
//! ```ignore
//! #[unsafe(no_mangle)]
//! pub unsafe extern "C" fn git_foo_bar(out: *mut c_int, name: *const c_char) -> c_int {
//!     barrier(|| {
//!         git_assert_arg!(!out.is_null(), "out", -1);
//!         // SAFETY: the caller passes a NUL-terminated `name` (this export's contract).
//!         let name = unsafe { cstr(name) };
//!         status(foo::bar(name).map(|v| unsafe { out.write(v) }))
//!     })
//! }
//! ```

use core::ffi::{CStr, c_char, c_int, c_void};

use super::errors::{self, Result};

/// The value an export returns when a panic stopped it: what the C returns
/// for a generic failure of the same signature.
pub trait PanicValue {
    /// The failure value.
    fn panic_value() -> Self;
}

impl PanicValue for c_int {
    #[inline]
    fn panic_value() -> Self {
        -1
    }
}

impl PanicValue for () {
    #[inline]
    fn panic_value() -> Self {}
}

impl PanicValue for bool {
    #[inline]
    fn panic_value() -> Self {
        false
    }
}

impl PanicValue for usize {
    #[inline]
    fn panic_value() -> Self {
        0
    }
}

impl PanicValue for u64 {
    #[inline]
    fn panic_value() -> Self {
        0
    }
}

impl PanicValue for u32 {
    #[inline]
    fn panic_value() -> Self {
        0
    }
}

impl PanicValue for i64 {
    #[inline]
    fn panic_value() -> Self {
        -1
    }
}

impl PanicValue for isize {
    #[inline]
    fn panic_value() -> Self {
        -1
    }
}

impl<T> PanicValue for *mut T {
    #[inline]
    fn panic_value() -> Self {
        core::ptr::null_mut()
    }
}

impl<T> PanicValue for *const T {
    #[inline]
    fn panic_value() -> Self {
        core::ptr::null()
    }
}

/// Run an export's body so that a panic never crosses into C.
///
/// A panic is caught (ffibox's `callback::barrier`), the thread's error is
/// set to class `GIT_ERROR_INTERNAL`, and the export returns
/// [`R::panic_value()`](PanicValue::panic_value): `-1` for an `int`, NULL for
/// a pointer.
#[inline(always)]
pub fn barrier<R: PanicValue>(f: impl FnOnce() -> R) -> R {
    ffibox::callback::barrier(f, || {
        errors::set_panic_error();
        R::panic_value()
    })
}

/// A `Result` as C's status: `0` on success, the error's code otherwise.
#[inline]
pub fn status(r: Result<()>) -> c_int {
    match r {
        Ok(()) => 0,
        Err(e) => e.raw(),
    }
}

/// A `Result` carrying a non-negative `int` as C's return value.
#[inline]
pub fn status_int(r: Result<c_int>) -> c_int {
    match r {
        Ok(v) => v,
        Err(e) => e.raw(),
    }
}

/// The `len` bytes at `ptr` as a slice; empty when `len` is 0, whatever
/// `ptr` is (C passes NULL with 0).
///
/// # Safety
///
/// When `len > 0`, `ptr` must be valid for reads of `len` bytes for `'a`,
/// and nothing may write them during `'a`.
#[inline(always)]
pub unsafe fn bytes<'a>(ptr: *const c_char, len: usize) -> &'a [u8] {
    if len == 0 || ptr.is_null() {
        return &[];
    }
    // SAFETY: the caller guarantees `len` readable, unwritten bytes at the
    // non-null `ptr` for `'a`; `u8` has alignment 1.
    unsafe { core::slice::from_raw_parts(ptr.cast::<u8>(), len) }
}

/// The NUL-terminated string at `ptr`, or `None` for NULL.
///
/// # Safety
///
/// A non-null `ptr` must point to a NUL-terminated string that stays
/// readable and unwritten for `'a`.
#[inline(always)]
pub unsafe fn cstr<'a>(ptr: *const c_char) -> Option<&'a CStr> {
    if ptr.is_null() {
        None
    } else {
        // SAFETY: the caller guarantees a live, unwritten NUL-terminated
        // string at the non-null `ptr` for `'a`.
        Some(unsafe { CStr::from_ptr(ptr) })
    }
}

/// The bytes (without the NUL) of the string at `ptr`; empty for NULL.
///
/// # Safety
///
/// As [`cstr`].
#[inline(always)]
pub unsafe fn cstr_bytes<'a>(ptr: *const c_char) -> &'a [u8] {
    // SAFETY: forwarded to the caller.
    match unsafe { cstr(ptr) } {
        Some(s) => s.to_bytes(),
        None => &[],
    }
}

/// Write `value` through `out` if `out` is not NULL (C's optional
/// out-parameters).
///
/// # Safety
///
/// A non-null `out` must be valid for a write of `T` and properly aligned.
#[inline(always)]
pub unsafe fn write_opt<T>(out: *mut T, value: T) {
    if !out.is_null() {
        // SAFETY: the caller guarantees a valid, aligned, non-null `out`.
        unsafe { out.write(value) };
    }
}

/// `void *` to the payload type C hands to callbacks: a no-op that names the
/// conversion.
#[inline(always)]
pub fn payload<T>(p: *mut T) -> *mut c_void {
    p.cast()
}
