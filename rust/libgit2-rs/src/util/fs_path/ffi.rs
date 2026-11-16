//! The C ABI of `fs_path.c`: thin shims over [`super`].
//!
//! Many C callers pass a `git_str`'s own buffer as a path argument of the
//! same call (`git_fs_path_dirname_r(&buf, buf.ptr)`, `prettify(&buf,
//! buf.ptr, …)`): such arguments are copied before the string is touched
//! ([`arg`]).

use super::*;
use core::ffi::c_void;
use crate::util::export::{barrier, bytes, cstr, cstr_bytes, status, write_opt};

/// The NUL-terminated string at `p` (empty for NULL), copied when it lies
/// in `buf`'s allocation, so that writing `buf` cannot change it.
///
/// # Safety
///
/// `buf` is NULL or a valid `git_str`; `p` is NULL or a NUL-terminated
/// string readable for `'a`, written during `'a` only through `buf`.
unsafe fn arg<'a>(buf: *const GitStr, p: *const c_char) -> Cow<'a, [u8]> {
    // SAFETY: forwarded.
    let b = unsafe { cstr_bytes(p) };
    if !buf.is_null() && !p.is_null() {
        // SAFETY: the caller passes a valid `git_str`, read for a moment.
        let (start, asize) = unsafe { ((*buf).as_ptr() as usize, (*buf).capacity()) };
        let a = p as usize;
        if asize > 0 && a >= start && a < start + asize {
            return Cow::Owned(b.to_vec());
        }
    }
    Cow::Borrowed(b)
}

/// The `git_str` at `buf`, or `None` for NULL.
///
/// # Safety
///
/// `buf` is NULL or a valid `git_str` that nothing else accesses for `'a`.
unsafe fn s<'a>(buf: *mut GitStr) -> Option<&'a mut GitStr> {
    // SAFETY: forwarded.
    unsafe { buf.as_mut() }
}

/// An optional NUL-terminated string argument (`None` for NULL), copied
/// when it lies in `buf`.
///
/// # Safety
///
/// As [`arg`].
unsafe fn opt_arg<'a>(buf: *const GitStr, p: *const c_char) -> Option<Cow<'a, [u8]>> {
    if p.is_null() {
        None
    } else {
        // SAFETY: forwarded.
        Some(unsafe { arg(buf, p) })
    }
}

/// A path argument as a C string for a system call (`c""` for NULL).
///
/// # Safety
///
/// `p` is NULL or a NUL-terminated string readable for `'a`.
unsafe fn cpath<'a>(p: *const c_char) -> &'a CStr {
    // SAFETY: forwarded.
    unsafe { cstr(p) }.unwrap_or(c"")
}

/// `git_fs_path_dirname`.
///
/// # Safety
///
/// `path` is NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_dirname(path: *const c_char) -> *mut c_char {
    barrier(|| {
        // SAFETY: the caller passes NULL or a NUL-terminated string.
        let p = unsafe { cstr(path) }.map(CStr::to_bytes);
        dirname(p).map_or(core::ptr::null_mut(), GitCStr::into_raw)
    })
}

/// `git_fs_path_basename`.
///
/// # Safety
///
/// `path` is NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_basename(path: *const c_char) -> *mut c_char {
    barrier(|| {
        // SAFETY: the caller passes NULL or a NUL-terminated string.
        let p = unsafe { cstr(path) }.map(CStr::to_bytes);
        basename(p).map_or(core::ptr::null_mut(), GitCStr::into_raw)
    })
}

/// `git_fs_path_dirname_r`.
///
/// # Safety
///
/// `buffer` is NULL or a valid `git_str`; `path` is NULL or a
/// NUL-terminated string (which may lie in `buffer`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_dirname_r(buffer: *mut GitStr, path: *const c_char) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract.
        let p = unsafe { opt_arg(buffer, path) };
        // SAFETY: as above; `p` no longer refers to `buffer`.
        let b = unsafe { s(buffer) };
        match dirname_r(b, p.as_deref()) {
            Ok(n) => n,
            Err(e) => e.raw(),
        }
    })
}

/// `git_fs_path_basename_r`.
///
/// # Safety
///
/// As [`git_fs_path_dirname_r`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_basename_r(buffer: *mut GitStr, path: *const c_char) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract.
        let p = unsafe { opt_arg(buffer, path) };
        // SAFETY: as above.
        let b = unsafe { s(buffer) };
        match basename_r(b, p.as_deref()) {
            Ok(n) => n,
            Err(e) => e.raw(),
        }
    })
}

