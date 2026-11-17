//! Port of `src/util/zstream.c`: libgit2's zlib stream (`git_zstream`).
//!
//! [`GitZstream`] is mirrored (kind 3): C that stays embeds it by value
//! (`git_packfile_stream`, `git_packbuilder`, odb_loose's stream) and reads
//! its `in_len` (pack.c). It embeds a zlib `z_stream`, which only this
//! module touches.
//!
//! Inflating uses zlib-rs (`libz-rs-sys`), a Rust zlib with zlib's API,
//! stream layout, return codes and messages, and faster than the system
//! zlib in bulk (see `rust/bench/RESULTS.md`) — but slower to set up and
//! to start (larger root tables). So an inflate stream picks its library
//! at its first output request: the system zlib for a request under
//! 4 KiB (header reads), zlib-rs otherwise; streams known to read whole
//! objects ([`ZStream::new_bulk`], `zstream_buf`) start on zlib-rs. The
//! outputs, codes and messages are the same either way. Deflating uses the system zlib
//! ([`crate::sys::zlib`]): zlib-rs compresses to other bytes, and the
//! deflated bytes are visible (binary patches, packs, loose objects). A
//! stream is one or the other for its whole life, so the two never see
//! each other's state.
//!
//! # No references while zlib holds a pointer
//!
//! zlib's deflate state keeps a pointer back to its `z_stream`
//! (`s->strm`), through which later `deflate()` calls read and write the
//! stream. So an initialized stream is reached only through one raw
//! pointer ([`ZstreamPtr`]), derived once from C's pointer (or the Rust
//! owner's): no `&mut GitZstream` or `&mut z_stream` lives across a zlib
//! call. Between calls, [`ZstreamPtr::with`] lends the fields to a closure
//! that makes no zlib call: zlib touches the stream only inside its calls,
//! so that reference overlaps none of its accesses. zlib-rs keeps no such
//! pointer, but both kinds go the same way.
//!
//! An initialized stream must not be moved or copied: zlib detected that
//! (`deflateStateCheck`), zlib-rs does not (rust/DIVERGENCES.md).
//!
//! The safe API is [`ZStream`] (a heap stream that borrows its input),
//! [`inflate`] and [`deflate`].

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
/// Internal `kind` values of an inflating stream (C never
/// reads the field back): not started yet (the library is picked by the
/// first output request), and started on the system zlib.
const KIND_INFLATE_PENDING: c_int = -1;
const KIND_INFLATE_SYS: c_int = -2;
/// First output requests below this go to the system zlib.
const SMALL_OUTPUT: usize = 4096;
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

// `ZstreamPtr::z` hands zlib the stream's own address for its `z`.
const _: () = assert!(core::mem::offset_of!(GitZstream, z) == 0);

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

    /// The input not consumed yet (`in_len`, which pack.c reads). A shared
    /// read between calls, which zlib does not mind.
    pub fn in_len(&self) -> usize {
        self.in_len
    }
}

/// A `git_zstream` reached only through this pointer (see the module
/// documentation). Fields are accessed through [`with`](Self::with), a
/// reference that ends before the next zlib call; zlib gets pointers
/// derived from this one ([`z`](Self::z)).
///
/// It has the ABI of `git_zstream *`: the exports take it, and their
/// `# Safety` (C's contract) is [`new`](Self::new)'s.
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct ZstreamPtr<'a>(*mut GitZstream, PhantomData<&'a mut GitZstream>);

