//! The C ABI of `filebuf.c`: thin shims over [`super`].
//!
//! `git_filebuf_printf` is variadic: its shim (`rust_shims.c`) formats with
//! `vsnprintf` and calls [`git_filebuf__printf_formatted`].

use super::*;
use crate::util::cabi::{CStrArg, Out};
use crate::util::export::{barrier, status};

// The exports' `file` is `Option<&mut GitFilebuf>`: NULL or a valid
// `git_filebuf` (`GIT_FILEBUF_INIT`, opened, or cleaned up) that nothing
// else accesses during the call. A `&mut GitFilebuf` covers the struct
// only: its buffers, paths and deflate stream are separate allocations
// reached through its pointers, so data arguments may lie in them.

/// `write_normal`, stored in `git_filebuf.write`.
///
/// # Safety
///
/// `file` is a valid, opened filebuf that nothing else accesses during the
/// call; `source` readable for `len` bytes, unwritten during the call.
///
/// (`file` stays a raw pointer: the function is stored in the C's
/// `git_filebuf.write` slot, of the C's type, [`WriteFn`].)
pub(super) unsafe extern "C" fn write_normal(
    file: *mut GitFilebuf,
    source: *mut c_void,
    len: usize,
) -> c_int {
    barrier(|| {
        // SAFETY: `file` is a valid, opened filebuf that nothing else
        // accesses during the call (this fn's contract).
        let file = unsafe { &mut *file };
        // SAFETY: `source` is readable for `len` bytes and unwritten during
        // the call (this fn's contract).
        let data = unsafe { crate::util::export::bytes(source.cast(), len) };
        match file.write_normal(data) {
            Ok(()) => 0,
            Err(e) => e.raw(),
        }
    })
}

/// `write_deflate`, stored in `git_filebuf.write`.
///
/// # Safety
///
/// As [`write_normal`].
pub(super) unsafe extern "C" fn write_deflate(
    file: *mut GitFilebuf,
    source: *mut c_void,
    len: usize,
) -> c_int {
    barrier(|| {
        // SAFETY: as in `write_normal`.
        let file = unsafe { &mut *file };
        // SAFETY: as in `write_normal`.
        let data = unsafe { crate::util::export::bytes(source.cast(), len) };
        match file.write_deflate(data) {
            Ok(()) => 0,
            Err(e) => e.raw(),
        }
    })
}

/// `git_filebuf_cleanup`.
///
/// # Safety
///
/// `file` is NULL or a valid `git_filebuf` that nothing else accesses
/// during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_cleanup(file: Option<&mut GitFilebuf>) {
    barrier(|| {
        if let Some(file) = file {
            file.cleanup();
        }
    })
}

/// `git_filebuf_flush`.
///
/// # Safety
///
/// As [`git_filebuf_cleanup`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_flush(file: Option<&mut GitFilebuf>) -> c_int {
    barrier(|| {
        let Some(file) = file else {
            return -1;
        };
        file.flush()
    })
}

/// `git_filebuf_open`.
///
/// # Safety
///
/// `file` is NULL or a valid `git_filebuf` (zeroed, or cleaned up: its
/// `buffer` is NULL) that nothing else accesses during the call; `path`
/// NULL or a NUL-terminated string, unwritten during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_open(
    file: Option<&mut GitFilebuf>,
    path: CStrArg<'_>,
    flags: c_int,
    mode: mode_t,
) -> c_int {
    // SAFETY: `file` and `path` are this export's arguments, under the
    // same contract.
    unsafe { git_filebuf_open_withsize(file, path, flags, mode, WRITE_BUFFER_SIZE) }
}

/// `git_filebuf_open_withsize`.
///
/// # Safety
///
/// As [`git_filebuf_open`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_open_withsize(
    file: Option<&mut GitFilebuf>,
    path: CStrArg<'_>,
    flags: c_int,
    mode: mode_t,
    size: usize,
) -> c_int {
    barrier(|| {
        let file = crate::git_arg!(file, "file", -1);
        let p = crate::git_arg!(path.get(), "path", -1).to_bytes();
        status(file.open_withsize(p, flags, mode, size))
    })
}