/// `git_fs_path_basename_offset`.
///
/// # Safety
///
/// `buffer` is NULL or a valid `git_str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_basename_offset(buffer: *mut GitStr) -> usize {
    // SAFETY: the caller passes NULL or a valid `git_str`.
    barrier(|| basename_offset(unsafe { buffer.as_ref() }))
}

/// `git_fs_path_root`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_root(path: *const c_char) -> c_int {
    // SAFETY: the caller passes a NUL-terminated string.
    barrier(|| root(unsafe { cstr_bytes(path) }))
}

/// `git_fs_path_to_dir`.
///
/// # Safety
///
/// `path` is a valid `git_str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_to_dir(path: *mut GitStr) -> c_int {
    barrier(|| {
        // SAFETY: the caller passes a valid `git_str`.
        let Some(p) = (unsafe { s(path) }) else { return -1 };
        status(to_dir(p))
    })
}

/// `git_fs_path_string_to_dir`.
///
/// # Safety
///
/// `path` is a NUL-terminated string, writable for two bytes from its NUL
/// when its length is below `size` (C's contract: the buffer has room for
/// `size + 1` bytes).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_string_to_dir(path: *mut c_char, size: usize) {
    barrier(|| {
        if path.is_null() {
            return;
        }
        // SAFETY: a NUL-terminated string.
        let end = unsafe { CStr::from_ptr(path) }.to_bytes().len();
        // SAFETY: the string and the two bytes from its NUL are writable
        // (the caller's contract), and nothing else uses them.
        let b = unsafe { core::slice::from_raw_parts_mut(path.cast::<u8>(), end + 2) };
        string_to_dir(b, size);
    })
}

/// `git_fs_path_dirlen`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_dirlen(path: *const c_char) -> usize {
    // SAFETY: the caller passes a NUL-terminated string.
    barrier(|| dirlen(unsafe { cstr_bytes(path) }))
}

/// `git__percent_decode`.
///
/// # Safety
///
/// `decoded_out` is a valid `git_str`; `input` a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git__percent_decode(decoded_out: *mut GitStr, input: *const c_char) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!decoded_out.is_null(), "decoded_out", -1);
        crate::git_assert_arg!(!input.is_null(), "input", -1);
        // SAFETY: the caller's contract.
        let i = unsafe { arg(decoded_out, input) };
        // SAFETY: as above.
        let out = unsafe { &mut *decoded_out };
        status(percent_decode(out, &i))
    })
}

/// `git_fs_path_fromurl`.
///
/// # Safety
///
/// `local_path_out` is a valid `git_str`; `file_url` a NUL-terminated
/// string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_fromurl(local_path_out: *mut GitStr, file_url: *const c_char) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!local_path_out.is_null(), "local_path_out", -1);
        crate::git_assert_arg!(!file_url.is_null(), "file_url", -1);
        // SAFETY: the caller's contract.
        let u = unsafe { arg(local_path_out, file_url) };
        // SAFETY: as above.
        status(fromurl(unsafe { &mut *local_path_out }, &u))
    })
}

/// `git_fs_path_is_local_file_url`.
///
/// # Safety
///
/// `file_url` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_is_local_file_url(file_url: *const c_char) -> bool {
    // SAFETY: the caller passes a NUL-terminated string.
    barrier(|| is_local_file_url(unsafe { cstr_bytes(file_url) }))
}

/// `git_fs_path_from_url_or_path`.
///
/// # Safety
///
/// `local_path_out` is a valid `git_str`; `url_or_path` a NUL-terminated
/// string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_from_url_or_path(
    local_path_out: *mut GitStr,
    url_or_path: *const c_char,
) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract.
        let u = unsafe { arg(local_path_out, url_or_path) };
        // SAFETY: as above.
        let Some(out) = (unsafe { s(local_path_out) }) else { return -1 };
        status(from_url_or_path(out, &u))
    })
}

/// A `git_str` reached through the pointer that `git_fs_path_direach`
/// passes to its C callback.
struct RawPath(*mut GitStr);

impl PathCell for RawPath {
    fn path(&mut self) -> &mut GitStr {
        // SAFETY: `RawPath` is made only in `git_fs_path_direach` from the
        // caller's valid `git_str`, which the C reaches only through the
        // callback's argument; the reference ends before the callback runs
        // (the borrow of `self`).
        unsafe { &mut *self.0 }
    }
}

