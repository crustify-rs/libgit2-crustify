//! The C ABI of `futils.c`: thin shims over [`super`].

use super::*;
use crate::util::cabi::{CStrArg, Out};
use crate::util::export::{barrier, status};
use crate::util::hashmap::{CStrKey, GitHashSetStr};
use crate::util::pool::GitPool;
use core::ptr::NonNull;

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
pub unsafe extern "C" fn git_futils_mkpath2file(path: CStrArg<'_>, mode: mode_t) -> c_int {
    barrier(|| status(mkpath2file(path.to_bytes(), mode)))
}

/// `git_futils_mktmp`.
///
/// # Safety
///
/// `path_out` is NULL (-1) or a valid `git_str` that nothing else accesses
/// during the call; `filename` NULL (`""`) or a NUL-terminated string that
/// nothing but the call writes (it may lie in `path_out`'s buffer).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_mktmp(
    path_out: *mut GitStr,
    filename: CStrArg<'_>,
    mode: mode_t,
) -> c_int {
    barrier(|| {
        // Copied first: it may lie in `path_out`'s buffer, which `mktmp`
        // rewrites.
        let f = filename.to_bytes().to_vec();
        // SAFETY: `path_out` is NULL or a valid `git_str` that nothing else
        // accesses during the call (this export's contract); `filename`
        // is not read again.
        let Some(out) = (unsafe { path_out.as_mut() }) else {
            return -1;
        };
        fd_status(mktmp(out, &f, mode))
    })
}

/// `git_futils_creat_withpath`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_creat_withpath(
    path: CStrArg<'_>,
    dirmode: mode_t,
    mode: mode_t,
) -> c_int {
    barrier(|| fd_status(creat_withpath(path.or_empty(), dirmode, mode)))
}

/// `git_futils_creat_locked`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_creat_locked(path: CStrArg<'_>, mode: mode_t) -> c_int {
    barrier(|| fd_status(creat_locked(path.or_empty(), mode)))
}

/// `git_futils_creat_locked_withpath`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_creat_locked_withpath(
    path: CStrArg<'_>,
    dirmode: mode_t,
    mode: mode_t,
) -> c_int {
    barrier(|| fd_status(creat_locked_withpath(path.or_empty(), dirmode, mode)))
}

/// `git_futils_open_ro`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_open_ro(path: CStrArg<'_>) -> c_int {
    barrier(|| fd_status(open_ro(path.or_empty())))
}

/// `git_futils_truncate`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_truncate(path: CStrArg<'_>, mode: c_int) -> c_int {
    barrier(|| status(truncate(path.or_empty(), mode)))
}

/// `git_futils_filesize`.
///
/// # Safety
///
/// `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_filesize(mut out: Out<'_, u64>, fd: c_int) -> c_int {
    barrier(|| match filesize(fd) {
        Ok(n) => {
            out.write(n);
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
/// `buf` is NULL (-1) or a valid `git_str` that nothing else accesses
/// during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_readbuffer_fd(
    buf: Option<&mut GitStr>,
    fd: c_int,
    len: usize,
) -> c_int {
    barrier(|| {
        let Some(b) = buf else {
            return -1;
        };
        status(readbuffer_fd(b, fd, len))
    })
}

/// `git_futils_readbuffer_fd_full`.
///
/// # Safety
///
/// As [`git_futils_readbuffer_fd`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_readbuffer_fd_full(
    buf: Option<&mut GitStr>,
    fd: c_int,
) -> c_int {
    barrier(|| {
        let Some(b) = buf else {
            return -1;
        };
        status(readbuffer_fd_full(b, fd))
    })
}

/// `git_futils_readbuffer_updated`.
///
/// # Safety
///
/// `out` is NULL (an assertion error) or a valid `git_str`; `path` NULL or
/// a NUL-terminated string that does not lie in `out`'s buffer (which the
/// call frees); `checksum` NULL or 32 readable and writable bytes;
/// `updated` NULL or a writable `int`. Nothing else accesses any of them
/// during the call, and none overlaps another.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_readbuffer_updated(
    out: Option<&mut GitStr>,
    path: CStrArg<'_>,
    checksum: Option<&mut [u8; crate::util::hash::SHA256_SIZE]>,
    updated: Option<&mut c_int>,
) -> c_int {
    barrier(|| {
        let out = crate::git_arg!(out, "out", -1);
        let Some(p) = path.get().filter(|p| !p.is_empty()) else {
            let _ = errors::assert_arg_failed("path && *path");
            return -1;
        };
        status(readbuffer_updated(out, p, checksum, updated))
    })
}

/// `git_futils_readbuffer`.
///
/// # Safety
///
/// `buf` and `path` as in [`git_futils_readbuffer_updated`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_readbuffer(
    buf: Option<&mut GitStr>,
    path: CStrArg<'_>,
) -> c_int {
    // SAFETY: `buf` and `path` are this export's arguments, under the same
    // contract; `None` is a valid `checksum` and `updated`.
    unsafe { git_futils_readbuffer_updated(buf, path, None, None) }
}

