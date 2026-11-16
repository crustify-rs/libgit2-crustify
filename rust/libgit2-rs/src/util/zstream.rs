//! Port of `src/util/zstream.c`: libgit2's zlib stream (`git_zstream`).
//!
//! [`GitZstream`] is mirrored (kind 3): C that stays embeds it by value
//! (`git_packfile_stream`, `git_packbuilder`, odb_loose's stream) and reads
//! its `in_len` (pack.c). It embeds a zlib `z_stream`, which only this
//! module touches.
//!
//! Inflating uses zlib-rs (`libz-rs-sys`), a Rust zlib with zlib's API,
//! stream layout, return codes and messages, and faster than the system
//! zlib (see `rust/bench/RESULTS.md`). Deflating uses the system zlib
//! ([`crate::sys::zlib`]): zlib-rs compresses to other bytes, and the
//! deflated bytes are visible (binary patches, packs, loose objects). A
//! stream is one or the other for its whole life, so the two never see
//! each other's state.
//!
//! The safe API is [`ZStream`] (a boxed stream that borrows its input),
//! [`deflate_buf`] and [`inflate_buf`].

use core::ffi::{CStr, c_char, c_int, c_uint};
use core::marker::PhantomData;
use core::mem::MaybeUninit;

use libz_rs_sys::{
    Z_BUF_ERROR, Z_DEFAULT_COMPRESSION, Z_FINISH, Z_MEM_ERROR, Z_NO_FLUSH, Z_OK, Z_STREAM_END,
    z_stream,
};

use super::errors::{self, ErrorClass, Result};
use super::str::GitStr;
use crate::sys::zlib;
use crate::{git_assert, git_error_set};

/// `ZSTREAM_BUFFER_SIZE`.
pub const ZSTREAM_BUFFER_SIZE: usize = 1024 * 1024;
/// `ZSTREAM_BUFFER_MIN_EXTRA`.
pub const ZSTREAM_BUFFER_MIN_EXTRA: usize = 8;

/// `git_zstream_t`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZstreamKind {
    /// `GIT_ZSTREAM_INFLATE`
    Inflate = 0,
    /// `GIT_ZSTREAM_DEFLATE`
    Deflate = 1,
}

/// `git_zstream`, mirrored.
///
/// The `type` field is a C `int` here: C stores what it is given, and the
/// C's test is `type == GIT_ZSTREAM_INFLATE` (anything else deflates).
#[repr(C)]
pub struct GitZstream {
    z: z_stream,
    kind: c_int,
    in_: *const c_char,
    in_len: usize,
    flush: c_int,
    zerr: c_int,
}

impl Default for GitZstream {
    fn default() -> GitZstream {
        GitZstream::new()
    }
}

impl GitZstream {
    /// `GIT_ZSTREAM_INIT`: all zero.
    pub fn new() -> GitZstream {
        // zlib-rs's `Default` fills in its Rust allocator; C's
        // `GIT_ZSTREAM_INIT` leaves NULL (each library then uses its own).
        let z = z_stream {
            zalloc: None,
            zfree: None,
            ..z_stream::default()
        };
        GitZstream {
            z,
            kind: ZstreamKind::Inflate as c_int,
            in_: core::ptr::null(),
            in_len: 0,
            flush: 0,
            zerr: 0,
        }
    }

    fn inflates(&self) -> bool {
        self.kind == ZstreamKind::Inflate as c_int
    }

    /// `zstream_seterr`.
    fn seterr(&self) -> Result<()> {
        match self.zerr {
            // Z_BUF_ERROR is not fatal: the caller retries with more room.
            Z_OK | Z_STREAM_END | Z_BUF_ERROR => Ok(()),
            Z_MEM_ERROR => Err(errors::oom()),
            _ => {
                if self.z.msg.is_null() {
                    Err(git_error_set!(
                        ErrorClass::ZLIB,
                        "unknown compression error"
                    ))
                } else {
                    // SAFETY: zlib (and zlib-rs) set `msg` to NULL or to a
                    // static NUL-terminated message.
                    let m = unsafe { CStr::from_ptr(self.z.msg) };
                    Err(errors::set_str(ErrorClass::ZLIB, m.to_bytes()))
                }
            }
        }
    }