/// `git_fs_path_direach`.
///
/// # Safety
///
/// `path` is a valid `git_str`; `callback` is sound to call with `payload`
/// and `path`, and may change `path` (whose length is restored after).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_direach(
    path: *mut GitStr,
    flags: u32,
    callback: Option<unsafe extern "C" fn(*mut c_void, *mut GitStr) -> c_int>,
    payload: *mut c_void,
) -> c_int {
    barrier(|| {
        if path.is_null() {
            return -1;
        }
        let mut p = RawPath(path);
        status(direach(&mut p, flags, &mut |p: &mut RawPath| match callback {
            // SAFETY: the caller's contract for the callback.
            Some(cb) => unsafe { cb(payload, p.0) },
            None => -1,
        }))
    })
}

/// `git_fs_path_walk_up`.
///
/// # Safety
///
/// `path` is a valid `git_str`, `ceiling` NULL or a NUL-terminated string;
/// `cb` is sound to call with `data` and a C string within `path`'s buffer
/// (which it must not change).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_walk_up(
    path: *mut GitStr,
    ceiling: *const c_char,
    cb: Option<unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int>,
    data: *mut c_void,
) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!path.is_null(), "path", -1);
        let Some(cb) = cb else {
            let _ = errors::assert_arg_failed("cb");
            return -1;
        };
        // SAFETY: the caller's contract: a valid `git_str`, and a string.
        let ceiling = unsafe { opt_arg(path, ceiling) };
        // A copy of the path, so that no reference into the buffer lives
        // while the callback reads it and the NULs are written into it.
        // SAFETY: as above, read for a moment.
        let (copy, buf, owned) = unsafe {
            let s = &*path;
            (s.as_bytes().to_vec(), s.as_ptr().cast_mut(), s.capacity() > s.len())
        };
        walk_up(&copy, ceiling.as_deref(), |step| match step {
            // SAFETY: the callback's contract.
            WalkStep::Empty => unsafe { cb(data, c"".as_ptr()) },
            WalkStep::Prefix(n) if owned => {
                // C truncates the buffer in place for the call: `n` is at
                // most `size`, within the allocation.
                // SAFETY: `buf + n` lies in the owned allocation, which only
                // this call and the callback (reading) use.
                let at = unsafe { buf.add(n) };
                // SAFETY: as above.
                let old = unsafe { at.read() };
                // SAFETY: as above.
                unsafe { at.write(0) };
                // SAFETY: the callback's contract; `buf` is NUL-terminated
                // at `n`.
                let r = unsafe { cb(data, buf) };
                // SAFETY: as above.
                unsafe { at.write(old) };
                r
            }
            WalkStep::Prefix(n) => {
                // A borrowed buffer is not written: a terminated copy.
                let c = to_cstring(&copy[..n]);
                // SAFETY: the callback's contract.
                unsafe { cb(data, c.as_ptr()) }
            }
        })
    })
}

/// `git_fs_path_exists`.
///
/// # Safety
///
/// `path` is NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_exists(path: *const c_char) -> bool {
    barrier(|| {
        crate::git_assert_arg!(!path.is_null(), "path", false);
        // SAFETY: a NUL-terminated string.
        exists(unsafe { cpath(path) })
    })
}

/// `git_fs_path_isdir`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_isdir(path: *const c_char) -> bool {
    // SAFETY: a NUL-terminated string (NULL: false).
    barrier(|| !path.is_null() && isdir(unsafe { cpath(path) }))
}

macro_rules! path_pred {
    ($name:ident => $f:ident) => {
        #[doc = concat!("`", stringify!($name), "`.")]
        ///
        /// # Safety
        ///
        /// `path` is NULL or a NUL-terminated string.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(path: *const c_char) -> bool {
            barrier(|| {
                crate::git_assert_arg!(!path.is_null(), "path", false);
                // SAFETY: a NUL-terminated string.
                $f(unsafe { cpath(path) })
            })
        }
    };
}

path_pred!(git_fs_path_isfile => isfile);
path_pred!(git_fs_path_isexecutable => isexecutable);
path_pred!(git_fs_path_islink => islink);

/// `git_fs_path_is_empty_dir`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_is_empty_dir(path: *const c_char) -> bool {
    // SAFETY: a NUL-terminated string (NULL: false).
    barrier(|| !path.is_null() && is_empty_dir(unsafe { cpath(path) }))
}