impl<'a> ZstreamPtr<'a> {
    /// # Safety
    ///
    /// `p` points to a `git_zstream` (in any state C allows) that, for
    /// `'a`, is valid, accessed only through this pointer and its copies,
    /// from one thread at a time, and not moved while initialized; no
    /// reference to it is live while one of these copies is used.
    pub(crate) unsafe fn new(p: *mut GitZstream) -> ZstreamPtr<'a> {
        ZstreamPtr(p, PhantomData)
    }

    /// The `z_stream`, as zlib is given it: the first field of the
    /// `repr(C)` stream, so the same address (no place is formed).
    fn z(self) -> *mut z_stream {
        self.0.cast::<z_stream>()
    }

    /// Run `f` on the stream's fields, between zlib calls.
    ///
    /// zlib reads and writes the stream only inside the calls this module
    /// makes (through the pointer it is given, or the one deflate stored
    /// at init), and `f` makes none (they take [`z`](Self::z), and need an
    /// `unsafe` block of their own): so a reference that ends when `f`
    /// returns overlaps no access of zlib's. No caller nests `with`.
    #[inline(always)]
    fn with<R>(self, f: impl FnOnce(&mut GitZstream) -> R) -> R {
        // SAFETY: `self.0` is a valid, aligned `git_zstream` for `'a`,
        // reached only through this pointer and its copies, by one thread
        // at a time, with no reference to it live (the constructor's
        // contract); no copy is used while `f` runs (no `with` nests, and
        // zlib is not running), so this reference is the only access.
        f(unsafe { &mut *self.0 })
    }

    fn kind(self) -> c_int {
        self.with(|s| s.kind)
    }

    fn inflates(self) -> bool {
        matches!(self.kind(), 0 | KIND_INFLATE_PENDING | KIND_INFLATE_SYS)
    }

    /// Start the inflater the first output request picks: the system
    /// zlib for a small one (cheaper to set up), zlib-rs otherwise (faster
    /// in bulk). Both give the same output, codes and messages.
    fn start_inflate(self, out_len: usize) -> c_int {
        // Miri cannot run the system zlib (foreign code): zlib-rs there.
        let sys = !cfg!(miri) && out_len < SMALL_OUTPUT;
        let r = if sys {
            // SAFETY: `z()` is the valid stream, without state yet,
            // reached only through this pointer and not moved while
            // initialized (the constructor's contract), with no reference
            // to it live.
            unsafe { zlib::inflate_init(self.z()) }
        } else {
            let version = libz_rs_sys::zlibVersion();
            // SAFETY: as above, for zlib-rs; `version` is its static
            // version string and the size that of the stream.
            unsafe {
                libz_rs_sys::inflateInit_(
                    self.z(),
                    version,
                    core::mem::size_of::<z_stream>() as c_int,
                )
            }
        };
        if r == Z_OK {
            self.with(|s| s.kind = if sys { KIND_INFLATE_SYS } else { 0 });
        }
        r
    }

    fn zerr(self) -> c_int {
        self.with(|s| s.zerr)
    }

    fn set_zerr(self, v: c_int) {
        self.with(|s| s.zerr = v);
    }

    pub(crate) fn in_len(self) -> usize {
        self.with(|s| s.in_len)
    }

    /// `zstream_seterr`.
    fn seterr(self) -> Result<()> {
        match self.zerr() {
            // Z_BUF_ERROR is not fatal: the caller retries with more room.
            Z_OK | Z_STREAM_END | Z_BUF_ERROR => Ok(()),
            Z_MEM_ERROR => Err(errors::oom()),
            _ => {
                let msg = self.with(|s| s.z.msg);
                if msg.is_null() {
                    Err(git_error_set!(
                        ErrorClass::ZLIB,
                        "unknown compression error"
                    ))
                } else {
                    // SAFETY: zlib (and zlib-rs) set `msg` to NULL or to a
                    // static NUL-terminated message, never written.
                    let m = unsafe { CStr::from_ptr(msg) };
                    Err(errors::set_str(ErrorClass::ZLIB, m.to_bytes()))
                }
            }
        }
    }

    /// `git_zstream_init`. `kind` is C's `git_zstream_t` value.
    pub(crate) fn init(self, kind: c_int) -> Result<()> {
        self.with(|s| {
            s.kind = if kind == 0 {
                KIND_INFLATE_PENDING
            } else {
                kind
            }
        });
        let r = if self.kind() == KIND_INFLATE_PENDING {
            Z_OK
        } else if self.inflates() {
            // SAFETY: `z()` is the valid stream, reached only through this
            // pointer and not moved while initialized (the constructor's
            // contract), with no reference to it live; `zlibVersion` is
            // zlib-rs's static version, the size that of the stream; the
            // allocator fields are NULL (zlib-rs's default) or what C put
            // there, as zlib's contract.
            unsafe {
                libz_rs_sys::inflateInit_(
                    self.z(),
                    libz_rs_sys::zlibVersion(),
                    core::mem::size_of::<z_stream>() as c_int,
                )
            }
        } else {
            // SAFETY: as above, for the system zlib, which keeps `self.z()`
            // as the stream's address: the constructor's contract keeps it
            // valid (and the only way in) until the stream is freed.
            unsafe { zlib::deflate_init(self.z(), Z_DEFAULT_COMPRESSION) }
        };
        self.set_zerr(r);
        self.seterr()
    }

    /// `init`, with an inflater that is zlib-rs from the
    /// start (streams that read the whole object, whatever their first
    /// output size).
    pub(crate) fn init_bulk(self, kind: c_int) -> Result<()> {
        self.init(kind)?;
        if self.kind() == KIND_INFLATE_PENDING {
            let r = self.start_inflate(usize::MAX);
            self.set_zerr(r);
            self.seterr()?;
        }
        Ok(())
    }

    /// `git_zstream_free`. Ending a stream that has no state (never
    /// initialized, or freed) does nothing.
    pub(crate) fn free(self) {
        let kind = self.kind();
        if kind == KIND_INFLATE_PENDING {
        } else if kind == KIND_INFLATE_SYS {
            // SAFETY: `z()` is the valid stream with no reference live
            // (the constructor's contract), whose state is the system
            // zlib's inflate state (`start_inflate` set this kind), or
            // NULL after a free (which zlib rejects).
            unsafe { zlib::inflate_end(self.z()) };
        } else if self.inflates() {
            // SAFETY: as above; the state is NULL or this stream's zlib-rs
            // inflate state: only `init`/`start_inflate` set it, with the
            // library `kind` names.
            unsafe { libz_rs_sys::inflateEnd(self.z()) };
        } else {
            // SAFETY: as above, for the system zlib's deflate state.
            unsafe { zlib::deflate_end(self.z()) };
        }
    }

    /// `git_zstream_reset`.
    pub(crate) fn reset(self) {
        let kind = self.kind();
        if kind == KIND_INFLATE_PENDING {
        } else if kind == KIND_INFLATE_SYS {
            // SAFETY: as in `free`.
            unsafe { zlib::inflate_reset(self.z()) };
        } else if self.inflates() {
            // SAFETY: as in `free`.
            unsafe { libz_rs_sys::inflateReset(self.z()) };
        } else {
            // SAFETY: as in `free`.
            unsafe { zlib::deflate_reset(self.z()) };
        }
        self.with(|s| {
            s.in_ = core::ptr::null();
            s.in_len = 0;
            s.zerr = Z_STREAM_END;
        });
    }

    /// `git_zstream_set_input`: the input is read by the next
    /// [`get_output`](Self::get_output) calls, whose callers vouch for it.
    pub(crate) fn set_input(self, input: *const c_char, len: usize) {
        self.with(|s| {
            s.in_ = input;
            s.in_len = len;
            s.zerr = Z_OK;
        });
    }

    /// `git_zstream_done`.
    pub(crate) fn done(self) -> bool {
        self.with(|s| s.in_len == 0 && s.zerr == Z_STREAM_END)
    }

    /// `git_zstream_eos`.
    pub(crate) fn eos(self) -> bool {
        self.zerr() == Z_STREAM_END
    }

    /// `git_zstream_suggest_output_len`.
    pub(crate) fn suggest_output_len(self) -> usize {
        let in_len = self.in_len();
        if in_len > ZSTREAM_BUFFER_SIZE {
            ZSTREAM_BUFFER_SIZE
        } else if in_len > ZSTREAM_BUFFER_MIN_EXTRA {
            in_len
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
    /// The input given to [`set_input`](Self::set_input) is still readable
    /// for `in_len` bytes and not written meanwhile; `out` is writable for
    /// `*out_len` bytes (maybe uninitialized) and overlaps neither the
    /// input nor the stream.
    pub(crate) unsafe fn get_output_chunk(self, out: *mut u8, out_len: &mut usize) -> Result<()> {
        let want = *out_len;
        let (in_queued, out_queued, flush) = self.with(|s| {
            s.z.next_in = s.in_.cast::<u8>();
            if s.in_len > c_uint::MAX as usize {
                s.z.avail_in = c_uint::MAX;
                s.flush = Z_NO_FLUSH;
            } else {
                s.z.avail_in = s.in_len as c_uint;
                s.flush = Z_FINISH;
            }
            s.z.next_out = out;
            s.z.avail_out = want as c_uint;
            if s.z.avail_out as usize != want {
                s.z.avail_out = c_uint::MAX;
            }
            (s.z.avail_in as usize, s.z.avail_out as usize, s.flush)
        });
        if self.kind() == KIND_INFLATE_PENDING {
            let r = self.start_inflate(out_queued);
            if r != Z_OK {
                self.set_zerr(r);
                return self.seterr();
            }
        }
        let z = self.z();
        let r = if self.kind() == KIND_INFLATE_SYS {
            // SAFETY: as below, for the system zlib's inflate state.
            unsafe { zlib::inflate(z, flush) }
        } else if self.inflates() {
            // SAFETY: `z` is the valid stream with no reference live (the
            // constructor's contract); its state is the inflate state
            // `init` made (or NULL, which zlib-rs rejects); `next_in` /
            // `next_out` are readable / writable for `avail_in` /
            // `avail_out` bytes, which are at most `in_len` / `*out_len`
            // (this fn's contract), and do not overlap each other or the
            // stream.
            unsafe { libz_rs_sys::inflate(z, flush) }
        } else {
            // SAFETY: as above, for the system zlib's deflate state.
            unsafe { zlib::deflate(z, flush) }
        };
        self.set_zerr(r);
        self.seterr()?;

        *out_len = self.with(|s| {
            let in_used = in_queued - s.z.avail_in as usize;
            s.in_len -= in_used;
            s.in_ = s.in_.wrapping_add(in_used);
            out_queued - s.z.avail_out as usize
        });
        Ok(())
    }

    /// `git_zstream_get_output`: all the output of the input, into `out`
    /// (`*out_len` bytes); `*out_len` becomes the bytes written.
    ///
    /// # Safety
    ///
    /// As [`get_output_chunk`](Self::get_output_chunk).
    pub(crate) unsafe fn get_output(self, mut out: *mut u8, out_len: &mut usize) -> Result<()> {
        let mut out_remain = *out_len;

        if self.in_len() != 0 && self.zerr() == Z_STREAM_END {
            return Err(git_error_set!(
                ErrorClass::ZLIB,
                "zlib input had trailing garbage"
            ));
        }

        while out_remain > 0 && self.zerr() != Z_STREAM_END {
            let mut out_written = out_remain;
            // SAFETY: the input is as `get_output_chunk` requires (this
            // fn's contract), and `out` is writable for `out_remain` bytes
            // (the contract, advanced by what was written).
            unsafe { self.get_output_chunk(out, &mut out_written) }?;

            // With room left, a Z_BUF_ERROR that wrote nothing is input
            // that ended mid-stream: more will not come within this loop.
            if out_written == 0 && self.zerr() == Z_BUF_ERROR {
                return Err(git_error_set!(
                    ErrorClass::ZLIB,
                    "premature end of zlib stream"
                ));
            }

            out_remain -= out_written;
            out = out.wrapping_add(out_written);
        }

        let flush = self.with(|s| s.flush);
        // Either the input is finished or the data was not flushed.
        git_assert!(
            self.in_len() > 0 || flush == Z_FINISH,
            "zstream->in_len > 0 || zstream->flush == Z_FINISH"
        );

        *out_len -= out_remain;
        Ok(())
    }
}

/// `zstream_buf`: append the (de/in)flated `input` to `out`.
fn zstream_buf(out: &mut GitStr, input: &[u8], kind: ZstreamKind) -> Result<()> {
    /// Frees the stream however the function returns.
    struct Free<'a>(ZstreamPtr<'a>);
    impl Drop for Free<'_> {
        fn drop(&mut self) {
            self.0.free();
        }
    }

    // On the stack, as C's: it does not move until it is freed, and is
    // reached only through `zs` from here on.
    let mut stream = GitZstream::new();
    // SAFETY: a local, used only through this pointer until the end.
    let zs = unsafe { ZstreamPtr::new(&raw mut stream) };
    // C returns without freeing when the init fails (zlib then holds no
    // state).
    zs.init_bulk(kind as c_int)?;
    let guard = Free(zs);
    zs.set_input(input.as_ptr().cast::<c_char>(), input.len());
    while !zs.done() {
        let step = zs.suggest_output_len();
        out.grow_by(step)?;
        // C writes `asize - size` bytes at `ptr + size`, over the NUL.
        let (spare, mut written) = out.spare_ptr();
        // SAFETY: the input is `input`, borrowed for the call; `spare` is
        // the string's writable room, distinct from both.
        unsafe { zs.get_output(spare, &mut written) }?;
        let len = out.len() + written;
        // SAFETY: zlib initialized the `written` bytes after the contents,
        // within the allocation (`len <= asize`); `terminate` below (or
        // the next growth) puts the NUL back.
        unsafe { out.set_len(len) };
    }
    drop(guard);
    // NUL-terminate. C does so only if there is room: output that filled
    // the buffer exactly was left without its NUL (rust/DEFECTS.md).
    out.terminate()
}