/// `git_futils_writebuffer`.
///
/// # Safety
///
/// `buf` is NULL (no data) or a valid `git_str`, and `path` NULL (`""`) or
/// a NUL-terminated string; nothing writes either during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_writebuffer(
    buf: Option<&GitStr>,
    path: CStrArg<'_>,
    flags: c_int,
    mode: mode_t,
) -> c_int {
    barrier(|| {
        let data = buf.map_or(&[][..], GitStr::as_bytes);
        status(writebuffer(data, path.or_empty(), flags, mode))
    })
}

/// `git_futils_mv_withpath`.
///
/// # Safety
///
/// `from` and `to` are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_mv_withpath(
    from: CStrArg<'_>,
    to: CStrArg<'_>,
    dirmode: mode_t,
) -> c_int {
    barrier(|| status(mv_withpath(from.or_empty(), to.or_empty(), dirmode)))
}

/// `git_futils_mmap_ro`.
///
/// # Safety
///
/// `out` is NULL (an assertion error) or valid for writes of a `git_map`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_mmap_ro(
    out: Out<'_, GitMap>,
    fd: c_int,
    begin: i64,
    len: usize,
) -> c_int {
    // SAFETY: `out` is the `Out` this export received; `p_mmap` requires
    // nothing more of it.
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
/// `out` as in [`git_futils_mmap_ro`]; `path` NULL (`""`) or a
/// NUL-terminated string, unwritten during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_mmap_ro_file(out: Out<'_, GitMap>, path: CStrArg<'_>) -> c_int {
    barrier(|| {
        let fd = match open_ro_owned(path.or_empty()) {
            Ok(fd) => fd,
            Err(e) => return e.raw(),
        };
        match filesize(fd.as_raw_fd()) {
            // SAFETY: `out` is the `Out` this export received, all that
            // `git_futils_mmap_ro` requires.
            Ok(len) => unsafe { git_futils_mmap_ro(out, fd.as_raw_fd(), 0, len as usize) },
            Err(e) => e.raw(),
        }
        // `fd` is closed here, after the mapping is made.
    })
}

/// `git_futils_mmap_free`.
///
/// # Safety
///
/// `out` is as `p_munmap`'s `map`: NULL, or a `git_map` filled by `p_mmap`
/// (or empty) that nothing else accesses during the call, into whose
/// mapping no reference outlives it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_mmap_free(out: Option<&mut GitMap>) {
    // SAFETY: `out` is this export's argument, under `p_munmap`'s contract.
    unsafe { crate::util::posix::ffi::p_munmap(out) };
}

/// C's mkdir cache: the caller's `git_pool` and `git_hashset_str`, through
/// their Rust ports (`GitPool`, `GitHashSetStr`).
///
/// Invariant: made only by [`git_futils_mkdir_relative`]'s export from
/// C's options, for that call: `pool` and `set` are each NULL or the
/// caller's live object, used by nothing else during the call, and the
/// pool outlives the set's entries (that export's contract).
struct CCache {
    pool: *mut GitPool,
    set: *mut GitHashSetStr<'static>,
}

impl MkdirCache for CCache {
    fn contains(&mut self, path: &CStr) -> bool {
        // SAFETY: `set` is NULL or the caller's live set, which nothing else
        // uses during the call (`git_futils_mkdir_relative`'s contract).
        let Some(set) = (unsafe { self.set.as_ref() }) else {
            return false;
        };
        // SAFETY: the key is only looked up, not stored: `path` outlives the
        // lookup, whatever lifetime the set's type names.
        let key = unsafe { CStrKey::from_ptr(NonNull::from(path).cast()) };
        set.contains(&key)
    }

    fn add(&mut self, buf: &[u8]) -> Result<()> {
        // SAFETY: NULL or the caller's live pool and set, used by nothing
        // else during the call (the contract).
        let (Some(pool), Some(set)) = (unsafe { self.pool.as_mut() }, unsafe { self.set.as_mut() })
        else {
            return Ok(());
        };
        let p = pool.malloc(buf.len())?;
        // SAFETY: `p` is a fresh pool allocation of `buf.len()` bytes.
        unsafe { core::ptr::copy_nonoverlapping(buf.as_ptr(), p.as_ptr(), buf.len()) };
        // SAFETY: `p` is a C string (`buf` ends with a NUL) that lives as
        // long as the pool, which outlives the set (the contract).
        let key = unsafe { CStrKey::from_ptr(p.cast()) };
        set.add(key).map(|_| ())
    }
}