/// `git_fs_path_set_error`.
///
/// # Safety
///
/// `path` and `action` are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_set_error(
    errno_value: c_int,
    path: *const c_char,
    action: *const c_char,
) -> c_int {
    barrier(|| {
        // SAFETY: the caller passes NUL-terminated strings.
        let (p, a) = unsafe { (cstr_bytes(path), cstr_bytes(action)) };
        set_error(errno_value, p, &String::from_utf8_lossy(a)).raw()
    })
}

/// `git_fs_path_lstat`.
///
/// # Safety
///
/// `path` is a NUL-terminated string; `st` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_lstat(path: *const c_char, st: *mut Stat) -> c_int {
    barrier(|| {
        // SAFETY: a NUL-terminated string.
        match lstat(unsafe { cpath(path) }) {
            Ok(v) => {
                // SAFETY: the caller passes a writable `struct stat`.
                unsafe { write_opt(st, v) };
                0
            }
            Err(e) => e.raw(),
        }
    })
}

macro_rules! contains_fn {
    ($name:ident => $f:ident) => {
        #[doc = concat!("`", stringify!($name), "`.")]
        ///
        /// # Safety
        ///
        /// `dir` is a valid `git_str`; `item` a NUL-terminated string.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(dir: *mut GitStr, item: *const c_char) -> bool {
            barrier(|| {
                // SAFETY: the caller's contract.
                let i = unsafe { arg(dir, item) };
                // SAFETY: as above.
                let Some(d) = (unsafe { s(dir) }) else { return false };
                $f(d, &i)
            })
        }
    };
}

contains_fn!(git_fs_path_contains => contains);
contains_fn!(git_fs_path_contains_dir => contains_dir);
contains_fn!(git_fs_path_contains_file => contains_file);

/// `git_fs_path_common_dirlen`.
///
/// # Safety
///
/// `one` and `two` are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_common_dirlen(one: *const c_char, two: *const c_char) -> usize {
    // SAFETY: the caller passes NUL-terminated strings.
    barrier(|| common_dirlen(unsafe { cstr_bytes(one) }, unsafe { cstr_bytes(two) }))
}

/// `git_fs_path_make_relative`.
///
/// # Safety
///
/// `path` is a valid `git_str`; `parent` a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_make_relative(path: *mut GitStr, parent: *const c_char) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract.
        let p = unsafe { arg(path, parent) };
        // SAFETY: as above.
        let Some(b) = (unsafe { s(path) }) else { return -1 };
        status(make_relative(b, &p))
    })
}

/// `git_fs_path_join_unrooted`.
///
/// # Safety
///
/// `path_out` is a valid `git_str`; `path` a NUL-terminated string, `base`
/// NULL or one; `root_at` NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_join_unrooted(
    path_out: *mut GitStr,
    path: *const c_char,
    base: *const c_char,
    root_at: *mut isize,
) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!path_out.is_null(), "path_out", -1);
        crate::git_assert_arg!(!path.is_null(), "path", -1);
        // SAFETY: the caller's contract.
        let (p, b) = unsafe { (arg(path_out, path), opt_arg(path_out, base)) };
        // SAFETY: as above.
        match join_unrooted(unsafe { &mut *path_out }, &p, b.as_deref()) {
            Ok(r) => {
                // SAFETY: NULL or writable.
                unsafe { write_opt(root_at, r) };
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_fs_path_squash_slashes`.
///
/// # Safety
///
/// `path` is a valid `git_str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_squash_slashes(path: *mut GitStr) {
    barrier(|| {
        // SAFETY: the caller passes a valid `git_str`.
        if let Some(p) = unsafe { s(path) } {
            squash_slashes(p);
        }
    })
}

/// `git_fs_path_prettify`.
///
/// # Safety
///
/// `path_out` is a valid `git_str`; `path` a NUL-terminated string, `base`
/// NULL or one (either may lie in `path_out`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_prettify(
    path_out: *mut GitStr,
    path: *const c_char,
    base: *const c_char,
) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!path_out.is_null(), "path_out", -1);
        crate::git_assert_arg!(!path.is_null(), "path", -1);
        // SAFETY: the caller's contract.
        let (p, b) = unsafe { (arg(path_out, path), opt_arg(path_out, base)) };
        let p = to_cstring(&p);
        // SAFETY: as above.
        status(prettify(unsafe { &mut *path_out }, &p, b.as_deref()))
    })
}