/// `git_zstream_deflatebuf`: append the deflated `input` to `out`.
pub(crate) fn deflate_buf(out: &mut GitStr, input: &[u8]) -> Result<()> {
    zstream_buf(out, input, ZstreamKind::Deflate)
}

/// `git_zstream_inflatebuf`: append the inflated `input` to `out`.
pub(crate) fn inflate_buf(out: &mut GitStr, input: &[u8]) -> Result<()> {
    zstream_buf(out, input, ZstreamKind::Inflate)
}

// ---------------------------------------------------------------------------
// The safe API
// ---------------------------------------------------------------------------

/// A zlib stream that borrows its input for `'a`: an initialized
/// `git_zstream` on the heap, reached only through its pointer, freed on
/// drop.
pub struct ZStream<'a> {
    /// From `Box::into_raw`; freed by `drop`.
    raw: *mut GitZstream,
    _owns: PhantomData<Box<GitZstream>>,
    _input: PhantomData<&'a [u8]>,
}

// SAFETY: `raw` is the only pointer to the boxed `GitZstream` (as a
// `Box`, which is `Send`); in it, `z`'s pointers are zlib's state (zlib-rs's
// or the system zlib's: heap memory with no thread affinity, reached only
// through the stream) and the current input/output, `in_` points into the
// `&'a [u8]` input (`[u8]` is `Sync`), and the rest are integers. Moving
// the `ZStream` moves every user of them; it is not `Sync`.
unsafe impl Send for ZStream<'_> {}

