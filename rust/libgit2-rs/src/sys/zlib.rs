//! The system zlib (`libz`), which C that stays uses directly (indexer's
//! `crc32`, filebuf's `deflate`) and which [`crate::util::zstream`] uses to
//! deflate, so that compressed bytes are zlib's own (zlib-rs, which
//! inflates, deflates to different bytes).
//!
//! The stream type is zlib-rs's `z_stream`, which has zlib's layout (the
//! layout test of `util/zstream.rs` checks it against `<zlib.h>`).
//!
//! These wrappers stay `unsafe` and take raw pointers: a `z_stream` carries
//! the pointers zlib reads and writes (`next_in`, `next_out`), and
//! `deflateInit_` stores the stream's own address in its state
//! (`s->strm`), through which every later call reads and writes the
//! stream. So the stream must be reached only through pointers derived
//! from the one given to `deflate_init` (no `&mut z_stream` in between),
//! and must not move until `deflate_end`. [`crate::util::zstream`]
//! establishes this.

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
/// `strm` is a valid stream whose `zalloc`, `zfree` and `opaque` are NULL
/// or a valid allocator (zlib's contract); until `deflate_end` it is not
/// moved, and reached only through `strm` and pointers derived from it
/// (zlib keeps `strm` and writes through it in later calls).
pub unsafe fn deflate_init(strm: *mut z_stream, level: c_int) -> c_int {
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
/// `strm` is the pointer given to [`deflate_init`] (or derived from it, as
/// that function requires; zlib rejects a stream it did not initialize),
/// and `next_in` / `next_out` are readable / writable for `avail_in` /
/// `avail_out` bytes.
pub unsafe fn deflate(strm: *mut z_stream, flush: c_int) -> c_int {
    // SAFETY: `strm` is the initialized stream, reached as `deflate_init`
    // requires, with its buffers valid for their lengths (this function's
    // contract): zlib's contract for `deflate`.
    unsafe { sys_deflate(strm, flush) }
}

/// `deflateEnd(strm)`: frees the state; a stream that has none (zeroed,
/// or already ended) gets `Z_STREAM_ERROR`.
///
/// # Safety
///
/// `strm` is a valid stream whose `state` is NULL or zlib's deflate state
/// for it, reached as [`deflate_init`] requires.
pub unsafe fn deflate_end(strm: *mut z_stream) -> c_int {
    // SAFETY: `strm` is a valid stream whose state is NULL or its own
    // deflate state (this function's contract): zlib checks the rest.
    unsafe { sys_deflateEnd(strm) }
}

/// `deflateReset(strm)`.
///
/// # Safety
///
/// As [`deflate_end`].
pub unsafe fn deflate_reset(strm: *mut z_stream) -> c_int {
    // SAFETY: as in `deflate_end` (this function's contract).
    unsafe { sys_deflateReset(strm) }
}

// The system zlib's inflate, for streams whose first output request is
// small (header reads): zlib's setup and 9/6-bit root tables cost less
// there than zlib-rs's (rust/bench/RESULTS.md, review l-odb-r1 L-ODB-12).
unsafe extern "C" {
    #[link_name = "inflateInit_"]
    fn sys_inflateInit_(strm: *mut z_stream, version: *const c_char, stream_size: c_int) -> c_int;
    #[link_name = "inflate"]
    fn sys_inflate(strm: *mut z_stream, flush: c_int) -> c_int;
    #[link_name = "inflateEnd"]
    fn sys_inflateEnd(strm: *mut z_stream) -> c_int;
    #[link_name = "inflateReset"]
    fn sys_inflateReset(strm: *mut z_stream) -> c_int;
}

/// `inflateInit(strm)` with the system zlib.
///
/// # Safety
///
/// As [`deflate_init`]: `strm` is a valid stream with no state, which does
/// not move and is reached only through pointers derived from this one
/// until [`inflate_end`] (zlib keeps a pointer back to it).
pub unsafe fn inflate_init(strm: *mut z_stream) -> c_int {
    // SAFETY: zlib's static version string.
    let version = unsafe { sys_zlibVersion() };
    // SAFETY: `strm` is a valid stream with no state, not moved and reached
    // only through pointers derived from this one until `inflate_end`
    // (this function's contract); the version and size are this zlib's
    // and its `z_stream`'s (layout-tested).
    unsafe { sys_inflateInit_(strm, version, core::mem::size_of::<z_stream>() as c_int) }
}

/// `inflate(strm, flush)` with the system zlib.
///
/// # Safety
///
/// `strm` went through [`inflate_init`] (as its contract requires), and
/// `next_in` / `next_out` are readable / writable for `avail_in` /
/// `avail_out` bytes.
pub unsafe fn inflate(strm: *mut z_stream, flush: c_int) -> c_int {
    // SAFETY: `strm` went through `inflate_init`, with its buffers valid for
    // their lengths (this function's contract): zlib's for `inflate`.
    unsafe { sys_inflate(strm, flush) }
}

/// `inflateEnd(strm)` with the system zlib.
///
/// # Safety
///
/// `strm` is a valid stream whose state is NULL or the system zlib's
/// inflate state for it.
pub unsafe fn inflate_end(strm: *mut z_stream) -> c_int {
    // SAFETY: `strm` is a valid stream whose state is NULL or its own
    // inflate state (this function's contract).
    unsafe { sys_inflateEnd(strm) }
}

/// `inflateReset(strm)` with the system zlib.
///
/// # Safety
///
/// As [`inflate_end`].
pub unsafe fn inflate_reset(strm: *mut z_stream) -> c_int {
    // SAFETY: as in `inflate_end` (this function's contract).
    unsafe { sys_inflateReset(strm) }
}
