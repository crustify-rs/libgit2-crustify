//! Port of `src/util/posix.c` and the Unix half of `posix.h` /
//! `unix/posix.h`: the `p_*` system-call wrappers.
//!
//! The header-only part (`p_lstat`, `p_mkdir`, `p_close`, `p_fsync`, … are
//! macros or inline functions over libc) stays in the headers for the C that
//! stays; this module gives the Rust its equivalents: safe functions that
//! take paths as `&CStr` (paths are bytes) and return
//! [`SysResult`], whose error is the `errno` value. `errno` itself is left
//! as the call set it, so that a following
//! `git_error_set!(ErrorClass::OS, …)` appends its message as the C's
//! `git_error_set(GIT_ERROR_OS, …)` does.
//!
//! `p_open` is variadic in C: its shim (`rust_shims.c`) collects the mode and
//! calls [`ffi::git__p_open`].

use core::ffi::{CStr, c_char, c_int, c_void};
use core::sync::atomic::{AtomicUsize, Ordering};

use super::errors::{self, Result};

pub use libc::{mode_t, off_t, stat as Stat};

/// An `errno` value returned by a failed system call (which also left it in
/// `errno`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Errno(pub c_int);

/// The result of a system call wrapper.
pub type SysResult<T> = core::result::Result<T, Errno>;

#[inline]
fn last_errno() -> Errno {
    Errno(errors::errno())
}

#[inline]
fn check(r: c_int) -> SysResult<()> {
    if r < 0 { Err(last_errno()) } else { Ok(()) }
}

/// `O_BINARY` (0 on Unix).
pub const O_BINARY: c_int = 0;

/// `p_fsync__cnt`: the number of `p_fsync` calls (for tests). C's inline
/// `p_fsync` increments it without atomicity, as it did; Rust's does with.
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub static p_fsync__cnt: AtomicUsize = AtomicUsize::new(0);

/// `S_IFGITLINK`.
pub const S_IFGITLINK: mode_t = 0o160000;