impl core::fmt::Debug for ZStream<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ZStream")
            .field("in_len", &self.z().in_len())
            .field("eos", &self.z().eos())
            .finish()
    }
}

impl<'a> ZStream<'a> {
    /// A stream that inflates with zlib-rs from the start (for streams
    /// that read whole objects), or deflates.
    pub fn new_bulk(kind: ZstreamKind) -> Result<ZStream<'a>> {
        let raw = Box::into_raw(Box::new(GitZstream::new()));
        let zs = ZStream {
            raw,
            _owns: PhantomData,
            _input: PhantomData,
        };
        zs.z().init_bulk(kind as c_int).map(|()| zs)
    }

    /// A stream that inflates or deflates (`git_zstream_init`).
    pub fn new(kind: ZstreamKind) -> Result<ZStream<'a>> {
        let raw = Box::into_raw(Box::new(GitZstream::new()));
        let zs = ZStream {
            raw,
            _owns: PhantomData,
            _input: PhantomData,
        };
        zs.z().init(kind as c_int).map(|()| zs)
    }

    /// The stream, for the borrow of `self`.
    fn z(&self) -> ZstreamPtr<'_> {
        // SAFETY: the allocation of `new`, which only this `ZStream`
        // reaches (through `raw`), never moves, and is freed by `drop`
        // only; a `ZStream` is used by one thread at a time (not `Sync`).
        unsafe { ZstreamPtr::new(self.raw) }
    }

    /// Give the input (`git_zstream_set_input`).
    pub fn set_input(&mut self, input: &'a [u8]) {
        self.z()
            .set_input(input.as_ptr().cast::<c_char>(), input.len());
    }

    /// `git_zstream_get_output_chunk`: as much output as the input gives
    /// at once, into `out`; the number of bytes written.
    pub fn get_output_chunk(&mut self, out: &mut [u8]) -> Result<usize> {
        let mut n = out.len();
        // SAFETY: the input is borrowed for `'a`, which outlives `self`;
        // `out` is writable for its length and is distinct from it.
        unsafe { self.z().get_output_chunk(out.as_mut_ptr(), &mut n) }?;
        Ok(n)
    }

    /// `git_zstream_get_output`: the output of all the input that fits
    /// into `out`; the number of bytes written.
    pub fn get_output(&mut self, out: &mut [u8]) -> Result<usize> {
        let mut n = out.len();
        // SAFETY: as in `get_output_chunk`.
        unsafe { self.z().get_output(out.as_mut_ptr(), &mut n) }?;
        Ok(n)
    }

    /// `git_zstream_get_output` into uninitialized memory; the first
    /// returned-count bytes of `out` are then initialized.
    pub fn get_output_uninit(&mut self, out: &mut [MaybeUninit<u8>]) -> Result<usize> {
        let mut n = out.len();
        // SAFETY: as in `get_output_chunk`; zlib only writes `out`.
        unsafe { self.z().get_output(out.as_mut_ptr().cast::<u8>(), &mut n) }?;
        Ok(n)
    }

    /// The input not consumed yet.
    pub fn in_len(&self) -> usize {
        self.z().in_len()
    }

    /// `git_zstream_suggest_output_len`.
    pub fn suggest_output_len(&self) -> usize {
        self.z().suggest_output_len()
    }

    /// `git_zstream_done`: all the input is consumed and the stream ended.
    pub fn done(&self) -> bool {
        self.z().done()
    }

    /// `git_zstream_eos`: the stream ended.
    pub fn eos(&self) -> bool {
        self.z().eos()
    }

    /// `git_zstream_reset`: start over, without input.
    pub fn reset(&mut self) {
        self.z().reset();
    }
}

