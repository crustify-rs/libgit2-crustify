//! The C ABI of `fs_path.c`: thin shims over [`super`].
//!
//! Many C callers pass a `git_str`'s own buffer as a path argument of the
//! same call (`git_fs_path_dirname_r(&buf, buf.ptr)`, `prettify(&buf,
//! buf.ptr, …)`): such arguments are copied before the string is touched
//! ([`arg`]).

use super::*;
use crate::util::cabi::{CStrArg, Out};
use crate::util::export::{barrier, bytes, cstr, status};
use core::ffi::c_void;
use core::mem::MaybeUninit;
use core::ptr::NonNull;

/// A string argument of an export that also takes a `git_str`: `None` for
/// NULL, a copy when it lay in the `git_str`'s buffer.
type StrArg<'a> = Option<Cow<'a, [u8]>>;

/// The `git_str` at `buf` (`None` for NULL) and the string arguments
/// `ps` (each `None` for NULL). A string that lies in `buf`'s allocation is
/// copied, so that writing `buf` cannot change it; all are read before the
/// reference to `buf` is made.
///
/// # Safety
///
/// `buf` is NULL or a valid `git_str` that nothing else accesses for `'a`
/// (the strings may point into its buffer, not at the `git_str` itself);
/// each of `ps` is NULL or a NUL-terminated string that is readable for
/// `'a` and written during `'a` only through `buf`.
unsafe fn str_args<'a, const N: usize>(
    buf: *mut GitStr,
    ps: [*const c_char; N],
) -> (Option<&'a mut GitStr>, [StrArg<'a>; N]) {
    // SAFETY: `buf` is NULL or a valid `git_str` (the contract); this
    // shared reference only reads its bounds and ends here.
    let alloc = unsafe { buf.as_ref() }
        .filter(|s| s.capacity() > 0)
        .map(|s| s.as_ptr() as usize..s.as_ptr() as usize + s.capacity());
    let args = ps.map(|p| {
        // SAFETY: `p` is NULL or a NUL-terminated string readable for `'a`
        // (the contract).
        let b = unsafe { cstr(p) }?.to_bytes();
        Some(match &alloc {
            Some(r) if r.contains(&(p as usize)) => Cow::Owned(b.to_vec()),
            // Not in `buf`'s allocation: unwritten for `'a` (a growth of
            // `buf` moves it to a new allocation).
            _ => Cow::Borrowed(b),
        })
    });
    // SAFETY: `buf` is NULL or a valid `git_str` that nothing else accesses
    // for `'a` (the contract); the strings in its buffer are copies now.
    (unsafe { buf.as_mut() }, args)
}

/// `git_fs_path_dirname`.
///
/// # Safety
///
/// `path` is NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_dirname(path: CStrArg<'_>) -> *mut c_char {
    barrier(|| {
        let p = path.get().map(CStr::to_bytes);
        dirname(p).map_or(core::ptr::null_mut(), GitCStr::into_raw)
    })
}

/// `git_fs_path_basename`.
///
/// # Safety
///
/// `path` is NULL or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_basename(path: CStrArg<'_>) -> *mut c_char {
    barrier(|| {
        let p = path.get().map(CStr::to_bytes);
        basename(p).map_or(core::ptr::null_mut(), GitCStr::into_raw)
    })
}

