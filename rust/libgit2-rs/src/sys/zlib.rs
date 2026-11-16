//! zlib (a system library the C links), as `git_filebuf` uses it: its
//! `z_stream` is embedded by value in `git_filebuf`, which C embeds in turn,
//! so the stream and the calls stay zlib's own.

use core::ffi::{c_char, c_int, c_uint, c_ulong, c_void};

/// `z_stream`, mirrored (kind 3: embedded in `git_filebuf`).
#[repr(C)]
#[derive(Debug)]
pub struct ZStream {
    /// `next_in`.
    pub next_in: *const u8,
    /// `avail_in`.
    pub avail_in: c_uint,
    /// `total_in`.
    pub total_in: c_ulong,
    /// `next_out`.
    pub next_out: *mut u8,
    /// `avail_out`.
    pub avail_out: c_uint,
    /// `total_out`.
    pub total_out: c_ulong,
    /// `msg`.
    pub msg: *const c_char,
    /// `state`.
    pub state: *mut c_void,
    /// `zalloc` (NULL: zlib's default).
    pub zalloc: *mut c_void,
    /// `zfree`.
    pub zfree: *mut c_void,
    /// `opaque`.
    pub opaque: *mut c_void,
    /// `data_type`.
    pub data_type: c_int,
    /// `adler`.
    pub adler: c_ulong,
    /// `reserved`.
    pub reserved: c_ulong,
}

impl ZStream {
    /// A zeroed stream (`{0}`), what `deflateInit` expects.
    pub const fn zeroed() -> Self {
        ZStream {
            next_in: core::ptr::null(),
            avail_in: 0,
            total_in: 0,
            next_out: core::ptr::null_mut(),
            avail_out: 0,
            total_out: 0,
            msg: core::ptr::null(),
            state: core::ptr::null_mut(),
            zalloc: core::ptr::null_mut(),
            zfree: core::ptr::null_mut(),
            opaque: core::ptr::null_mut(),
            data_type: 0,
            adler: 0,
            reserved: 0,
        }
    }
}

/// `Z_OK`.
pub const Z_OK: c_int = 0;
/// `Z_STREAM_ERROR`.
pub const Z_STREAM_ERROR: c_int = -2;
/// `Z_NO_FLUSH`.
pub const Z_NO_FLUSH: c_int = 0;
/// `Z_FINISH`.
pub const Z_FINISH: c_int = 4;

#[link(name = "z")]
unsafe extern "C" {
    fn zlibVersion() -> *const c_char;
    fn deflateInit_(strm: *mut ZStream, level: c_int, version: *const c_char, stream_size: c_int) -> c_int;
    /// `deflate`.
    pub fn deflate(strm: *mut ZStream, flush: c_int) -> c_int;
    /// `deflateEnd`.
    pub fn deflateEnd(strm: *mut ZStream) -> c_int;
}

/// `deflateInit(strm, level)`.
///
/// # Safety
///
/// `strm` is a valid, zeroed (or ended) stream that does not move until
/// `deflateEnd`.
pub unsafe fn deflate_init(strm: *mut ZStream, level: c_int) -> c_int {
    // SAFETY: `zlibVersion` has no preconditions; the version it returns
    // matches the library that is called, and the stream is as zlib's
    // `z_stream` (the layout test).
    unsafe {
        deflateInit_(
            strm,
            level,
            zlibVersion(),
            core::mem::size_of::<ZStream>() as c_int,
        )
    }
}