/// `S_ISDIR`.
#[inline]
pub const fn s_isdir(m: mode_t) -> bool {
    m & libc::S_IFMT == libc::S_IFDIR
}
/// `S_ISREG`.
#[inline]
pub const fn s_isreg(m: mode_t) -> bool {
    m & libc::S_IFMT == libc::S_IFREG
}
/// `S_ISLNK`.
#[inline]
pub const fn s_islnk(m: mode_t) -> bool {
    m & libc::S_IFMT == libc::S_IFLNK
}
/// `S_ISGITLINK`.
#[inline]
pub const fn s_isgitlink(m: mode_t) -> bool {
    m & libc::S_IFMT == S_IFGITLINK
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

/// `p_open`: `open(2)` with `O_CLOEXEC` added. `mode` is used with
/// `O_CREAT` only (C reads it from the variadic arguments only then).
pub fn open(path: &CStr, flags: c_int, mode: mode_t) -> SysResult<c_int> {
    let mode = if flags & libc::O_CREAT != 0 { mode } else { 0 };
    // SAFETY: `path` is NUL-terminated; `open` has no other precondition.
    let fd = unsafe { libc::open(path.as_ptr(), flags | O_BINARY | libc::O_CLOEXEC, mode) };
    if fd < 0 { Err(last_errno()) } else { Ok(fd) }
}

/// `p_creat`.
pub fn creat(path: &CStr, mode: mode_t) -> SysResult<c_int> {
    open(
        path,
        libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC,
        mode,
    )
}

/// `p_close`.
#[inline]
pub fn close(fd: c_int) -> SysResult<()> {
    // SAFETY: closing any descriptor number is defined (EBADF at worst).
    // The descriptor belongs to the caller, which gives it up.
    check(unsafe { libc::close(fd) })
}

/// `p_read` over raw memory: read up to `len` bytes into `buf`, looping on
/// short reads, `EINTR` and `EAGAIN`, until `len` bytes or end of file.
/// Returns the count, or -1 with `errno` set.
///
/// # Safety
///
/// `buf` is valid for writes of `len` bytes.
pub unsafe fn read_raw(fd: c_int, buf: *mut u8, len: usize) -> isize {
    if isize::try_from(len).is_err() {
        errors::set_errno(libc::EINVAL);
        return -1;
    }
    let mut done = 0usize;
    while done < len {
        // SAFETY: `buf + done` has `len - done` writable bytes (caller).
        let r = unsafe { libc::read(fd, buf.wrapping_add(done).cast::<c_void>(), len - done) };
        if r < 0 {
            let e = errors::errno();
            if e == libc::EINTR || e == libc::EAGAIN {
                continue;
            }
            return -1;
        }
        if r == 0 {
            break;
        }
        done += r as usize;
    }
    done as isize
}

/// `p_read`: read into `buf` until it is full or at end of file.
pub fn read(fd: c_int, buf: &mut [u8]) -> SysResult<usize> {
    // SAFETY: `buf` is writable for its length.
    let r = unsafe { read_raw(fd, buf.as_mut_ptr(), buf.len()) };
    if r < 0 {
        Err(last_errno())
    } else {
        Ok(r as usize)
    }
}

/// `p_write`: write all of `buf`, looping on short writes, `EINTR` and
/// `EAGAIN`; a write of 0 bytes is `EPIPE`.
pub fn write(fd: c_int, buf: &[u8]) -> SysResult<()> {
    let mut rest = buf;
    while !rest.is_empty() {
        // SAFETY: `rest` is readable for its length.
        let r = unsafe { libc::write(fd, rest.as_ptr().cast::<c_void>(), rest.len()) };
        if r < 0 {
            let e = errors::errno();
            if e == libc::EINTR || e == libc::EAGAIN || e == libc::EWOULDBLOCK {
                continue;
            }
            return Err(Errno(e));
        }
        if r == 0 {
            errors::set_errno(libc::EPIPE);
            return Err(Errno(libc::EPIPE));
        }
        rest = &rest[r as usize..];
    }
    Ok(())
}

/// `p_fsync` (counts the call in [`p_fsync__cnt`]).
pub fn fsync(fd: c_int) -> SysResult<()> {
    p_fsync__cnt.fetch_add(1, Ordering::Relaxed);
    // SAFETY: `fsync` of any descriptor number is defined.
    check(unsafe { libc::fsync(fd) })
}

fn stat_with(
    f: unsafe extern "C" fn(*const c_char, *mut Stat) -> c_int,
    path: &CStr,
) -> SysResult<Stat> {
    let mut st = core::mem::MaybeUninit::<Stat>::uninit();
    // SAFETY: `path` is NUL-terminated and `st` writable; `f` is `stat` or
    // `lstat`.
    let r = unsafe { f(path.as_ptr(), st.as_mut_ptr()) };
    if r < 0 {
        return Err(last_errno());
    }
    // SAFETY: the call succeeded, so it filled `st`.
    Ok(unsafe { st.assume_init() })
}

/// `p_stat`.
pub fn stat(path: &CStr) -> SysResult<Stat> {
    stat_with(libc::stat, path)
}

/// `p_lstat` (and `p_lstat_posixly`).
pub fn lstat(path: &CStr) -> SysResult<Stat> {
    stat_with(libc::lstat, path)
}

/// `p_fstat`.
pub fn fstat(fd: c_int) -> SysResult<Stat> {
    let mut st = core::mem::MaybeUninit::<Stat>::uninit();
    // SAFETY: `st` is writable.
    let r = unsafe { libc::fstat(fd, st.as_mut_ptr()) };
    if r < 0 {
        return Err(last_errno());
    }
    // SAFETY: the call succeeded, so it filled `st`.
    Ok(unsafe { st.assume_init() })
}

macro_rules! path_call {
    ($(#[$m:meta])* $name:ident($($arg:ident: $t:ty),*) => $f:ident) => {
        $(#[$m])*
        pub fn $name(path: &CStr $(, $arg: $t)*) -> SysResult<()> {
            // SAFETY: `path` is NUL-terminated; the call has no other
            // precondition.
            check(unsafe { libc::$f(path.as_ptr() $(, $arg)*) })
        }
    };
}

path_call!(/// `p_access`.
    access(mode: c_int) => access);
path_call!(/// `p_mkdir`.
    mkdir(mode: mode_t) => mkdir);
path_call!(/// `p_rmdir`.
    rmdir() => rmdir);
path_call!(/// `p_unlink`.
    unlink() => unlink);
path_call!(/// `p_chmod`.
    chmod(mode: mode_t) => chmod);

/// `p_link`.
pub fn link(from: &CStr, to: &CStr) -> SysResult<()> {
    // SAFETY: both NUL-terminated.
    check(unsafe { libc::link(from.as_ptr(), to.as_ptr()) })
}

/// `p_symlink`.
pub fn symlink(target: &CStr, path: &CStr) -> SysResult<()> {
    // SAFETY: both NUL-terminated.
    check(unsafe { libc::symlink(target.as_ptr(), path.as_ptr()) })
}

/// `p_readlink` into `buf`: the number of bytes written (not
/// NUL-terminated).
pub fn readlink(path: &CStr, buf: &mut [u8]) -> SysResult<usize> {
    // SAFETY: `path` is NUL-terminated, `buf` writable for its length.
    let r = unsafe { libc::readlink(path.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if r < 0 {
        Err(last_errno())
    } else {
        Ok(r as usize)
    }
}

/// `p_utimes`.
pub fn utimes(path: &CStr, times: &[libc::timeval; 2]) -> SysResult<()> {
    // SAFETY: `path` is NUL-terminated and `times` two readable timevals.
    check(unsafe { libc::utimes(path.as_ptr(), times.as_ptr()) })
}

/// `p_rename`: `link` then `unlink` the source (so an existing target is
/// not replaced), else `rename`.
pub fn rename(from: &CStr, to: &CStr) -> SysResult<()> {
    if link(from, to).is_ok() {
        let _ = unlink(from);
        return Ok(());
    }
    // SAFETY: both NUL-terminated.
    check(unsafe { libc::rename(from.as_ptr(), to.as_ptr()) })
}

/// `p_realpath(path, NULL)`: the canonical absolute path, as bytes.
pub fn realpath(path: &CStr) -> SysResult<Vec<u8>> {
    // SAFETY: `path` is NUL-terminated; with a NULL buffer `realpath`
    // allocates the result with `malloc`.
    let p = unsafe { libc::realpath(path.as_ptr(), core::ptr::null_mut()) };
    if p.is_null() {
        return Err(last_errno());
    }
    // SAFETY: `realpath` returned a NUL-terminated string it allocated.
    let v = unsafe { CStr::from_ptr(p) }.to_bytes().to_vec();
    // SAFETY: allocated by the C library's `malloc`, freed once.
    unsafe { libc::free(p.cast()) };
    Ok(v)
}

/// `GIT_PATH_MAX`, the size of the buffers C passes to `realpath`.
pub const GIT_PATH_MAX: usize = 4096;

/// `p_realpath(path, buf)` with a `GIT_PATH_MAX` buffer: the length of the
/// canonical path written at the start of `buf` (NUL-terminated).
pub fn realpath_buf(path: &CStr, buf: &mut [u8; GIT_PATH_MAX]) -> SysResult<usize> {
    const _: () = assert!(GIT_PATH_MAX >= libc::PATH_MAX as usize);
    // SAFETY: `path` is NUL-terminated and `buf` has `PATH_MAX` writable
    // bytes, what `realpath` writes at most.
    let p = unsafe { libc::realpath(path.as_ptr(), buf.as_mut_ptr().cast()) };
    if p.is_null() {
        return Err(last_errno());
    }
    Ok(memchr::memchr(0, buf).unwrap_or(GIT_PATH_MAX))
}

/// `getcwd` as bytes, without a trailing slash.
pub fn getcwd() -> SysResult<Vec<u8>> {
    let mut buf = vec![0u8; libc::PATH_MAX as usize];
    loop {
        // SAFETY: `buf` is writable for its length.
        let p = unsafe { libc::getcwd(buf.as_mut_ptr().cast(), buf.len()) };
        if !p.is_null() {
            let n = memchr::memchr(0, &buf).unwrap_or(buf.len());
            buf.truncate(n);
            return Ok(buf);
        }
        let e = errors::errno();
        if e != libc::ERANGE {
            return Err(Errno(e));
        }
        let n = buf.len() * 2;
        buf.resize(n, 0);
    }
}

// ---------------------------------------------------------------------------
// Memory maps (`unix/map.c`, `map.h`)
// ---------------------------------------------------------------------------

/// `GIT_PROT_READ`.
pub const GIT_PROT_READ: c_int = 0x1;
/// `GIT_PROT_WRITE`.
pub const GIT_PROT_WRITE: c_int = 0x2;
/// `GIT_MAP_SHARED`.
pub const GIT_MAP_SHARED: c_int = 1;
/// `GIT_MAP_PRIVATE`.
pub const GIT_MAP_PRIVATE: c_int = 2;
/// `GIT_MAP_TYPE`.
pub const GIT_MAP_TYPE: c_int = 0xf;
/// `GIT_MAP_FIXED`.
pub const GIT_MAP_FIXED: c_int = 0x10;

/// `git_map`, mirrored (kind 3: C embeds it in `git_mwindow`, the pack and
/// index code): `{ void *data; size_t len; }`.
#[repr(C)]
#[derive(Debug)]
pub struct GitMap {
    /// The mapped bytes.
    pub data: *mut c_void,
    /// Their length.
    pub len: usize,
}

impl Default for GitMap {
    fn default() -> Self {
        GitMap {
            data: core::ptr::null_mut(),
            len: 0,
        }
    }
}

/// `GIT_MMAP_VALIDATE` and the translation of `prot`/`flags` to `mmap`'s:
/// the system call's `(prot, flags)`, or the C's assertion error.
pub fn mmap_args(len: usize, prot: c_int, flags: c_int) -> Result<(c_int, c_int)> {
    crate::git_assert!(len > 0, "out != ((void*)0) && len > 0");
    crate::git_assert!(
        (prot & GIT_PROT_WRITE) != 0 || (prot & GIT_PROT_READ) != 0,
        "(prot & 0x2) || (prot & 0x1)"
    );
    crate::git_assert!((flags & GIT_MAP_FIXED) == 0, "(flags & 0x10) == 0");
    let mut mprot = libc::PROT_READ;
    if prot & GIT_PROT_WRITE != 0 {
        mprot |= libc::PROT_WRITE;
    }
    let mflag = if flags & GIT_MAP_TYPE == GIT_MAP_PRIVATE {
        libc::MAP_PRIVATE
    } else {
        libc::MAP_SHARED
    };
    Ok((mprot, mflag))
}

/// `p_mmap`: map `len` bytes of `fd` at `offset`.
pub fn mmap(len: usize, prot: c_int, flags: c_int, fd: c_int, offset: i64) -> Result<GitMap> {
    let (mprot, mflag) = mmap_args(len, prot, flags)?;
    // SAFETY: a fresh mapping (NULL address hint, no MAP_FIXED) cannot
    // overlap memory Rust uses; the arguments are checked by the kernel.
    let p = unsafe { libc::mmap(core::ptr::null_mut(), len, mprot, mflag, fd, offset) };
    if p.is_null() || p == libc::MAP_FAILED {
        return Err(crate::git_error_set!(
            errors::ErrorClass::OS,
            "failed to mmap. Could not write data"
        ));
    }
    Ok(GitMap { data: p, len })
}

/// `p_munmap`: unmap and reset `map`.
///
/// # Safety
///
/// `map` was filled by [`mmap`] (or `p_mmap`) and not unmapped since, or
/// is empty; no reference into the mapping outlives the call.
pub unsafe fn munmap(map: &mut GitMap) {
    if !map.data.is_null() {
        // SAFETY: the caller's contract: a live mapping of `len` bytes.
        unsafe { libc::munmap(map.data, map.len) };
    }
    map.data = core::ptr::null_mut();
    map.len = 0;
}

/// `git__page_size`.
pub fn page_size() -> Result<usize> {
    // SAFETY: `sysconf` has no preconditions.
    let r = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if r < 0 {
        return Err(crate::git_error_set!(
            errors::ErrorClass::OS,
            "can't determine system page size"
        ));
    }
    Ok(r as usize)
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

/// The C ABI of `posix.c`, `unix/map.c` and `unix/realpath.c`.
pub mod ffi {
    use super::*;
    use crate::util::alloc;
    use crate::util::export::{barrier, cstr};

    /// The body of `p_open` (whose variadic shim is in `rust_shims.c`).
    ///
    /// # Safety
    ///
    /// `path` is a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__p_open(path: *const c_char, flags: c_int, mode: mode_t) -> c_int {
        barrier(|| {
            // SAFETY: the caller passes a NUL-terminated path.
            let Some(path) = (unsafe { cstr(path) }) else {
                errors::set_errno(libc::EFAULT);
                return -1;
            };
            open(path, flags, mode).unwrap_or(-1)
        })
    }

    /// `p_creat`.
    ///
    /// # Safety
    ///
    /// `path` is a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn p_creat(path: *const c_char, mode: mode_t) -> c_int {
        barrier(|| {
            // SAFETY: the caller passes a NUL-terminated path.
            let Some(path) = (unsafe { cstr(path) }) else {
                errors::set_errno(libc::EFAULT);
                return -1;
            };
            creat(path, mode).unwrap_or(-1)
        })
    }

    /// `p_read`.
    ///
    /// # Safety
    ///
    /// `buf` is valid for writes of `cnt` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn p_read(fd: c_int, buf: *mut c_void, cnt: usize) -> isize {
        // SAFETY: forwarded.
        barrier(|| unsafe { read_raw(fd, buf.cast(), cnt) })
    }

    /// `p_write`.
    ///
    /// # Safety
    ///
    /// `buf` is valid for reads of `cnt` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn p_write(fd: c_int, buf: *const c_void, cnt: usize) -> c_int {
        barrier(|| {
            if cnt > isize::MAX as usize {
                // No readable buffer is this large (`write` would fail).
                errors::set_errno(libc::EINVAL);
                return -1;
            }
            // SAFETY: the caller passes `cnt` readable bytes.
            let b = unsafe { crate::util::export::bytes(buf.cast(), cnt) };
            match write(fd, b) {
                Ok(()) => 0,
                Err(_) => -1,
            }
        })
    }

    /// `p_getcwd`: the working directory, with a trailing slash when it
    /// fits (C writes the NUL one past the buffer when the slash just fits:
    /// rust/DEFECTS.md).
    ///
    /// # Safety
    ///
    /// `buffer_out` is valid for writes of `size` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn p_getcwd(buffer_out: *mut c_char, size: usize) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!buffer_out.is_null(), "buffer_out", -1);
            crate::git_assert_arg!(size > 0, "size > 0", -1);
            // SAFETY: the caller passes `size` writable bytes.
            if unsafe { libc::getcwd(buffer_out, size) }.is_null() {
                return -1;
            }
            // SAFETY: `getcwd` wrote a NUL-terminated string into the buffer
            // (whose other bytes may be uninitialized: no slice is made).
            let end = unsafe { CStr::from_ptr(buffer_out) }.to_bytes().len();
            let p = buffer_out.cast::<u8>();
            // `git_fs_path_string_to_dir`, keeping both bytes in the buffer
            // (C wrote the NUL at `buffer_out[size]` when only the slash fit).
            // SAFETY: `end - 1` is within the string when `end > 0`.
            if end > 0 && end + 1 < size && unsafe { p.wrapping_add(end - 1).read() } != b'/' {
                // SAFETY: `end + 1 < size`: both bytes are in the buffer.
                unsafe { p.wrapping_add(end).write(b'/') };
                // SAFETY: as above.
                unsafe { p.wrapping_add(end + 1).write(0) };
            }
            0
        })
    }

    /// `p_rename`.
    ///
    /// # Safety
    ///
    /// `from` and `to` are NUL-terminated strings.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn p_rename(from: *const c_char, to: *const c_char) -> c_int {
        barrier(|| {
            // SAFETY: the caller passes NUL-terminated strings.
            let (Some(from), Some(to)) = (unsafe { cstr(from) }, unsafe { cstr(to) }) else {
                errors::set_errno(libc::EFAULT);
                return -1;
            };
            match rename(from, to) {
                Ok(()) => 0,
                Err(_) => -1,
            }
        })
    }

    /// `p_realpath`: into `resolved` (`PATH_MAX` bytes) or, when NULL, a
    /// string from `git__allocator`.
    ///
    /// # Safety
    ///
    /// `pathname` is a NUL-terminated string; `resolved` is NULL or valid
    /// for writes of `PATH_MAX` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn p_realpath(
        pathname: *const c_char,
        resolved: *mut c_char,
    ) -> *mut c_char {
        barrier(|| {
            // SAFETY: the caller's contract is `realpath`'s.
            let result = unsafe { libc::realpath(pathname, resolved) };
            if result.is_null() || !resolved.is_null() {
                return result;
            }
            // SAFETY: `realpath` allocated a NUL-terminated string.
            let dup = alloc::strdup(unsafe { CStr::from_ptr(result) });
            // SAFETY: allocated by the C library's `malloc`, freed once.
            unsafe { libc::free(result.cast()) };
            dup
        })
    }

    /// `git__page_size`.
    ///
    /// # Safety
    ///
    /// `page_size` is writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__page_size(page_size: *mut usize) -> c_int {
        barrier(|| match super::page_size() {
            Ok(n) => {
                // SAFETY: the caller passes a writable `size_t`.
                unsafe { page_size.write(n) };
                0
            }
            Err(e) => e.raw(),
        })
    }

    /// `git__mmap_alignment`.
    ///
    /// # Safety
    ///
    /// `alignment` is writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git__mmap_alignment(alignment: *mut usize) -> c_int {
        // SAFETY: forwarded.
        unsafe { git__page_size(alignment) }
    }

    /// `p_mmap`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or a writable `git_map`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn p_mmap(
        out: *mut GitMap,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: i64,
    ) -> c_int {
        barrier(|| {
            crate::git_assert!(
                !out.is_null() && len > 0,
                "out != ((void*)0) && len > 0",
                -1
            );
            if let Err(e) = mmap_args(len, prot, flags) {
                return e.raw();
            }
            // SAFETY: the caller passes a writable `git_map`.
            unsafe { out.write(GitMap::default()) };
            match mmap(len, prot, flags, fd, offset) {
                Ok(m) => {
                    // SAFETY: as above.
                    unsafe { out.write(m) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `p_munmap`.
    ///
    /// # Safety
    ///
    /// `map` is NULL or a `git_map` filled by `p_mmap` (or empty).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn p_munmap(map: *mut GitMap) -> c_int {
        barrier(|| {
            crate::git_assert_arg!(!map.is_null(), "map", -1);
            // SAFETY: the caller passes a valid `git_map`, used by no one
            // during the call.
            let map = unsafe { &mut *map };
            // SAFETY: a mapping from `p_mmap` (or empty).
            unsafe { munmap(map) };
            0
        })
    }
}

crate::c_layout_test! {
    headers: ["map.h"],
    types: [
        GitMap => "git_map" { data, len },
    ],
    asserts: [
        "GIT_PROT_READ == 0x1", "GIT_PROT_WRITE == 0x2", "GIT_MAP_SHARED == 1",
        "GIT_MAP_PRIVATE == 2", "GIT_MAP_TYPE == 0xf", "GIT_MAP_FIXED == 0x10",
        "sizeof(p_fsync__cnt) == 8",
    ],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mmap_args_translate() {
        let _g = crate::util::test_support::init();
        assert_eq!(
            mmap_args(1, GIT_PROT_READ, GIT_MAP_PRIVATE),
            Ok((libc::PROT_READ, libc::MAP_PRIVATE))
        );
        assert_eq!(
            mmap_args(1, GIT_PROT_WRITE, GIT_MAP_SHARED),
            Ok((libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED))
        );
        assert_eq!(
            mmap_args(1, GIT_PROT_READ, 0),
            Ok((libc::PROT_READ, libc::MAP_SHARED))
        );
        assert!(mmap_args(0, GIT_PROT_READ, 0).is_err());
        assert_eq!(
            errors::last_message().unwrap(),
            b"unrecoverable internal error: 'out != ((void*)0) && len > 0'"
        );
        assert!(mmap_args(1, 0, 0).is_err());
        assert!(mmap_args(1, GIT_PROT_READ, GIT_MAP_FIXED).is_err());
        assert_eq!(
            errors::last_message().unwrap(),
            b"unrecoverable internal error: '(flags & 0x10) == 0'"
        );
    }

    #[test]
    fn mode_predicates() {
        assert!(s_isdir(0o040755));
        assert!(s_isreg(0o100644));
        assert!(s_islnk(0o120000));
        assert!(s_isgitlink(0o160000));
        assert!(!s_isdir(0o160000));
    }
}
