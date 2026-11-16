//! The C ABI of `filebuf.c`: thin shims over [`super`].
//!
//! `git_filebuf_printf` is variadic: its shim (`rust_shims.c`) formats with
//! `vsnprintf` and calls [`git_filebuf__printf_formatted`].

use super::*;
use crate::util::cabi::{CStrArg, Out};
use crate::util::export::{barrier, cstr_bytes, status};

/// The filebuf at `file`.
///
/// # Safety
///
/// `file` is NULL or a valid `git_filebuf` (`GIT_FILEBUF_INIT` or opened)
/// that nothing else accesses during the call.
unsafe fn f<'a>(file: *mut GitFilebuf) -> Option<&'a mut GitFilebuf> {
    // SAFETY: forwarded.
    unsafe { file.as_mut() }
}

/// `write_normal`, stored in `git_filebuf.write`.
///
/// # Safety
///
/// `file` is a valid, opened filebuf; `source` readable for `len` bytes.
pub(super) unsafe extern "C" fn write_normal(
    file: *mut GitFilebuf,
    source: *mut c_void,
    len: usize,
) -> c_int {
    barrier(|| {
        // SAFETY: a valid, opened filebuf, used only by this call.
        let file = unsafe { &mut *file };
        // SAFETY: `len` readable bytes, outside the filebuf.
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
        // SAFETY: a valid, opened filebuf, used only by this call.
        let file = unsafe { &mut *file };
        // SAFETY: `len` readable bytes, outside the filebuf.
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
/// As [`f`] (not NULL).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_cleanup(file: *mut GitFilebuf) {
    barrier(|| {
        // SAFETY: the caller's contract.
        if let Some(file) = unsafe { f(file) } {
            file.cleanup();
        }
    })
}

/// `git_filebuf_flush`.
///
/// # Safety
///
/// As [`f`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_flush(file: *mut GitFilebuf) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract.
        let Some(file) = (unsafe { f(file) }) else {
            return -1;
        };
        file.flush()
    })
}

/// `git_filebuf_open`.
///
/// # Safety
///
/// `file` is a writable `git_filebuf` (zeroed, or cleaned up: its `buffer`
/// is NULL); `path` a NUL-terminated string. The filebuf does not move
/// until it is cleaned up.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_open(
    file: *mut GitFilebuf,
    path: *const c_char,
    flags: c_int,
    mode: mode_t,
) -> c_int {
    // SAFETY: forwarded.
    unsafe { git_filebuf_open_withsize(file, path, flags, mode, WRITE_BUFFER_SIZE) }
}

/// `git_filebuf_open_withsize`.
///
/// # Safety
///
/// As [`git_filebuf_open`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_open_withsize(
    file: *mut GitFilebuf,
    path: *const c_char,
    flags: c_int,
    mode: mode_t,
    size: usize,
) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!file.is_null(), "file", -1);
        crate::git_assert_arg!(!path.is_null(), "path", -1);
        // SAFETY: a writable filebuf, used only by this call.
        let file = unsafe { &mut *file };
        // SAFETY: a NUL-terminated string.
        let p = unsafe { cstr_bytes(path) };
        // SAFETY: the caller keeps the filebuf in place until it is cleaned
        // up (this export's contract).
        status(unsafe { file.open_withsize(p, flags, mode, size) })
    })
}

/// `git_filebuf_hash`.
///
/// # Safety
///
/// `out` is writable for the algorithm's digest size; `file` as in [`f`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_hash(out: *mut u8, file: *mut GitFilebuf) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!out.is_null(), "out", -1);
        // SAFETY: the caller's contract.
        let Some(file) = (unsafe { f(file) }) else {
            let _ = errors::assert_arg_failed("file");
            return -1;
        };
        // SAFETY: the caller passes room for the digest.
        status(unsafe { file.hash_into(out) })
    })
}

/// `git_filebuf_commit_at`.
///
/// # Safety
///
/// As [`f`]; `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_commit_at(file: *mut GitFilebuf, path: CStrArg<'_>) -> c_int {
    barrier(|| {
        // SAFETY: NULL or a valid filebuf, used only by this call.
        let Some(file) = (unsafe { f(file) }) else {
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
/// As [`f`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_commit(file: *mut GitFilebuf) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract.
        let Some(file) = (unsafe { f(file) }) else {
            return errors::assert_arg_failed("file").raw();
        };
        status(file.commit())
    })
}

/// `git_filebuf_write`.
///
/// # Safety
///
/// As [`f`]; `buff` is readable for `len` bytes, outside the filebuf's
/// buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_write(
    file: *mut GitFilebuf,
    buff: *const c_void,
    len: usize,
) -> c_int {
    barrier(|| {
        // SAFETY: NULL or a valid filebuf, used only by this call.
        let Some(file) = (unsafe { f(file) }) else {
            return -1;
        };
        if len > isize::MAX as usize {
            // No readable buffer is this large.
            return -1;
        }
        // SAFETY: `len` readable bytes outside the filebuf.
        let data = unsafe { crate::util::export::bytes(buff.cast(), len) };
        status(file.write(data))
    })
}

/// `git_filebuf_reserve`.
///
/// # Safety
///
/// As [`f`]; `buff` is writable. The reserved bytes stay valid until the
/// next call on `file`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_reserve(
    file: *mut GitFilebuf,
    mut buff: Out<'_, *mut c_void>,
    len: usize,
) -> c_int {
    barrier(|| {
        buff.write(core::ptr::null_mut());
        // SAFETY: the caller's contract.
        let Some(file) = (unsafe { f(file) }) else {
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
/// As [`f`]; `data` is readable for `len` bytes when `len >= 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf__printf_formatted(
    file: *mut GitFilebuf,
    data: *const c_char,
    len: c_int,
) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract.
        let Some(file) = (unsafe { f(file) }) else {
            return -1;
        };
        let d = if len < 0 {
            None
        } else {
            // SAFETY: `len` readable bytes.
            Some(unsafe { crate::util::export::bytes(data, len as usize) })
        };
        status(file.printf_formatted(d))
    })
}

/// `git_filebuf_stats`.
///
/// # Safety
///
/// As [`f`]; `mtime` and `size` are NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_filebuf_stats(
    mut mtime: Out<'_, libc::time_t>,
    mut size: Out<'_, usize>,
    file: *mut GitFilebuf,
) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract.
        let Some(file) = (unsafe { f(file) }) else {
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