/// `git_filebuf_hash`.
///
/// # Safety
///
/// `out` is NULL or writable for the algorithm's digest size, and no
/// reference covers it; `file` as in [`git_filebuf_cleanup`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_hash(out: *mut u8, file: Option<&mut GitFilebuf>) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!out.is_null(), "out", -1);
        let file = crate::git_arg!(file, "file", -1);
        // SAFETY: the non-null `out` is writable for the digest's size
        // (this export's contract), `hash_into`'s requirement.
        status(unsafe { file.hash_into(out) })
    })
}

/// `git_filebuf_commit_at`.
///
/// # Safety
///
/// `file` as in [`git_filebuf_cleanup`]; `path` NULL or a NUL-terminated
/// string, unwritten during the call, and not `file`'s own
/// `path_original` (which the call frees before copying `path`: C's
/// use-after-free too).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_commit_at(
    file: Option<&mut GitFilebuf>,
    path: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        let Some(file) = file else {
            return -1;
        };
        let Some(p) = path.get() else {
            return -1;
        };
        status(file.commit_at(p))
    })
}

/// `git_filebuf_commit`.
///
/// # Safety
///
/// As [`git_filebuf_cleanup`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_commit(file: Option<&mut GitFilebuf>) -> c_int {
    barrier(|| {
        let Some(file) = file else {
            return errors::assert_arg_failed("file").raw();
        };
        status(file.commit())
    })
}

/// `git_filebuf_write`.
///
/// # Safety
///
/// `file` as in [`git_filebuf_cleanup`]; `buff` is readable for `len`
/// bytes, unwritten during the call and outside the filebuf's buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_write(
    file: Option<&mut GitFilebuf>,
    buff: *const c_void,
    len: usize,
) -> c_int {
    barrier(|| {
        let Some(file) = file else {
            return -1;
        };
        if len > isize::MAX as usize {
            // No readable buffer is this large.
            return -1;
        }
        // SAFETY: `buff` has `len <= isize::MAX` readable bytes that
        // nothing writes during the call, outside the buffer the write
        // fills (this export's contract).
        let data = unsafe { crate::util::export::bytes(buff.cast(), len) };
        status(file.write(data))
    })
}

/// `git_filebuf_reserve`.
///
/// # Safety
///
/// `file` as in [`git_filebuf_cleanup`]; `buff` is NULL or writable. The
/// reserved bytes stay valid until the next call on `file`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_reserve(
    file: Option<&mut GitFilebuf>,
    mut buff: Out<'_, *mut c_void>,
    len: usize,
) -> c_int {
    barrier(|| {
        buff.write(core::ptr::null_mut());
        let Some(file) = file else {
            return -1;
        };
        match file.reserve_offset(len) {
            Ok(at) => {
                let p = file.buffer.wrapping_add(at);
                buff.write(p.cast());
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// The body of `git_filebuf_printf`: `len` formatted bytes at `data`
/// (`len < 0`: the formatting, or the allocation for it, failed).
///
/// # Safety
///
/// `file` as in [`git_filebuf_cleanup`]; `data` is readable for `len`
/// bytes when `len >= 0`, unwritten during the call and outside the
/// filebuf's buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf__printf_formatted(
    file: Option<&mut GitFilebuf>,
    data: *const c_char,
    len: c_int,
) -> c_int {
    barrier(|| {
        let Some(file) = file else {
            return -1;
        };
        let d = if len < 0 {
            None
        } else {
            // SAFETY: `data` has `len >= 0` readable bytes, unwritten
            // during the call (this export's contract).
            Some(unsafe { crate::util::export::bytes(data, len as usize) })
        };
        status(file.printf_formatted(d))
    })
}

/// `git_filebuf_stats`.
///
/// # Safety
///
/// `file` is NULL or a valid `git_filebuf` that nothing writes during the
/// call; `mtime` and `size` are NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_stats(
    mut mtime: Out<'_, libc::time_t>,
    mut size: Out<'_, usize>,
    file: Option<&GitFilebuf>,
) -> c_int {
    barrier(|| {
        let Some(file) = file else {
            return -1;
        };
        match file.stats() {
            Ok((m, s)) => {
                mtime.write(m);
                size.write(s);
                0
            }
            Err(e) => e.raw(),
        }
    })
}