/// `git_fs_path_prettify_dir`.
///
/// # Safety
///
/// As [`git_fs_path_prettify`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_prettify_dir(
    path_out: *mut GitStr,
    path: *const c_char,
    base: *const c_char,
) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!path_out.is_null(), "path_out", -1);
        crate::git_assert_arg!(!path.is_null(), "path", -1);
        // SAFETY: the caller's contract.
        let (p, b) = unsafe { (arg(path_out, path), opt_arg(path_out, base)) };
        let p = to_cstring(&p);
        // SAFETY: as above.
        status(prettify_dir(unsafe { &mut *path_out }, &p, b.as_deref()))
    })
}

/// `git_fs_path_find_dir`.
///
/// # Safety
///
/// `dir` is a valid `git_str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_find_dir(dir: *mut GitStr) -> c_int {
    barrier(|| {
        // SAFETY: the caller passes a valid `git_str`.
        let Some(d) = (unsafe { s(dir) }) else { return -1 };
        status(find_dir(d))
    })
}

/// `git_fs_path_resolve_relative`.
///
/// # Safety
///
/// `path` is NULL or a valid `git_str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_resolve_relative(path: *mut GitStr, ceiling: usize) -> c_int {
    barrier(|| {
        // SAFETY: the caller passes NULL or a valid `git_str`.
        let Some(p) = (unsafe { s(path) }) else { return -1 };
        status(resolve_relative(p, ceiling))
    })
}

/// `git_fs_path_apply_relative`: 0, or 1 on failure (C's `||`).
///
/// # Safety
///
/// `target` is a valid `git_str`; `relpath` a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_apply_relative(target: *mut GitStr, relpath: *const c_char) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract.
        let r = unsafe { arg(target, relpath) };
        // SAFETY: as above.
        let Some(t) = (unsafe { s(target) }) else { return 1 };
        apply_relative(t, &r).is_err() as c_int
    })
}

/// `git_fs_path_cmp`.
///
/// # Safety
///
/// `name1` and `name2` are readable for `min(len1, len2) + 1` bytes;
/// `compare` is sound to call on them with that length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_cmp(
    name1: *const c_char,
    len1: usize,
    isdir1: c_int,
    name2: *const c_char,
    len2: usize,
    isdir2: c_int,
    compare: Option<unsafe extern "C" fn(*const c_char, *const c_char, usize) -> c_int>,
) -> c_int {
    barrier(|| {
        let len = len1.min(len2);
        if let Some(f) = compare {
            // SAFETY: the caller's contract.
            let r = unsafe { f(name1, name2, len) };
            if r != 0 {
                return r;
            }
        }
        // SAFETY: the caller passes `len + 1` readable bytes of each (C
        // reads the byte after the common length).
        let (a, b) = unsafe { (bytes(name1, len + 1), bytes(name2, len + 1)) };
        cmp(a, len, isdir1 != 0, b, len, isdir2 != 0, |_, _, _| 0)
    })
}

/// `git_fs_path_has_non_ascii`.
///
/// # Safety
///
/// `path` is readable for `pathlen` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_has_non_ascii(path: *const c_char, pathlen: usize) -> bool {
    // SAFETY: the caller passes `pathlen` readable bytes.
    barrier(|| has_non_ascii(unsafe { bytes(path, pathlen) }))
}

/// `git_fs_path_does_decompose_unicode`.
///
/// # Safety
///
/// None (the argument is not read).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_does_decompose_unicode(_root: *const c_char) -> bool {
    false
}

/// `git_fs_path_diriter_init`.
///
/// # Safety
///
/// `diriter` is a writable `git_fs_path_diriter` (what it held is not
/// released); `path` a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_diriter_init(
    diriter: *mut GitFsPathDiriter,
    path: *const c_char,
    flags: c_uint,
) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!diriter.is_null(), "diriter", -1);
        crate::git_assert_arg!(!path.is_null(), "path", -1);
        // SAFETY: the caller's contract.
        let p = unsafe { cstr_bytes(path) };
        // SAFETY: a writable diriter, whose old contents are overwritten
        // without being read (C's memset).
        unsafe { diriter.write(GitFsPathDiriter::new()) };
        // SAFETY: as above, now initialized.
        status(unsafe { &mut *diriter }.init(p, flags))
    })
}