    /// `git_zstream_init`. `kind` is C's `git_zstream_t` value.
    pub fn init(&mut self, kind: c_int) -> Result<()> {
        self.kind = kind;
        self.zerr = if self.inflates() {
            // SAFETY: `zlibVersion` returns zlib-rs's static version; the
            // size is that of the stream; the allocator fields are NULL
            // (zlib-rs's default) or what C put there, as zlib's contract.
            unsafe {
                libz_rs_sys::inflateInit_(
                    &raw mut self.z,
                    libz_rs_sys::zlibVersion(),
                    core::mem::size_of::<z_stream>() as c_int,
                )
            }
        } else {
            // SAFETY: as above, for the system zlib.
            unsafe { zlib::deflate_init(&mut self.z, Z_DEFAULT_COMPRESSION) }
        };
        self.seterr()
    }

    /// `git_zstream_free`. Ending a stream that has no state (never
    /// initialized, or freed) does nothing.
    pub fn free(&mut self) {
        if self.inflates() {
            // SAFETY: the state is NULL or this stream's inflate state:
            // only `init` sets it, with the library `kind` picks.
            unsafe { libz_rs_sys::inflateEnd(&raw mut self.z) };
        } else {
            // SAFETY: as above, for deflate.
            unsafe { zlib::deflate_end(&mut self.z) };
        }
    }

    /// `git_zstream_reset`.
    pub fn reset(&mut self) {
        if self.inflates() {
            // SAFETY: as in `free`.
            unsafe { libz_rs_sys::inflateReset(&raw mut self.z) };
        } else {
            // SAFETY: as in `free`.
            unsafe { zlib::deflate_reset(&mut self.z) };
        }
        self.in_ = core::ptr::null();
        self.in_len = 0;
        self.zerr = Z_STREAM_END;
    }

    /// `git_zstream_set_input`: the input is read by the next
    /// [`get_output`](Self::get_output) calls, whose callers vouch for it.
    pub fn set_input_raw(&mut self, input: *const c_char, len: usize) {
        self.in_ = input;
        self.in_len = len;
        self.zerr = Z_OK;
    }

    /// The input not consumed yet (`in_len`).
    pub fn in_len(&self) -> usize {
        self.in_len
    }

    /// `git_zstream_done`.
    pub fn done(&self) -> bool {
        self.in_len == 0 && self.zerr == Z_STREAM_END
    }

    /// `git_zstream_eos`.
    pub fn eos(&self) -> bool {
        self.zerr == Z_STREAM_END
    }

    /// `git_zstream_suggest_output_len`.
    pub fn suggest_output_len(&self) -> usize {
        if self.in_len > ZSTREAM_BUFFER_SIZE {
            ZSTREAM_BUFFER_SIZE
        } else if self.in_len > ZSTREAM_BUFFER_MIN_EXTRA {
            self.in_len
        } else {
            ZSTREAM_BUFFER_MIN_EXTRA
        }
    }

    /// `git_zstream_get_output_chunk`: as much output as the input gives,
    /// into `out` (`*out_len` bytes), at most `UINT_MAX` of input at a
    /// time; `*out_len` becomes the bytes written.
    ///
    /// # Safety
    ///
    /// The input given to [`set_input_raw`](Self::set_input_raw) is still
    /// readable for `in_len` bytes and not written meanwhile; `out` is
    /// writable for `*out_len` bytes and overlaps neither the input nor
    /// `self`.
    pub unsafe fn get_output_chunk(&mut self, out: *mut u8, out_len: &mut usize) -> Result<()> {
        self.z.next_in = self.in_.cast::<u8>();
        if self.in_len > c_uint::MAX as usize {
            self.z.avail_in = c_uint::MAX;
            self.flush = Z_NO_FLUSH;
        } else {
            self.z.avail_in = self.in_len as c_uint;
            self.flush = Z_FINISH;
        }
        let in_queued = self.z.avail_in as usize;

        self.z.next_out = out;
        self.z.avail_out = *out_len as c_uint;
        if self.z.avail_out as usize != *out_len {
            self.z.avail_out = c_uint::MAX;
        }
        let out_queued = self.z.avail_out as usize;

        self.zerr = if self.inflates() {
            // SAFETY: the stream's state is the inflate state `init` made
            // (or NULL, which zlib-rs rejects); `next_in` / `next_out` are
            // readable / writable for `avail_in` / `avail_out` bytes, which
            // are at most `in_len` / `*out_len` (the caller's contract).
            unsafe { libz_rs_sys::inflate(&raw mut self.z, self.flush) }
        } else {
            // SAFETY: as above, for the system zlib's deflate state.
            unsafe { zlib::deflate(&mut self.z, self.flush) }
        };
        self.seterr()?;

        let in_used = in_queued - self.z.avail_in as usize;
        self.in_len -= in_used;
        self.in_ = self.in_.wrapping_add(in_used);
        *out_len = out_queued - self.z.avail_out as usize;
        Ok(())
    }