/// `git_futils_mkdir_relative`.
///
/// # Safety
///
/// `relative_path` is a NUL-terminated string, `base` NULL or one; `opts`
/// is NULL or a valid `struct git_futils_mkdir_options`, used by nothing
/// else during the call, whose
/// `cache_pathset` is NULL or a live `git_hashset_str`, and `cache_pool`
/// NULL or a live `git_pool` (of 1-byte items) that outlives the set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_mkdir_relative(
    relative_path: CStrArg<'_>,
    base: CStrArg<'_>,
    mode: mode_t,
    flags: u32,
    opts: Option<&mut MkdirOptions>,
) -> c_int {
    barrier(|| {
        let rel = relative_path.to_bytes();
        let base = base.get().map(CStr::to_bytes);
        match opts {
            None => status(mkdir_relative(
                rel,
                base,
                mode,
                flags,
                &mut MkdirOpts::default(),
            )),
            Some(o) => {
                let mut cache = CCache {
                    pool: o.cache_pool.cast(),
                    set: o.cache_pathset.cast(),
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
pub unsafe extern "C" fn git_futils_mkdir(path: CStrArg<'_>, mode: mode_t, flags: u32) -> c_int {
    barrier(|| status(mkdir(path.to_bytes(), mode, flags)))
}

/// `git_futils_mkdir_r`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_mkdir_r(path: CStrArg<'_>, mode: mode_t) -> c_int {
    barrier(|| status(mkdir_r(path.to_bytes(), mode)))
}

/// `git_futils_rmdir_r`.
///
/// # Safety
///
/// `path` is a NUL-terminated string, `base` NULL or one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_rmdir_r(
    path: CStrArg<'_>,
    base: CStrArg<'_>,
    flags: u32,
) -> c_int {
    barrier(|| {
        let p = path.to_bytes();
        let b = base.get().map(CStr::to_bytes);
        status(rmdir_r(p, b, flags))
    })
}

/// `git_futils_fake_symlink`.
///
/// # Safety
///
/// `target` and `path` are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_fake_symlink(target: CStrArg<'_>, path: CStrArg<'_>) -> c_int {
    barrier(|| status(fake_symlink(target.to_bytes(), path.or_empty())))
}

/// `git_futils_cp`.
///
/// # Safety
///
/// `from` and `to` are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_cp(
    from: CStrArg<'_>,
    to: CStrArg<'_>,
    filemode: mode_t,
) -> c_int {
    barrier(|| status(cp(from.or_empty(), to.or_empty(), filemode)))
}

/// `git_futils_touch`.
///
/// # Safety
///
/// `path` is a NUL-terminated string; `when` NULL or a readable `time_t`;
/// nothing writes either during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_touch(path: CStrArg<'_>, when: Option<&libc::time_t>) -> c_int {
    barrier(|| status(touch(path.or_empty(), when.copied())))
}

/// `git_futils_cp_r`.
///
/// # Safety
///
/// `from` and `to` are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_cp_r(
    from: CStrArg<'_>,
    to: CStrArg<'_>,
    flags: u32,
    dirmode: mode_t,
) -> c_int {
    barrier(|| {
        let from = from.to_bytes();
        let to = to.to_bytes();
        status(cp_r(from, to, flags, dirmode))
    })
}

/// `git_futils_filestamp_check`.
///
/// # Safety
///
/// `stamp` is NULL or a valid `git_futils_filestamp` that nothing else
/// accesses during the call; `path` a NUL-terminated string, unwritten
/// during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_filestamp_check(
    stamp: Option<&mut Filestamp>,
    path: CStrArg<'_>,
) -> c_int {
    barrier(|| filestamp_check(stamp, path.or_empty()))
}

/// `git_futils_filestamp_set`.
///
/// # Safety
///
/// `target` is NULL (nothing is written) or a writable stamp; `source`
/// NULL or a readable one (it may equal `target`: it stays a raw pointer,
/// read before `target` is written).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_filestamp_set(
    mut target: Out<'_, Filestamp>,
    source: *const Filestamp,
) {
    barrier(|| {
        // SAFETY: `source` is NULL or a readable stamp (this export's
        // contract); the reference ends with the copy, before `target`
        // (which may be the same stamp) is written.
        let src = unsafe { source.as_ref() }.copied();
        target.write(src.unwrap_or_default());
    })
}

/// `git_futils_filestamp_set_from_stat`.
///
/// # Safety
///
/// `stamp` is NULL (nothing is written) or a writable stamp; `st` NULL or
/// a readable `struct stat` that nothing writes during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_filestamp_set_from_stat(
    mut stamp: Out<'_, Filestamp>,
    st: Option<&Stat>,
) {
    barrier(|| stamp.write(st.map(Filestamp::from_stat).unwrap_or_default()))
}

/// `git_futils_fsync_dir`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_fsync_dir(path: CStrArg<'_>) -> c_int {
    barrier(|| status(fsync_dir(path.or_empty())))
}

/// `git_futils_fsync_parent`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_futils_fsync_parent(path: CStrArg<'_>) -> c_int {
    barrier(|| status(fsync_parent(path.to_bytes())))
}
