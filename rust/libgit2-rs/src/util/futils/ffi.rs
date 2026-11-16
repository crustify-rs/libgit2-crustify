//! The C ABI of `futils.c`: thin shims over [`super`].

use super::*;
use crate::sys::hashmap_str;
use crate::util::export::{barrier, cstr, cstr_bytes, status, write_opt};

/// A path argument as a C string (`c""` for NULL, which C would crash on).
///
/// # Safety
///
/// `p` is NULL or a NUL-terminated string readable for `'a`.
unsafe fn cpath<'a>(p: *const c_char) -> &'a CStr {
    // SAFETY: forwarded.
    unsafe { cstr(p) }.unwrap_or(c"")
}

use core::ffi::c_char;

/// An fd-returning result as C's `int`.
fn fd_status(r: Result<c_int>) -> c_int {
    match r {
        Ok(fd) => fd,
        Err(e) => e.raw(),
    }
}

/// `git_futils_mkpath2file`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_mkpath2file(path: *const c_char, mode: mode_t) -> c_int {
    // SAFETY: a NUL-terminated string.
    barrier(|| status(mkpath2file(unsafe { cstr_bytes(path) }, mode)))
}

/// `git_futils_mktmp`.
///
/// # Safety
///
/// `path_out` is a valid `git_str`; `filename` a NUL-terminated string
/// (which may lie in `path_out`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_mktmp(path_out: *mut GitStr, filename: *const c_char, mode: mode_t) -> c_int {
    barrier(|| {
        // SAFETY: a NUL-terminated string, copied since it may lie in
        // `path_out` (`git_fs_path_does_decompose_unicode` passes it so).
        let f = unsafe { cstr_bytes(filename) }.to_vec();
        // SAFETY: a valid `git_str`.
        let Some(out) = (unsafe { path_out.as_mut() }) else { return -1 };
        fd_status(mktmp(out, &f, mode))
    })
}

/// `git_futils_creat_withpath`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_creat_withpath(path: *const c_char, dirmode: mode_t, mode: mode_t) -> c_int {
    // SAFETY: a NUL-terminated string.
    barrier(|| fd_status(creat_withpath(unsafe { cpath(path) }, dirmode, mode)))
}

/// `git_futils_creat_locked`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_creat_locked(path: *const c_char, mode: mode_t) -> c_int {
    // SAFETY: a NUL-terminated string.
    barrier(|| fd_status(creat_locked(unsafe { cpath(path) }, mode)))
}

/// `git_futils_creat_locked_withpath`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_creat_locked_withpath(
    path: *const c_char,
    dirmode: mode_t,
    mode: mode_t,
) -> c_int {
    // SAFETY: a NUL-terminated string.
    barrier(|| fd_status(creat_locked_withpath(unsafe { cpath(path) }, dirmode, mode)))
}

/// `git_futils_open_ro`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_open_ro(path: *const c_char) -> c_int {
    // SAFETY: a NUL-terminated string.
    barrier(|| fd_status(open_ro(unsafe { cpath(path) })))
}

/// `git_futils_truncate`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_truncate(path: *const c_char, mode: c_int) -> c_int {
    // SAFETY: a NUL-terminated string.
    barrier(|| status(truncate(unsafe { cpath(path) }, mode)))
}

/// `git_futils_filesize`.
///
/// # Safety
///
/// `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_filesize(out: *mut u64, fd: c_int) -> c_int {
    barrier(|| match filesize(fd) {
        Ok(n) => {
            // SAFETY: a writable `uint64_t`.
            unsafe { write_opt(out, n) };
            0
        }
        Err(e) => e.raw(),
    })
}

/// `git_futils_canonical_mode`.
///
/// # Safety
///
/// None.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_canonical_mode(raw_mode: mode_t) -> mode_t {
    barrier(|| canonical_mode(raw_mode) as u32) as mode_t
}