/// `git_fs_path_diriter_next`.
///
/// # Safety
///
/// `diriter` is NULL or a `git_fs_path_diriter` set up by
/// `GIT_FS_PATH_DIRITER_INIT` or [`git_fs_path_diriter_init`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_diriter_next(diriter: *mut GitFsPathDiriter) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!diriter.is_null(), "diriter", -1);
        // SAFETY: the caller's contract.
        status(unsafe { &mut *diriter }.next())
    })
}

/// `git_fs_path_diriter_filename`.
///
/// # Safety
///
/// `out` and `out_len` are writable; `diriter` as in
/// [`git_fs_path_diriter_next`]. The name stays valid until the next call
/// on `diriter`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_diriter_filename(
    out: *mut *const c_char,
    out_len: *mut usize,
    diriter: *mut GitFsPathDiriter,
) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!out.is_null(), "out", -1);
        crate::git_assert_arg!(!out_len.is_null(), "out_len", -1);
        crate::git_assert_arg!(!diriter.is_null(), "diriter", -1);
        // SAFETY: the caller's contract.
        match unsafe { &*diriter }.filename() {
            Ok(name) => {
                // SAFETY: writable out-parameters.
                unsafe {
                    out.write(name.as_ptr().cast());
                    out_len.write(name.len());
                }
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_fs_path_diriter_fullpath`.
///
/// # Safety
///
/// As [`git_fs_path_diriter_filename`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_diriter_fullpath(
    out: *mut *const c_char,
    out_len: *mut usize,
    diriter: *mut GitFsPathDiriter,
) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!out.is_null(), "out", -1);
        crate::git_assert_arg!(!out_len.is_null(), "out_len", -1);
        crate::git_assert_arg!(!diriter.is_null(), "diriter", -1);
        // SAFETY: the caller's contract.
        let d = unsafe { &*diriter };
        // SAFETY: writable out-parameters; C's `path.ptr`.
        unsafe {
            out.write(d.path.as_ptr());
            out_len.write(d.path.len());
        }
        0
    })
}

/// `git_fs_path_diriter_stat`.
///
/// # Safety
///
/// `out` is writable; `diriter` as in [`git_fs_path_diriter_next`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_diriter_stat(out: *mut Stat, diriter: *mut GitFsPathDiriter) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!out.is_null(), "out", -1);
        crate::git_assert_arg!(!diriter.is_null(), "diriter", -1);
        // SAFETY: the caller's contract.
        match unsafe { &*diriter }.stat() {
            Ok(st) => {
                // SAFETY: a writable `struct stat`.
                unsafe { out.write(st) };
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_fs_path_diriter_free`.
///
/// # Safety
///
/// `diriter` is NULL or as in [`git_fs_path_diriter_next`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_diriter_free(diriter: *mut GitFsPathDiriter) {
    barrier(|| {
        // SAFETY: the caller's contract.
        if let Some(d) = unsafe { diriter.as_mut() } {
            d.free();
        }
    })
}

/// `git_fs_path_dirload`.
///
/// # Safety
///
/// `contents` is a valid `git_vector` (of `char *` from `git__allocator`);
/// `path` a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_dirload(
    contents: *mut GitVector,
    path: *const c_char,
    prefix_len: usize,
    flags: u32,
) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!contents.is_null(), "contents", -1);
        crate::git_assert_arg!(!path.is_null(), "path", -1);
        // SAFETY: the caller's contract.
        let (v, p) = unsafe { (&mut *contents, cstr_bytes(path)) };
        status(dirload(v, p, prefix_len, flags))
    })
}

/// `git_str`'s fields, read without forming a `GitStr` (the inline
/// `git_fs_path_is_valid` passes a `size` of `SIZE_MAX`, which is not a
/// valid `GitStr`).
#[repr(C)]
struct RawStr {
    ptr: *const c_char,
    asize: usize,
    size: usize,
}

const _: () = assert!(core::mem::size_of::<RawStr>() == core::mem::size_of::<GitStr>());