    /// `git_zstream_get_output`: all the output of the input, into `out`
    /// (`*out_len` bytes); `*out_len` becomes the bytes written.
    ///
    /// # Safety
    ///
    /// As [`get_output_chunk`](Self::get_output_chunk).
    pub unsafe fn get_output(&mut self, mut out: *mut u8, out_len: &mut usize) -> Result<()> {
        let mut out_remain = *out_len;

        if self.in_len != 0 && self.zerr == Z_STREAM_END {
            return Err(git_error_set!(
                ErrorClass::ZLIB,
                "zlib input had trailing garbage"
            ));
        }

        while out_remain > 0 && self.zerr != Z_STREAM_END {
            let mut out_written = out_remain;
            // SAFETY: `out` is writable for `out_remain` bytes (the
            // caller's contract, advanced by what was written).
            unsafe { self.get_output_chunk(out, &mut out_written) }?;

            // With room left, a Z_BUF_ERROR that wrote nothing is input
            // that ended mid-stream: more will not come within this loop.
            if out_written == 0 && self.zerr == Z_BUF_ERROR {
                return Err(git_error_set!(
                    ErrorClass::ZLIB,
                    "premature end of zlib stream"
                ));
            }

            out_remain -= out_written;
            out = out.wrapping_add(out_written);
        }

        // Either the input is finished or the data was not flushed.
        git_assert!(
            self.in_len > 0 || self.flush == Z_FINISH,
            "zstream->in_len > 0 || zstream->flush == Z_FINISH"
        );

        *out_len -= out_remain;
        Ok(())
    }
}

/// `zstream_buf`: append the (de/in)flated `input` to `out`.
fn zstream_buf(out: &mut GitStr, input: &[u8], kind: ZstreamKind) -> Result<()> {
    /// Frees the stream however the function returns.
    struct Free<'a>(&'a mut GitZstream);
    impl Drop for Free<'_> {
        fn drop(&mut self) {
            self.0.free();
        }
    }

    // On the stack, as C's: it does not move until it is freed.
    let mut stream = GitZstream::new();
    // C returns without freeing when the init fails (zlib then holds no
    // state).
    stream.init(kind as c_int)?;
    let zs = Free(&mut stream);
    zs.0.set_input_raw(input.as_ptr().cast::<c_char>(), input.len());
    while !zs.0.done() {
        let step = zs.0.suggest_output_len();
        out.grow_by(step)?;
        let spare = out.spare_capacity_mut();
        let mut written = spare.len();
        // SAFETY: the input is `input`, borrowed for the call; `spare` is
        // the string's writable spare capacity, distinct from both.
        unsafe {
            zs.0.get_output(spare.as_mut_ptr().cast::<u8>(), &mut written)
        }?;
        let len = out.len() + written;
        // SAFETY: zlib initialized the `written` bytes after the contents,
        // within the allocation (`len <= asize`).
        unsafe { out.set_len(len) };
    }
    drop(zs);
    // NUL-terminate. C does so only if there is room: output that filled
    // the buffer exactly was left without its NUL (rust/DEFECTS.md).
    if out.len() == out.capacity() && out.capacity() > 0 {
        out.grow(out.len() + 1)?;
    } else if let Some(nul) = out.spare_capacity_mut().first_mut() {
        nul.write(0);
    }
    Ok(())
}

/// `git_zstream_deflatebuf`: append the deflated `input` to `out`.
pub fn deflate_buf(out: &mut GitStr, input: &[u8]) -> Result<()> {
    zstream_buf(out, input, ZstreamKind::Deflate)
}

/// `git_zstream_inflatebuf`: append the inflated `input` to `out`.
pub fn inflate_buf(out: &mut GitStr, input: &[u8]) -> Result<()> {
    zstream_buf(out, input, ZstreamKind::Inflate)
}

// ---------------------------------------------------------------------------
// The safe API
// ---------------------------------------------------------------------------

/// A zlib stream that borrows its input for `'a`: [`GitZstream`],
/// initialized, on the heap (zlib's deflate state points back to its
/// stream), freed on drop.
pub struct ZStream<'a> {
    z: Box<GitZstream>,
    _input: PhantomData<&'a [u8]>,
}

// SAFETY: the stream owns its zlib state (zlib-rs's or the system zlib's),
// which has no tie to a thread; the input is a shared borrow of bytes,
// readable from any thread.
unsafe impl Send for ZStream<'_> {}

impl core::fmt::Debug for ZStream<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ZStream")
            .field("kind", &self.z.kind)
            .field("in_len", &self.z.in_len)
            .field("zerr", &self.z.zerr)
            .finish()
    }
}