/// `git_futils_readbuffer_fd`.
///
/// # Safety
///
/// `buf` is a valid `git_str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_readbuffer_fd(buf: *mut GitStr, fd: c_int, len: usize) -> c_int {
    barrier(|| {
        // SAFETY: a valid `git_str`.
        let Some(b) = (unsafe { buf.as_mut() }) else { return -1 };
        status(readbuffer_fd(b, fd, len))
    })
}

/// `git_futils_readbuffer_fd_full`.
///
/// # Safety
///
/// `buf` is a valid `git_str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_readbuffer_fd_full(buf: *mut GitStr, fd: c_int) -> c_int {
    barrier(|| {
        // SAFETY: a valid `git_str`.
        let Some(b) = (unsafe { buf.as_mut() }) else { return -1 };
        status(readbuffer_fd_full(b, fd))
    })
}

/// `git_futils_readbuffer_updated`.
///
/// # Safety
///
/// `out` is a valid `git_str`; `path` a NUL-terminated string; `checksum`
/// NULL or 32 readable and writable bytes; `updated` NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_readbuffer_updated(
    out: *mut GitStr,
    path: *const c_char,
    checksum: *mut u8,
    updated: *mut c_int,
) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!out.is_null(), "out", -1);
        // SAFETY: NULL or a NUL-terminated string.
        let p = unsafe { cstr(path) };
        let Some(p) = p.filter(|p| !p.is_empty()) else {
            let _ = errors::assert_arg_failed("path && *path");
            return -1;
        };
        // SAFETY: the caller's contract for the optional arguments; none
        // aliases `out`.
        let (out, checksum, updated) = unsafe {
            (
                &mut *out,
                checksum.cast::<[u8; pending::GIT_HASH_SHA256_SIZE]>().as_mut(),
                updated.as_mut(),
            )
        };
        status(readbuffer_updated(out, p, checksum, updated))
    })
}

/// `git_futils_readbuffer`.
///
/// # Safety
///
/// `buf` is a valid `git_str`; `path` a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_readbuffer(buf: *mut GitStr, path: *const c_char) -> c_int {
    // SAFETY: forwarded.
    unsafe { git_futils_readbuffer_updated(buf, path, core::ptr::null_mut(), core::ptr::null_mut()) }
}

/// `git_futils_writebuffer`.
///
/// # Safety
///
/// `buf` is a valid `git_str`; `path` a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_writebuffer(
    buf: *const GitStr,
    path: *const c_char,
    flags: c_int,
    mode: mode_t,
) -> c_int {
    barrier(|| {
        // SAFETY: a valid `git_str`, only read.
        let data = unsafe { buf.as_ref() }.map_or(&[][..], GitStr::as_bytes);
        // SAFETY: a NUL-terminated string.
        status(writebuffer(data, unsafe { cpath(path) }, flags, mode))
    })
}

/// `git_futils_mv_withpath`.
///
/// # Safety
///
/// `from` and `to` are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_mv_withpath(from: *const c_char, to: *const c_char, dirmode: mode_t) -> c_int {
    // SAFETY: NUL-terminated strings.
    barrier(|| status(mv_withpath(unsafe { cpath(from) }, unsafe { cpath(to) }, dirmode)))
}

/// `git_futils_mmap_ro`.
///
/// # Safety
///
/// `out` is a writable `git_map`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_mmap_ro(out: *mut GitMap, fd: c_int, begin: i64, len: usize) -> c_int {
    // SAFETY: forwarded.
    unsafe {
        crate::util::posix::ffi::p_mmap(
            out,
            len,
            posix::GIT_PROT_READ,
            posix::GIT_MAP_PRIVATE,
            fd,
            begin,
        )
    }
}