/// `git_fs_path_str_is_valid_ext`.
///
/// # Safety
///
/// `path` points to a `git_str` whose `ptr` is readable for `size` bytes,
/// or is a NUL-terminated string when `size` is `SIZE_MAX`; the callbacks
/// are NULL or sound to call with `payload`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_str_is_valid_ext(
    path: *const GitStr,
    flags: c_uint,
    validate_char_cb: Option<unsafe extern "C" fn(c_char, *mut c_void) -> bool>,
    validate_component_cb: Option<unsafe extern "C" fn(*const c_char, usize, *mut c_void) -> bool>,
    _validate_length_cb: Option<unsafe extern "C" fn(*const c_char, usize, usize) -> bool>,
    payload: *mut c_void,
) -> bool {
    barrier(|| {
        if flags == 0 {
            return true;
        }
        if path.is_null() {
            return false;
        }
        // SAFETY: the caller passes a `git_str`-shaped value.
        let raw = unsafe { &*path.cast::<RawStr>() };
        let (bytes, sized) = if raw.size == usize::MAX {
            // SAFETY: a NUL-terminated string (the inline wrappers).
            (unsafe { cstr_bytes(raw.ptr) }, false)
        } else {
            // SAFETY: `size` readable bytes.
            (unsafe { bytes(raw.ptr, raw.size) }, true)
        };
        let mut cc = |c: u8| match validate_char_cb {
            // SAFETY: the callback's contract.
            Some(f) => unsafe { f(c as c_char, payload) },
            None => true,
        };
        let mut comp = |c: &[u8]| match validate_component_cb {
            // SAFETY: the callback's contract; `c` lies in the caller's path.
            Some(f) => unsafe { f(c.as_ptr().cast(), c.len(), payload) },
            None => true,
        };
        str_is_valid_ext(
            bytes,
            sized,
            flags,
            ValidateCallbacks {
                char_cb: validate_char_cb.is_some().then_some(&mut cc as &mut dyn FnMut(u8) -> bool),
                component_cb: validate_component_cb
                    .is_some()
                    .then_some(&mut comp as &mut dyn FnMut(&[u8]) -> bool),
            },
        )
    })
}

/// `git_fs_path_validate_str_length_with_suffix`: always 0 on Unix.
///
/// # Safety
///
/// None (the arguments are not read).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_validate_str_length_with_suffix(
    _path: *mut GitStr,
    _suffix_len: usize,
) -> c_int {
    0
}

/// `git_fs_path_normalize_slashes`.
///
/// # Safety
///
/// `out` is a valid `git_str`; `path` a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_normalize_slashes(out: *mut GitStr, path: *const c_char) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract.
        let p = unsafe { arg(out, path) };
        // SAFETY: as above.
        let Some(o) = (unsafe { s(out) }) else { return -1 };
        status(normalize_slashes(o, &p))
    })
}

/// `git_fs_path_supports_symlinks`.
///
/// # Safety
///
/// `dir` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_supports_symlinks(dir: *const c_char) -> bool {
    // SAFETY: a NUL-terminated string.
    barrier(|| supports_symlinks(unsafe { cstr_bytes(dir) }))
}

/// `git_fs_path__set_owner`.
///
/// # Safety
///
/// None.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path__set_owner(owner: c_uint) {
    barrier(|| set_owner(owner))
}

/// `git_fs_path_owner_is`.
///
/// # Safety
///
/// `out` is writable; `path` a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_owner_is(out: *mut bool, path: *const c_char, owner_type: c_uint) -> c_int {
    barrier(|| {
        // SAFETY: a NUL-terminated string.
        match owner_is(unsafe { cpath(path) }, owner_type) {
            Ok(v) => {
                // SAFETY: a writable `bool`.
                unsafe { write_opt(out, v) };
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_fs_path_owner_is_current_user`.
///
/// # Safety
///
/// As [`git_fs_path_owner_is`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_owner_is_current_user(out: *mut bool, path: *const c_char) -> c_int {
    // SAFETY: forwarded.
    unsafe { git_fs_path_owner_is(out, path, GIT_FS_PATH_OWNER_CURRENT_USER) }
}

/// `git_fs_path_owner_is_system`.
///
/// # Safety
///
/// As [`git_fs_path_owner_is`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_owner_is_system(out: *mut bool, path: *const c_char) -> c_int {
    // SAFETY: forwarded.
    unsafe { git_fs_path_owner_is(out, path, GIT_FS_PATH_OWNER_ADMINISTRATOR) }
}

/// `git_fs_path_find_executable`.
///
/// # Safety
///
/// `fullpath` is a valid `git_str`; `executable` a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_find_executable(fullpath: *mut GitStr, executable: *const c_char) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract.
        let e = unsafe { arg(fullpath, executable) };
        // SAFETY: as above.
        let Some(f) = (unsafe { s(fullpath) }) else { return -1 };
        status(find_executable(f, &e))
    })
}