/// `git_fs_path_dirname_r`.
///
/// # Safety
///
/// `buffer` is NULL or a valid `git_str` that nothing else accesses during
/// the call; `path` is NULL or a NUL-terminated string that nothing but
/// the call writes (it may lie in `buffer`'s buffer).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_dirname_r(buffer: *mut GitStr, path: *const c_char) -> c_int {
    barrier(|| {
        // SAFETY: `str_args`' requirements are this export's contract.
        let (b, [p]) = unsafe { str_args(buffer, [path]) };
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
        // SAFETY: `str_args`' requirements are this export's contract.
        let (b, [p]) = unsafe { str_args(buffer, [path]) };
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
/// `buffer` is NULL or a valid `git_str` that nothing writes during the
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_basename_offset(buffer: Option<&GitStr>) -> usize {
    barrier(|| basename_offset(buffer))
}

/// `git_fs_path_root`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_root(path: *const c_char) -> c_int {
    if path.is_null() {
        return -1;
    }
    // Only the bytes C reads: at most the first five, never past the NUL
    // (`root_with`'s contract), so no `strlen`.
    barrier(|| {
        root_with(|i| {
            // SAFETY: `path` is a NUL-terminated string and `i` is at most
            // the index of its NUL.
            unsafe { path.cast::<u8>().wrapping_add(i).read() }
        })
    })
}

/// `git_fs_path_to_dir`.
///
/// # Safety
///
/// `path` is NULL (-1) or a valid `git_str` that nothing else accesses
/// during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_to_dir(path: Option<&mut GitStr>) -> c_int {
    barrier(|| {
        let Some(p) = path else {
            return -1;
        };
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
        // SAFETY: the non-null `path` is a NUL-terminated string (this
        // export's contract) that nothing else accesses during the call;
        // the `CStr` ends with the measurement.
        let end = unsafe { CStr::from_ptr(path) }.to_bytes().len();
        let p = path.cast::<u8>();
        // C's test and writes, with raw writes: the byte after the NUL may
        // be uninitialized (no slice is made over it).
        // SAFETY: `end > 0` (tested first): `end - 1` is a byte of the
        // string measured above.
        if end > 0 && end < size && unsafe { p.wrapping_add(end - 1).read() } != b'/' {
            // SAFETY: the length is below `size`, so the two bytes from the
            // NUL are writable (the contract).
            unsafe { p.wrapping_add(end).write(b'/') };
            // SAFETY: as above.
            unsafe { p.wrapping_add(end + 1).write(0) };
        }
    })
}

/// `git_fs_path_dirlen`.
///
/// # Safety
///
/// `path` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_dirlen(path: CStrArg<'_>) -> usize {
    barrier(|| dirlen(path.to_bytes()))
}

/// `git__percent_decode`.
///
/// # Safety
///
/// `decoded_out` is NULL or a valid `git_str` that nothing else accesses
/// during the call; `input` NULL or a NUL-terminated string that nothing
/// but the call writes (it may lie in `decoded_out`'s buffer).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git__percent_decode(
    decoded_out: *mut GitStr,
    input: *const c_char,
) -> c_int {
    barrier(|| {
        // SAFETY: `str_args`' requirements are this export's contract.
        let (out, [i]) = unsafe { str_args(decoded_out, [input]) };
        let out = crate::git_arg!(out, "decoded_out", -1);
        let i = crate::git_arg!(i, "input", -1);
        status(percent_decode(out, &i))
    })
}

/// `git_fs_path_fromurl`.
///
/// # Safety
///
/// `local_path_out` is NULL or a valid `git_str` that nothing else
/// accesses during the call; `file_url` NULL or a NUL-terminated string
/// that nothing but the call writes (it may lie in `local_path_out`'s
/// buffer).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_fromurl(
    local_path_out: *mut GitStr,
    file_url: *const c_char,
) -> c_int {
    barrier(|| {
        // SAFETY: `str_args`' requirements are this export's contract.
        let (out, [u]) = unsafe { str_args(local_path_out, [file_url]) };
        let out = crate::git_arg!(out, "local_path_out", -1);
        let u = crate::git_arg!(u, "file_url", -1);
        status(fromurl(out, &u))
    })
}

/// `git_fs_path_is_local_file_url`.
///
/// # Safety
///
/// `file_url` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_is_local_file_url(file_url: CStrArg<'_>) -> bool {
    barrier(|| is_local_file_url(file_url.to_bytes()))
}