/// `git_futils_mmap_ro_file`.
///
/// # Safety
///
/// `out` is a writable `git_map`; `path` a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_mmap_ro_file(out: *mut GitMap, path: *const c_char) -> c_int {
    barrier(|| {
        // SAFETY: a NUL-terminated string.
        let p = unsafe { cpath(path) };
        let fd = match open_ro(p) {
            Ok(fd) => fd,
            Err(e) => return e.raw(),
        };
        let r = match filesize(fd) {
            // SAFETY: forwarded: a writable `git_map`.
            Ok(len) => unsafe { git_futils_mmap_ro(out, fd, 0, len as usize) },
            Err(e) => e.raw(),
        };
        let _ = posix::close(fd);
        r
    })
}

/// `git_futils_mmap_free`.
///
/// # Safety
///
/// `out` is a `git_map` filled by `p_mmap` (or empty).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_mmap_free(out: *mut GitMap) {
    // SAFETY: forwarded.
    unsafe { crate::util::posix::ffi::p_munmap(out) };
}

/// C's mkdir cache: `git_pool` + `git_hashset_str`.
struct CCache {
    pool: *mut c_void,
    set: *mut c_void,
}

impl MkdirCache for CCache {
    fn contains(&mut self, path: &CStr) -> bool {
        // SAFETY: `set` is NULL or the caller's live `git_hashset_str`
        // (`git_futils_mkdir_relative`'s contract); `path` is a C string.
        !self.set.is_null() && unsafe { hashmap_str::git_hashset_str__contains(self.set, path.as_ptr()) } != 0
    }

    fn add(&mut self, buf: &[u8]) -> Result<()> {
        if self.set.is_null() || self.pool.is_null() {
            return Ok(());
        }
        // SAFETY: `pool` is the caller's live `git_pool` of 1-byte items.
        let p = unsafe { pending::git_pool_malloc(self.pool, buf.len()) }.cast::<u8>();
        if p.is_null() {
            return Err(errors::oom());
        }
        // SAFETY: `p` is a fresh pool allocation of `buf.len()` bytes.
        unsafe { core::ptr::copy_nonoverlapping(buf.as_ptr(), p, buf.len()) };
        // SAFETY: `set` is live; `p` is a C string (buf ends with NUL) that
        // lives as long as the pool, which outlives the set by the caller's
        // contract.
        let r = unsafe { hashmap_str::git_hashset_str__add(self.set, p.cast()) };
        ErrorCode::check(r).map(|_| ())
    }
}

/// `git_futils_mkdir_relative`.
///
/// # Safety
///
/// `relative_path` is a NUL-terminated string, `base` NULL or one; `opts`
/// is NULL or a valid `struct git_futils_mkdir_options` whose
/// `cache_pathset` is NULL or a live `git_hashset_str`, and `cache_pool`
/// NULL or a live `git_pool` (of 1-byte items) that outlives the set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_mkdir_relative(
    relative_path: *const c_char,
    base: *const c_char,
    mode: mode_t,
    flags: u32,
    opts: *mut MkdirOptions,
) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract.
        let (rel, base, opts) = unsafe {
            (
                cstr_bytes(relative_path),
                cstr(base).map(CStr::to_bytes),
                opts.as_mut(),
            )
        };
        match opts {
            None => status(mkdir_relative(rel, base, mode, flags, &mut MkdirOpts::default())),
            Some(o) => {
                let mut cache = CCache {
                    pool: o.cache_pool,
                    set: o.cache_pathset,
                };
                let mut ro = MkdirOpts {
                    cache: Some(&mut cache),
                    perfdata: o.perfdata,
                };
                let r = mkdir_relative(rel, base, mode, flags, &mut ro);
                o.perfdata = ro.perfdata;
                status(r)
            }
        }
    })
}

/// `git_futils_mkdir`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_mkdir(path: *const c_char, mode: mode_t, flags: u32) -> c_int {
    // SAFETY: a NUL-terminated string.
    barrier(|| status(mkdir(unsafe { cstr_bytes(path) }, mode, flags)))
}

/// `git_futils_mkdir_r`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_mkdir_r(path: *const c_char, mode: mode_t) -> c_int {
    // SAFETY: a NUL-terminated string.
    barrier(|| status(mkdir_r(unsafe { cstr_bytes(path) }, mode)))
}

