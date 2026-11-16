//! The system zlib (`libz`), which C that stays uses directly (indexer's
//! `crc32`, filebuf's `deflate`) and which [`crate::util::zstream`] uses to
//! deflate, so that compressed bytes are zlib's own (zlib-rs, which
//! inflates, deflates to different bytes).
//!
//! The stream type is zlib-rs's `z_stream`, which has zlib's layout (the
//! layout test of `util/zstream.rs` checks it against `<zlib.h>`).
//!
//! These wrappers stay `unsafe`: a `z_stream` carries the raw pointers zlib
//! reads and writes (`next_in`, `next_out`), so no wrapper can check the
//! contract; [`crate::util::zstream`] establishes it at each call.

use core::ffi::{c_char, c_int};

pub use libz_rs_sys::z_stream;

unsafe extern "C" {
    #[link_name = "zlibVersion"]
    fn sys_zlibVersion() -> *const c_char;
    #[link_name = "deflateInit_"]
    fn sys_deflateInit_(
        strm: *mut z_stream,
        level: c_int,
        version: *const c_char,
        stream_size: c_int,
    ) -> c_int;
    #[link_name = "deflate"]
    fn sys_deflate(strm: *mut z_stream, flush: c_int) -> c_int;
    #[link_name = "deflateEnd"]
    fn sys_deflateEnd(strm: *mut z_stream) -> c_int;
    #[link_name = "deflateReset"]
    fn sys_deflateReset(strm: *mut z_stream) -> c_int;
}

/// `deflateInit(strm, level)`.
///
/// # Safety
///
/// `strm`'s `zalloc`, `zfree` and `opaque` are NULL or a valid allocator
/// (zlib's contract); `strm` is not moved while zlib's state refers to it
/// (zlib then reports `Z_STREAM_ERROR`).
pub unsafe fn deflate_init(strm: &mut z_stream, level: c_int) -> c_int {
    // SAFETY: `zlibVersion` has no precondition and returns a static
    // string.
    let version = unsafe { sys_zlibVersion() };
    // SAFETY: `deflateInit_` gets this library's version and `z_stream`
    // size (checked against `<zlib.h>` by the layout test), and the
    // caller's contract on `strm`.
    unsafe {
        sys_deflateInit_(
            strm,
            level,
            version,
            core::mem::size_of::<z_stream>() as c_int,
        )
    }
}

/// `deflate(strm, flush)`.
///
/// # Safety
///
/// `strm` went through [`deflate_init`] (or zlib rejects it), and
/// `next_in` / `next_out` are readable / writable for `avail_in` /
/// `avail_out` bytes.
pub unsafe fn deflate(strm: &mut z_stream, flush: c_int) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { sys_deflate(strm, flush) }
}

/// `deflateEnd(strm)`: frees the state; a stream that has none (zeroed,
/// or already ended) gets `Z_STREAM_ERROR`.
///
/// # Safety
///
/// `strm`'s `state` is NULL or zlib's deflate state for this stream.
pub unsafe fn deflate_end(strm: &mut z_stream) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { sys_deflateEnd(strm) }
}

/// `deflateReset(strm)`.
///
/// # Safety
///
/// As [`deflate_end`].
pub unsafe fn deflate_reset(strm: &mut z_stream) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { sys_deflateReset(strm) }
}