impl Drop for ZStream<'_> {
    fn drop(&mut self) {
        self.z().free();
        // SAFETY: the allocation of `new`, freed once; zlib's state is
        // gone, so nothing points into it any more.
        drop(unsafe { Box::from_raw(self.raw) });
    }
}

/// The (de/in)flated `input`, as `zstream_buf` makes it.
fn zstream_vec(input: &[u8], kind: ZstreamKind) -> Result<Vec<u8>> {
    let mut zs = ZStream::new(kind)?;
    zs.set_input(input);
    let mut out: Vec<u8> = Vec::new();
    while !zs.done() {
        let step = zs.suggest_output_len();
        out.try_reserve(step).map_err(|_| errors::oom())?;
        let spare = &mut out.spare_capacity_mut()[..step];
        let n = zs.get_output_uninit(spare)?;
        let len = out.len() + n;
        // SAFETY: zlib initialized the `n` bytes after the contents, within
        // the reserved capacity.
        unsafe { out.set_len(len) };
    }
    Ok(out)
}

/// `input`, deflated (zlib format, default level: the bytes libgit2
/// writes).
pub fn deflate(input: &[u8]) -> Result<Vec<u8>> {
    zstream_vec(input, ZstreamKind::Deflate)
}

/// `input`, inflated. Fails on a corrupt or truncated stream, or one with
/// trailing data.
pub fn inflate(input: &[u8]) -> Result<Vec<u8>> {
    zstream_vec(input, ZstreamKind::Inflate)
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

pub mod ffi {
    use super::*;
    use crate::util::export::{barrier, status};

    // The exports take the stream as a `ZstreamPtr` (the ABI of
    // `git_zstream *`): their contract, "a `git_zstream` that nothing else
    // accesses during the call, from which C forms no reference while
    // these run, kept in place while initialized", is `ZstreamPtr::new`'s
    // for the call.

    /// `git_zstream_init`.
    ///
    /// # Safety
    ///
    /// `zstream` points to a `git_zstream` (`GIT_ZSTREAM_INIT` or freed)
    /// that nothing else accesses during the call, and that is neither
    /// moved nor copied until it is freed.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_init(zstream: ZstreamPtr<'_>, kind: c_int) -> c_int {
        barrier(|| status(zstream.init(kind)))
    }

    /// `git_zstream_free`.
    ///
    /// # Safety
    ///
    /// `zstream` points to a `git_zstream` (initialized, freed, or
    /// `GIT_ZSTREAM_INIT`) that nothing else accesses during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_free(zstream: ZstreamPtr<'_>) {
        barrier(|| zstream.free())
    }

    /// `git_zstream_reset`.
    ///
    /// # Safety
    ///
    /// As `git_zstream_free`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_reset(zstream: ZstreamPtr<'_>) {
        barrier(|| zstream.reset())
    }

    /// `git_zstream_set_input`.
    ///
    /// # Safety
    ///
    /// As `git_zstream_free`; `input` stays readable for `len` bytes until
    /// it is consumed or replaced.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_set_input(
        zstream: ZstreamPtr<'_>,
        input: *const c_char,
        len: usize,
    ) -> c_int {
        barrier(|| {
            zstream.set_input(input, len);
            0
        })
    }

    /// `git_zstream_suggest_output_len`.
    ///
    /// # Safety
    ///
    /// As `git_zstream_free`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_suggest_output_len(zstream: ZstreamPtr<'_>) -> usize {
        barrier(|| zstream.suggest_output_len())
    }

    /// `git_zstream_get_output_chunk`.
    ///
    /// # Safety
    ///
    /// `zstream` was initialized and nothing else accesses it during the
    /// call; its input is as `git_zstream_set_input` requires; `out_len`
    /// is a valid `size_t`, and `out` is writable for `*out_len` bytes
    /// (maybe uninitialized); none of them overlaps another, or the input.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_get_output_chunk(
        out: *mut u8,
        out_len: &mut usize,
        zstream: ZstreamPtr<'_>,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the stream's input and `out` are as `get_output_chunk`
            // requires (this export's contract).
            status(unsafe { zstream.get_output_chunk(out, out_len) })
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
        out_len: &mut usize,
        zstream: ZstreamPtr<'_>,
    ) -> c_int {
        barrier(|| {
            // SAFETY: as in `git_zstream_get_output_chunk`.
            status(unsafe { zstream.get_output(out, out_len) })
        })
    }

    /// `git_zstream_done`.
    ///
    /// # Safety
    ///
    /// As `git_zstream_free`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_done(zstream: ZstreamPtr<'_>) -> bool {
        barrier(|| zstream.done())
    }

    /// `git_zstream_eos`.
    ///
    /// # Safety
    ///
    /// As `git_zstream_free`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_eos(zstream: ZstreamPtr<'_>) -> bool {
        barrier(|| zstream.eos())
    }

    /// # Safety
    ///
    /// `input` is readable for `len` bytes (or `len` is 0), unwritten
    /// during the call, and does not lie in `out`'s buffer.
    unsafe fn buf(
        out: &mut GitStr,
        input: *const c_char,
        len: usize,
        f: fn(&mut GitStr, &[u8]) -> Result<()>,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `input` is readable for `len` bytes, unwritten during
            // the call (this fn's contract): `out` is written only in its
            // own buffer, where `input` is not.
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
        out: &mut GitStr,
        input: *const c_char,
        len: usize,
    ) -> c_int {
        // SAFETY: `input` is as `buf` requires (this export's contract).
        unsafe { buf(out, input, len, deflate_buf) }
    }

    /// `git_zstream_inflatebuf`.
    ///
    /// # Safety
    ///
    /// As `git_zstream_deflatebuf`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_zstream_inflatebuf(
        out: &mut GitStr,
        input: *const c_char,
        len: usize,
    ) -> c_int {
        // SAFETY: `input` is as `buf` requires (this export's contract).
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `zlib.compress(b"a" * 16)`: 16 bytes of output into the 16 bytes
    /// the C's growth gives, which the C left without a NUL.
    const A16: [u8; 11] = [
        0x78, 0x9c, 0x4b, 0x4c, 0x44, 0x05, 0x00, 0x33, 0x98, 0x06, 0x11,
    ];

    #[test]
    fn inflate_buf_terminates_a_full_buffer() {
        let _g = crate::util::test_support::init();
        let mut out = GitStr::new();
        inflate_buf(&mut out, &A16).unwrap();
        assert_eq!(out.as_bytes(), [b'a'; 16]);
        assert!(out.capacity() > out.len());
        assert_eq!(out.as_c_str().unwrap().to_bytes(), [b'a'; 16]);
        // Appending to existing contents.
        inflate_buf(&mut out, &A16).unwrap();
        assert_eq!(out.as_bytes(), [b'a'; 32]);
        assert_eq!(out.as_c_str().unwrap().to_bytes(), [b'a'; 32]);
    }
}