/// `git_futils_rmdir_r`.
///
/// # Safety
///
/// `path` is a NUL-terminated string, `base` NULL or one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_rmdir_r(path: *const c_char, base: *const c_char, flags: u32) -> c_int {
    barrier(|| {
        // SAFETY: NUL-terminated strings.
        let (p, b) = unsafe { (cstr_bytes(path), cstr(base).map(CStr::to_bytes)) };
        status(rmdir_r(p, b, flags))
    })
}

/// `git_futils_fake_symlink`.
///
/// # Safety
///
/// `target` and `path` are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_fake_symlink(target: *const c_char, path: *const c_char) -> c_int {
    // SAFETY: NUL-terminated strings.
    barrier(|| status(fake_symlink(unsafe { cstr_bytes(target) }, unsafe { cpath(path) })))
}

/// `git_futils_cp`.
///
/// # Safety
///
/// `from` and `to` are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_cp(from: *const c_char, to: *const c_char, filemode: mode_t) -> c_int {
    // SAFETY: NUL-terminated strings.
    barrier(|| status(cp(unsafe { cpath(from) }, unsafe { cpath(to) }, filemode)))
}

/// `git_futils_touch`.
///
/// # Safety
///
/// `path` is a NUL-terminated string; `when` NULL or readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_touch(path: *const c_char, when: *const libc::time_t) -> c_int {
    barrier(|| {
        // SAFETY: NULL or a readable `time_t`.
        let w = unsafe { when.as_ref() }.copied();
        // SAFETY: a NUL-terminated string.
        status(touch(unsafe { cpath(path) }, w))
    })
}

/// `git_futils_cp_r`.
///
/// # Safety
///
/// `from` and `to` are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_cp_r(from: *const c_char, to: *const c_char, flags: u32, dirmode: mode_t) -> c_int {
    // SAFETY: NUL-terminated strings.
    barrier(|| status(cp_r(unsafe { cstr_bytes(from) }, unsafe { cstr_bytes(to) }, flags, dirmode)))
}

/// `git_futils_filestamp_check`.
///
/// # Safety
///
/// `stamp` is NULL or a valid `git_futils_filestamp`; `path` a
/// NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_filestamp_check(stamp: *mut Filestamp, path: *const c_char) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract.
        let (s, p) = unsafe { (stamp.as_mut(), cpath(path)) };
        filestamp_check(s, p)
    })
}

/// `git_futils_filestamp_set`.
///
/// # Safety
///
/// `target` is writable; `source` NULL or readable (it may equal
/// `target`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_filestamp_set(target: *mut Filestamp, source: *const Filestamp) {
    barrier(|| {
        // SAFETY: NULL or a readable stamp, copied out first.
        let src = unsafe { source.as_ref() }.copied();
        // SAFETY: a writable stamp.
        unsafe { write_opt(target, src.unwrap_or_default()) };
    })
}

/// `git_futils_filestamp_set_from_stat`.
///
/// # Safety
///
/// `stamp` is writable; `st` NULL or a readable `struct stat`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_filestamp_set_from_stat(stamp: *mut Filestamp, st: *const Stat) {
    barrier(|| {
        // SAFETY: NULL or a readable `struct stat`.
        let v = unsafe { st.as_ref() }.map(Filestamp::from_stat).unwrap_or_default();
        // SAFETY: a writable stamp.
        unsafe { write_opt(stamp, v) };
    })
}

/// `git_futils_fsync_dir`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_fsync_dir(path: *const c_char) -> c_int {
    // SAFETY: a NUL-terminated string.
    barrier(|| status(fsync_dir(unsafe { cpath(path) })))
}

/// `git_futils_fsync_parent`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_fsync_parent(path: *const c_char) -> c_int {
    // SAFETY: a NUL-terminated string.
    barrier(|| status(fsync_parent(unsafe { cstr_bytes(path) })))
}