/// `git_fs_path_from_url_or_path`.
///
/// # Safety
///
/// As [`git_fs_path_fromurl`] (`url_or_path` for `file_url`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_from_url_or_path(
    local_path_out: *mut GitStr,
    url_or_path: *const c_char,
) -> c_int {
    barrier(|| {
        // SAFETY: `str_args`' requirements are this export's contract.
        let (out, [u]) = unsafe { str_args(local_path_out, [url_or_path]) };
        let Some(out) = out else {
            return -1;
        };
        status(from_url_or_path(out, &u.unwrap_or_default()))
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
        status(direach(
            &mut p,
            flags,
            &mut |p: &mut RawPath| match callback {
                // SAFETY: `cb` is sound to call with `payload` and `path`
                // (this export's contract), which `p.0` is; no reference
                // to the `git_str` is live (`RawPath::path`'s ended).
                Some(cb) => unsafe { cb(payload, p.0) },
                None => -1,
            },
        ))
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
        // SAFETY: `str_args`' requirements are this export's contract.
        let (s, [ceiling]) = unsafe { str_args(path, [ceiling]) };
        let Some(s) = s else {
            return -1; // Not reached: `path` was checked above.
        };
        // A copy of the path, so that no reference into the buffer lives
        // while the callback reads it and the NULs are written into it.
        let (copy, buf, owned) = (
            s.as_bytes().to_vec(),
            s.as_ptr().cast_mut(),
            s.capacity() > s.len(),
        );
        walk_up(&copy, ceiling.as_deref(), |step| match step {
            // SAFETY: `cb` is sound to call with `data` and a C string
            // (this export's contract); `c""` is one, for the call.
            WalkStep::Empty => unsafe { cb(data, c"".as_ptr()) },
            WalkStep::Prefix(n) if owned => {
                // C truncates the buffer in place for the call. `n` is at
                // most `size`, and `size < asize` (`owned`), so `at` is in
                // the owned, initialized allocation (byte `size` is the
                // NUL), which no reference covers and which the callback
                // only reads (the contract).
                let at = buf.wrapping_add(n);
                // SAFETY: `at` is in the allocation and initialized (above).
                let old = unsafe { at.read() };
                // SAFETY: `at` is in the allocation, written by nothing
                // else during the call (above).
                unsafe { at.write(0) };
                // SAFETY: `cb` is sound to call with `data` and a C string
                // in `path`'s buffer (the contract); `buf` is one, ended
                // at `n`.
                let r = unsafe { cb(data, buf) };
                // SAFETY: as the write of the NUL.
                unsafe { at.write(old) };
                r
            }
            WalkStep::Prefix(n) => {
                // A borrowed buffer is not written: a terminated copy.
                let c = to_cstring(&copy[..n]);
                // SAFETY: `cb` is sound to call with `data` and a C string
                // (the contract); `c` is one, alive for the call.
                unsafe { cb(data, c.as_ptr()) }
            }
        })
    })
}

/// `git_fs_path_exists`.
///
/// # Safety
///
/// `path` is NULL or a NUL-terminated string, unwritten during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_exists(path: CStrArg<'_>) -> bool {
    barrier(|| exists(crate::git_arg!(path.get(), "path", false)))
}

/// `git_fs_path_isdir`.
///
/// # Safety
///
/// `path` is NULL (false) or a NUL-terminated string, unwritten during the
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_isdir(path: CStrArg<'_>) -> bool {
    barrier(|| path.get().is_some_and(isdir))
}

macro_rules! path_pred {
    ($name:ident => $f:ident) => {
        #[doc = concat!("`", stringify!($name), "`.")]
        ///
        /// # Safety
        ///
        /// `path` is NULL or a NUL-terminated string.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(path: CStrArg<'_>) -> bool {
            barrier(|| $f(crate::git_arg!(path.get(), "path", false)))
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
/// As [`git_fs_path_isdir`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_is_empty_dir(path: CStrArg<'_>) -> bool {
    barrier(|| path.get().is_some_and(is_empty_dir))
}

/// `git_fs_path_set_error`.
///
/// # Safety
///
/// `path` and `action` are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_set_error(
    errno_value: c_int,
    path: CStrArg<'_>,
    action: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        let p = path.to_bytes();
        let a = action.to_bytes();
        set_error_bytes(errno_value, p, a).raw()
    })
}

/// `git_fs_path_lstat`.
///
/// # Safety
///
/// `path` is NULL (`""`) or a NUL-terminated string, unwritten during the
/// call; `st` is NULL (nothing is written) or a writable `struct stat`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_lstat(path: CStrArg<'_>, mut st: Out<'_, Stat>) -> c_int {
    barrier(|| match lstat(path.or_empty()) {
        Ok(v) => {
            st.write(v);
            0
        }
        Err(e) => e.raw(),
    })
}