impl<'a> ZStream<'a> {
    /// A stream that inflates or deflates (`git_zstream_init`).
    pub fn new(kind: ZstreamKind) -> Result<ZStream<'a>> {
        let mut z = Box::new(GitZstream::new());
        let r = z.init(kind as c_int);
        let zs = ZStream {
            z,
            _input: PhantomData,
        };
        r.map(|()| zs)
    }

    /// Give the input (`git_zstream_set_input`).
    pub fn set_input(&mut self, input: &'a [u8]) {
        self.z
            .set_input_raw(input.as_ptr().cast::<c_char>(), input.len());
    }

    /// `git_zstream_get_output_chunk`: as much output as the input gives
    /// at once, into `out`; the number of bytes written.
    pub fn get_output_chunk(&mut self, out: &mut [u8]) -> Result<usize> {
        let mut n = out.len();
        // SAFETY: the input is borrowed for `'a`, which outlives `self`;
        // `out` is writable for its length and is distinct from it.
        unsafe { self.z.get_output_chunk(out.as_mut_ptr(), &mut n) }?;
        Ok(n)
    }

    /// `git_zstream_get_output`: the output of all the input that fits
    /// into `out`; the number of bytes written.
    pub fn get_output(&mut self, out: &mut [u8]) -> Result<usize> {
        let mut n = out.len();
        // SAFETY: as in `get_output_chunk`.
        unsafe { self.z.get_output(out.as_mut_ptr(), &mut n) }?;
        Ok(n)
    }

    /// `git_zstream_get_output` into uninitialized memory.
    pub fn get_output_uninit(&mut self, out: &mut [MaybeUninit<u8>]) -> Result<usize> {
        let mut n = out.len();
        // SAFETY: as in `get_output_chunk`; zlib only writes `out`.
        unsafe { self.z.get_output(out.as_mut_ptr().cast::<u8>(), &mut n) }?;
        Ok(n)
    }

    /// The input not consumed yet.
    pub fn in_len(&self) -> usize {
        self.z.in_len
    }

    /// `git_zstream_suggest_output_len`.
    pub fn suggest_output_len(&self) -> usize {
        self.z.suggest_output_len()
    }

    /// `git_zstream_done`: all the input is consumed and the stream ended.
    pub fn done(&self) -> bool {
        self.z.done()
    }

    /// `git_zstream_eos`: the stream ended.
    pub fn eos(&self) -> bool {
        self.z.eos()
    }

    /// `git_zstream_reset`: start over, without input.
    pub fn reset(&mut self) {
        self.z.reset();
    }
}

impl Drop for ZStream<'_> {
    fn drop(&mut self) {
        self.z.free();
    }
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

pub mod ffi {
    use super::*;
    use crate::util::export::{barrier, status};

    /// # Safety
    ///
    /// `zs` points to a `git_zstream` that nothing else accesses during
    /// the call.
    unsafe fn zs<'a>(zs: *mut GitZstream) -> &'a mut GitZstream {
        // SAFETY: the caller's contract.
        unsafe { &mut *zs }
    }