macro_rules! contains_fn {
    ($name:ident => $f:ident) => {
        #[doc = concat!("`", stringify!($name), "`.")]
        ///
        /// # Safety
        ///
        /// `dir` is NULL (false) or a valid `git_str` that nothing else
        /// accesses during the call; `item` NULL (`""`) or a
        /// NUL-terminated string that nothing but the call writes (it may
        /// lie in `dir`'s buffer).
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(dir: *mut GitStr, item: *const c_char) -> bool {
            barrier(|| {
                // SAFETY: `str_args`' requirements are this export's
                // contract.
                let (d, [i]) = unsafe { str_args(dir, [item]) };
                let Some(d) = d else {
                    return false;
                };
                $f(d, &i.unwrap_or_default())
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
pub unsafe extern "C" fn git_fs_path_common_dirlen(
    one: *const c_char,
    two: *const c_char,
) -> usize {
    let (Some(one), Some(two)) = (NonNull::new(one.cast_mut()), NonNull::new(two.cast_mut()))
    else {
        return 0;
    };
    // SAFETY: the caller passes NUL-terminated strings, unwritten during
    // the call.
    let one = unsafe { RawCStr::new(one) };
    // SAFETY: as above.
    let two = unsafe { RawCStr::new(two) };
    barrier(|| common_dirlen_with(one, two))
}

/// `git_fs_path_make_relative`.
///
/// # Safety
///
/// `path` is NULL (-1) or a valid `git_str` that nothing else accesses
/// during the call; `parent` NULL (`""`) or a NUL-terminated string that
/// nothing but the call writes (it may lie in `path`'s buffer).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_make_relative(
    path: *mut GitStr,
    parent: *const c_char,
) -> c_int {
    barrier(|| {
        // SAFETY: `str_args`' requirements are this export's contract.
        let (b, [p]) = unsafe { str_args(path, [parent]) };
        let Some(b) = b else {
            return -1;
        };
        status(make_relative(b, &p.unwrap_or_default()))
    })
}

/// `git_fs_path_join_unrooted`.
///
/// # Safety
///
/// `path_out` is NULL or a valid `git_str` that nothing else accesses
/// during the call; `path` and `base` NULL or NUL-terminated strings that
/// nothing but the call writes (they may lie in `path_out`'s buffer);
/// `root_at` NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_join_unrooted(
    path_out: *mut GitStr,
    path: *const c_char,
    base: *const c_char,
    mut root_at: Out<'_, isize>,
) -> c_int {
    barrier(|| {
        // SAFETY: `str_args`' requirements are this export's contract.
        let (out, [p, b]) = unsafe { str_args(path_out, [path, base]) };
        let out = crate::git_arg!(out, "path_out", -1);
        let p = crate::git_arg!(p, "path", -1);
        match join_unrooted(out, &p, b.as_deref()) {
            Ok(r) => {
                root_at.write(r);
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
/// `path` is NULL or a valid `git_str` that nothing else accesses during
/// the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_squash_slashes(path: Option<&mut GitStr>) {
    barrier(|| {
        if let Some(p) = path {
            squash_slashes(p);
        }
    })
}

/// `git_fs_path_prettify`.
///
/// # Safety
///
/// As [`git_fs_path_join_unrooted`] (without `root_at`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_prettify(
    path_out: *mut GitStr,
    path: *const c_char,
    base: *const c_char,
) -> c_int {
    barrier(|| {
        // SAFETY: `str_args`' requirements are this export's contract.
        let (out, [p, b]) = unsafe { str_args(path_out, [path, base]) };
        let out = crate::git_arg!(out, "path_out", -1);
        let p = to_cstring(&crate::git_arg!(p, "path", -1));
        status(prettify(out, &p, b.as_deref()))
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
        // SAFETY: `str_args`' requirements are this export's contract.
        let (out, [p, b]) = unsafe { str_args(path_out, [path, base]) };
        let out = crate::git_arg!(out, "path_out", -1);
        let p = to_cstring(&crate::git_arg!(p, "path", -1));
        status(prettify_dir(out, &p, b.as_deref()))
    })
}

/// `git_fs_path_find_dir`.
///
/// # Safety
///
/// `dir` is NULL (-1) or a valid `git_str` that nothing else accesses
/// during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_find_dir(dir: Option<&mut GitStr>) -> c_int {
    barrier(|| {
        let Some(d) = dir else {
            return -1;
        };
        status(find_dir(d))
    })
}

/// `git_fs_path_resolve_relative`.
///
/// # Safety
///
/// `path` is NULL (-1) or a valid `git_str` that nothing else accesses
/// during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_resolve_relative(
    path: Option<&mut GitStr>,
    ceiling: usize,
) -> c_int {
    barrier(|| {
        let Some(p) = path else {
            return -1;
        };
        status(resolve_relative(p, ceiling))
    })
}

/// `git_fs_path_apply_relative`: 0, or 1 on failure (C's `||`).
///
/// # Safety
///
/// `target` is NULL (1) or a valid `git_str` that nothing else accesses
/// during the call; `relpath` NULL (`""`) or a NUL-terminated string that
/// nothing but the call writes (it may lie in `target`'s buffer).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_apply_relative(
    target: *mut GitStr,
    relpath: *const c_char,
) -> c_int {
    barrier(|| {
        // SAFETY: `str_args`' requirements are this export's contract.
        let (t, [r]) = unsafe { str_args(target, [relpath]) };
        let Some(t) = t else {
            return 1;
        };
        apply_relative(t, &r.unwrap_or_default()).is_err() as c_int
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
            // SAFETY: `compare` is sound to call on the names with the
            // common length (this export's contract).
            let r = unsafe { f(name1, name2, len) };
            if r != 0 {
                return r;
            }
        }
        // C reads the byte after the common length of each name.
        // SAFETY: `name1` is readable for `len + 1` bytes (this export's
        // contract): byte `len` is the last of them.
        let mut c1 = unsafe { name1.cast::<u8>().wrapping_add(len).read() };
        // SAFETY: as above.
        let mut c2 = unsafe { name2.cast::<u8>().wrapping_add(len).read() };
        if c1 == 0 && isdir1 != 0 {
            c1 = b'/';
        }
        if c2 == 0 && isdir2 != 0 {
            c2 = b'/';
        }
        (c1 > c2) as c_int - (c1 < c2) as c_int
    })
}

/// `git_fs_path_has_non_ascii`.
///
/// # Safety
///
/// `path` is readable for `pathlen` bytes, which nothing writes during the
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_has_non_ascii(path: *const c_char, pathlen: usize) -> bool {
    barrier(|| {
        // No readable buffer is larger than `isize::MAX` bytes.
        let n = pathlen.min(isize::MAX as usize);
        // SAFETY: `path` is readable for `pathlen >= n` bytes, which
        // nothing writes during the call (this export's contract), and
        // `n <= isize::MAX`.
        has_non_ascii(unsafe { bytes(path, n) })
    })
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
/// `diriter` is NULL or valid for writes of a `git_fs_path_diriter` (what
/// it held is not read or released), which nothing else accesses during
/// the call; `path` NULL or a NUL-terminated string, unwritten during the
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_diriter_init(
    diriter: *mut GitFsPathDiriter,
    path: CStrArg<'_>,
    flags: c_uint,
) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!diriter.is_null(), "diriter", -1);
        let p = crate::git_arg!(path.get(), "path", -1).to_bytes();
        // SAFETY: `diriter` is non-null, aligned and writable for a
        // diriter that nothing else accesses during the call (this
        // export's contract); as `MaybeUninit` its old bytes are neither
        // read nor dropped (C's memset), and `write` returns the reference
        // to the new value.
        let d = unsafe { &mut *diriter.cast::<MaybeUninit<GitFsPathDiriter>>() }
            .write(GitFsPathDiriter::new());
        status(d.init(p, flags))
    })
}