    /// `git_zstream_init`.
    ///
    /// # Safety
    ///
    /// `zstream` points to a `git_zstream` (`GIT_ZSTREAM_INIT` or freed)
    /// that nothing else accesses during the call, and that does not move
    /// until it is freed.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_init(zstream: *mut GitZstream, kind: c_int) -> c_int {
        // SAFETY: forwarded to the caller.
        barrier(|| status(unsafe { zs(zstream) }.init(kind)))
    }

    /// `git_zstream_free`.
    ///
    /// # Safety
    ///
    /// `zstream` points to a `git_zstream` (initialized, freed, or
    /// `GIT_ZSTREAM_INIT`) that nothing else accesses during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_free(zstream: *mut GitZstream) {
        // SAFETY: forwarded to the caller.
        barrier(|| unsafe { zs(zstream) }.free())
    }

    /// `git_zstream_reset`.
    ///
    /// # Safety
    ///
    /// As `git_zstream_free`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_reset(zstream: *mut GitZstream) {
        // SAFETY: forwarded to the caller.
        barrier(|| unsafe { zs(zstream) }.reset())
    }

    /// `git_zstream_set_input`.
    ///
    /// # Safety
    ///
    /// As `git_zstream_free`; `input` stays readable for `len` bytes until
    /// it is consumed or replaced.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_set_input(
        zstream: *mut GitZstream,
        input: *const c_char,
        len: usize,
    ) -> c_int {
        barrier(|| {
            // SAFETY: forwarded to the caller.
            unsafe { zs(zstream) }.set_input_raw(input, len);
            0
        })
    }

    /// `git_zstream_suggest_output_len`.
    ///
    /// # Safety
    ///
    /// As `git_zstream_free`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_suggest_output_len(zstream: *mut GitZstream) -> usize {
        // SAFETY: forwarded to the caller.
        barrier(|| unsafe { zs(zstream) }.suggest_output_len())
    }

    /// `git_zstream_get_output_chunk`.
    ///
    /// # Safety
    ///
    /// `zstream` was initialized and nothing else accesses it during the
    /// call; its input is as `git_zstream_set_input` requires; `out` is
    /// writable for `*out_len` bytes, overlapping neither.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_get_output_chunk(
        out: *mut u8,
        out_len: *mut usize,
        zstream: *mut GitZstream,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `out_len` is a valid `size_t` (the caller's contract),
            // distinct from the stream.
            let out_len = unsafe { &mut *out_len };
            // SAFETY: forwarded to the caller.
            let zs = unsafe { zs(zstream) };
            // SAFETY: forwarded to the caller.
            status(unsafe { zs.get_output_chunk(out, out_len) })
        })
    }

    /// `git_zstream_get_output`.
    ///
    /// # Safety
    ///
    /// As `git_zstream_get_output_chunk`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_get_output(
        out: *mut u8,
        out_len: *mut usize,
        zstream: *mut GitZstream,
    ) -> c_int {
        barrier(|| {
            // SAFETY: as in `git_zstream_get_output_chunk`.
            let out_len = unsafe { &mut *out_len };
            // SAFETY: forwarded to the caller.
            let zs = unsafe { zs(zstream) };
            // SAFETY: forwarded to the caller.
            status(unsafe { zs.get_output(out, out_len) })
        })
    }

    /// `git_zstream_done`.
    ///
    /// # Safety
    ///
    /// As `git_zstream_free`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_done(zstream: *mut GitZstream) -> bool {
        // SAFETY: forwarded to the caller.
        barrier(|| unsafe { zs(zstream) }.done())
    }

    /// `git_zstream_eos`.
    ///
    /// # Safety
    ///
    /// As `git_zstream_free`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_eos(zstream: *mut GitZstream) -> bool {
        // SAFETY: forwarded to the caller.
        barrier(|| unsafe { zs(zstream) }.eos())
    }

    /// # Safety
    ///
    /// `out` is a valid `git_str` that nothing else accesses during the
    /// call; `input` is readable for `len` bytes (or `len` is 0) and does
    /// not lie in `out`'s buffer.
    unsafe fn buf(
        out: *mut GitStr,
        input: *const c_char,
        len: usize,
        f: fn(&mut GitStr, &[u8]) -> Result<()>,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract.
            let out = unsafe { &mut *out };
            // SAFETY: the caller's contract.
            let input = unsafe { crate::util::export::bytes(input, len) };
            status(f(out, input))
        })
    }

    /// `git_zstream_deflatebuf`.
    ///
    /// # Safety
    ///
    /// `out` is a valid `git_str` that nothing else accesses during the
    /// call; `input` is readable for `len` bytes (or `len` is 0) and does
    /// not lie in `out`'s buffer.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_deflatebuf(
        out: *mut GitStr,
        input: *const c_char,
        len: usize,
    ) -> c_int {
        // SAFETY: forwarded to the caller.
        unsafe { buf(out, input, len, deflate_buf) }
    }

    /// `git_zstream_inflatebuf`.
    ///
    /// # Safety
    ///
    /// As `git_zstream_deflatebuf`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_inflatebuf(
        out: *mut GitStr,
        input: *const c_char,
        len: usize,
    ) -> c_int {
        // SAFETY: forwarded to the caller.
        unsafe { buf(out, input, len, inflate_buf) }
    }
}

crate::c_layout_test! {
    headers: ["zstream.h"],
    types: [
        GitZstream => "git_zstream" { z, kind as "type", in_ as "in", in_len, flush, zerr },
        z_stream => "z_stream" {
            next_in, avail_in, total_in, next_out, avail_out, total_out, msg, state,
            zalloc, zfree, opaque, data_type, adler, reserved
        },
    ],
    asserts: [
        "GIT_ZSTREAM_INFLATE == 0",
        "GIT_ZSTREAM_DEFLATE == 1",
        "sizeof(git_zstream_t) == sizeof(int)",
        "Z_OK == 0 && Z_STREAM_END == 1 && Z_BUF_ERROR == -5 && Z_MEM_ERROR == -4",
        "Z_NO_FLUSH == 0 && Z_FINISH == 4 && Z_DEFAULT_COMPRESSION == -1",
    ],
}