/// `git_fs_path_diriter_next`.
///
/// # Safety
///
/// `diriter` is NULL or a `git_fs_path_diriter` set up by
/// `GIT_FS_PATH_DIRITER_INIT` or [`git_fs_path_diriter_init`], which
/// nothing else accesses during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_diriter_next(diriter: Option<&mut GitFsPathDiriter>) -> c_int {
    barrier(|| status(crate::git_arg!(diriter, "diriter", -1).next()))
}

/// `git_fs_path_diriter_filename`.
///
/// # Safety
///
/// `out` and `out_len` are NULL or writable, and `diriter` NULL or as in
/// [`git_fs_path_diriter_next`] but only read: nothing writes it during
/// the call. The name stays valid until the next call on `diriter`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_diriter_filename(
    mut out: Out<'_, *const c_char>,
    mut out_len: Out<'_, usize>,
    diriter: Option<&GitFsPathDiriter>,
) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!out.is_null(), "out", -1);
        crate::git_assert_arg!(!out_len.is_null(), "out_len", -1);
        let diriter = crate::git_arg!(diriter, "diriter", -1);
        match diriter.filename() {
            Ok(name) => {
                out.write(name.as_ptr().cast());
                out_len.write(name.len());
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
    mut out: Out<'_, *const c_char>,
    mut out_len: Out<'_, usize>,
    diriter: Option<&GitFsPathDiriter>,
) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!out.is_null(), "out", -1);
        crate::git_assert_arg!(!out_len.is_null(), "out_len", -1);
        let d = crate::git_arg!(diriter, "diriter", -1);
        // C's `path.ptr`.
        out.write(d.path.as_ptr());
        out_len.write(d.path.len());
        0
    })
}

/// `git_fs_path_diriter_stat`.
///
/// # Safety
///
/// `out` is NULL or a writable `struct stat`; `diriter` as in
/// [`git_fs_path_diriter_filename`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_diriter_stat(
    mut out: Out<'_, Stat>,
    diriter: Option<&GitFsPathDiriter>,
) -> c_int {
    barrier(|| {
        crate::git_assert_arg!(!out.is_null(), "out", -1);
        match crate::git_arg!(diriter, "diriter", -1).stat() {
            Ok(st) => {
                out.write(st);
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
pub unsafe extern "C" fn git_fs_path_diriter_free(diriter: Option<&mut GitFsPathDiriter>) {
    barrier(|| {
        if let Some(d) = diriter {
            d.free();
        }
    })
}

/// `git_fs_path_dirload`.
///
/// # Safety
///
/// `contents` is NULL or a valid `git_vector` (of `char *` from
/// `git__allocator`) that nothing else accesses during the call; `path`
/// NULL or a NUL-terminated string, unwritten during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_dirload(
    contents: Option<&mut GitVector>,
    path: CStrArg<'_>,
    prefix_len: usize,
    flags: u32,
) -> c_int {
    barrier(|| {
        let v = crate::git_arg!(contents, "contents", -1);
        let p = crate::git_arg!(path.get(), "path", -1).to_bytes();
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
/// `path` is NULL or points to a `git_str` whose `ptr` is readable for
/// `size` bytes, or is a NUL-terminated string when `size` is `SIZE_MAX`;
/// nothing writes either during the call. The callbacks are NULL or sound
/// to call with `payload` (the component callback with a pointer into the
/// path, or to a NUL-terminated copy of its last component).
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
        // SAFETY: the non-null `path` points to a readable `git_str`, which
        // nothing writes during the call (this export's contract), and
        // `RawStr` has its layout (asserted above) with no invalid values.
        let raw = unsafe { &*path.cast::<RawStr>() };
        let mut cc = |c: u8| match validate_char_cb {
            // SAFETY: a non-null `validate_char_cb` is sound to call with
            // any character and `payload` (this export's contract).
            Some(f) => unsafe { f(c as c_char, payload) },
            None => true,
        };
        // Where a sized path ends (`ptr + size`): C's validators read the
        // byte after a component (`path.c`'s `validate_dotgit_ntfs` reads
        // `path[len]`), which inside the path is the `/` or the NUL that
        // ended it, but after the last component of a sized path is
        // `ptr[size]`, which the caller's buffer need not have (a Rust
        // caller's slice). That component goes to the callback as a
        // NUL-terminated copy: what C saw after an owned `git_str`.
        let sized_end = (raw.size != usize::MAX).then(|| (raw.ptr as usize).wrapping_add(raw.size));
        let mut comp = |c: &[u8]| match validate_component_cb {
            Some(f) if sized_end == Some(c.as_ptr() as usize + c.len()) => {
                let mut stack = [0u8; 256];
                let mut heap = Vec::new();
                let buf: &mut [u8] = if c.len() < stack.len() {
                    &mut stack[..=c.len()]
                } else {
                    if heap.try_reserve_exact(c.len() + 1).is_err() {
                        let _ = crate::util::errors::oom();
                        return false;
                    }
                    heap.resize(c.len() + 1, 0);
                    &mut heap
                };
                buf[..c.len()].copy_from_slice(c);
                buf[c.len()] = 0;
                // SAFETY: `f` is sound to call with a component of the path
                // and `payload` (this export's contract); `buf` holds the
                // component and a NUL after it, for the call.
                unsafe { f(buf.as_ptr().cast(), c.len(), payload) }
            }
            // The component is handed over as a pointer into the caller's
            // path (the C's), whose provenance covers the `/` or NUL after
            // it, not as the slice's pointer, which covers only the
            // component.
            // SAFETY: `f` is sound to call with a component of the path and
            // `payload` (this export's contract); `c` lies in the caller's
            // path at that offset and is followed there by its `/` or NUL.
            Some(f) => unsafe {
                f(
                    raw.ptr.wrapping_add(c.as_ptr() as usize - raw.ptr as usize),
                    c.len(),
                    payload,
                )
            },
            None => true,
        };
        let cbs = ValidateCallbacks {
            char_cb: validate_char_cb
                .is_some()
                .then_some(&mut cc as &mut dyn FnMut(u8) -> bool),
            component_cb: validate_component_cb
                .is_some()
                .then_some(&mut comp as &mut dyn FnMut(&[u8]) -> bool),
        };
        if raw.size == usize::MAX {
            // A C string (the inline `git_fs_path_is_valid`), read up to its
            // NUL as C does, without measuring it first.
            let Some(p) = NonNull::new(raw.ptr.cast_mut()) else {
                return false;
            };
            // SAFETY: with `size == SIZE_MAX`, `ptr` is a NUL-terminated
            // string that nothing writes during the call (the contract).
            let p = unsafe { RawCStr::new(p) };
            str_is_valid_with(p, usize::MAX, flags, cbs)
        } else if raw.size > isize::MAX as usize {
            // No readable buffer is this large.
            false
        } else {
            // SAFETY: `ptr` is readable for `size <= isize::MAX` (checked)
            // bytes that nothing writes during the call (the contract).
            let b = unsafe { bytes(raw.ptr, raw.size) };
            str_is_valid_with(b, raw.size, flags, cbs)
        }
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
/// `out` is NULL (-1) or a valid `git_str` that nothing else accesses
/// during the call; `path` NULL (`""`) or a NUL-terminated string that
/// nothing but the call writes (it may lie in `out`'s buffer).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_normalize_slashes(
    out: *mut GitStr,
    path: *const c_char,
) -> c_int {
    barrier(|| {
        // SAFETY: `str_args`' requirements are this export's contract.
        let (o, [p]) = unsafe { str_args(out, [path]) };
        let Some(o) = o else {
            return -1;
        };
        status(normalize_slashes(o, &p.unwrap_or_default()))
    })
}

/// `git_fs_path_supports_symlinks`.
///
/// # Safety
///
/// `dir` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_supports_symlinks(dir: CStrArg<'_>) -> bool {
    barrier(|| supports_symlinks(dir.to_bytes()))
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
/// `out` is NULL (nothing is written) or writable; `path` NULL (`""`) or a
/// NUL-terminated string, unwritten during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_owner_is(
    mut out: Out<'_, bool>,
    path: CStrArg<'_>,
    owner_type: c_uint,
) -> c_int {
    barrier(|| match owner_is(path.or_empty(), owner_type) {
        Ok(v) => {
            out.write(v);
            0
        }
        Err(e) => e.raw(),
    })
}

/// `git_fs_path_owner_is_current_user`.
///
/// # Safety
///
/// As [`git_fs_path_owner_is`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_owner_is_current_user(
    out: Out<'_, bool>,
    path: CStrArg<'_>,
) -> c_int {
    // SAFETY: `out` and `path` are this export's arguments, under
    // `git_fs_path_owner_is`'s contract.
    unsafe { git_fs_path_owner_is(out, path, GIT_FS_PATH_OWNER_CURRENT_USER) }
}

/// `git_fs_path_owner_is_system`.
///
/// # Safety
///
/// As [`git_fs_path_owner_is`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_owner_is_system(
    out: Out<'_, bool>,
    path: CStrArg<'_>,
) -> c_int {
    // SAFETY: `out` and `path` are this export's arguments, under
    // `git_fs_path_owner_is`'s contract.
    unsafe { git_fs_path_owner_is(out, path, GIT_FS_PATH_OWNER_ADMINISTRATOR) }
}

/// `git_fs_path_find_executable`.
///
/// # Safety
///
/// `fullpath` is NULL (-1) or a valid `git_str` that nothing else accesses
/// during the call; `executable` NULL (`""`) or a NUL-terminated string
/// that nothing but the call writes (it may lie in `fullpath`'s buffer).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_fs_path_find_executable(
    fullpath: *mut GitStr,
    executable: *const c_char,
) -> c_int {
    barrier(|| {
        // SAFETY: `str_args`' requirements are this export's contract.
        let (f, [e]) = unsafe { str_args(fullpath, [executable]) };
        let Some(f) = f else {
            return -1;
        };
        status(find_executable(f, &e.unwrap_or_default()))
    })
}
