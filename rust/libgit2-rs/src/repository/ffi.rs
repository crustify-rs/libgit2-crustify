//! The exports of `repository.c`.
//!
//! # Safety contract common to these exports
//!
//! A `git_repository *` argument is NULL (rejected as C's `GIT_ASSERT_ARG`
//! does, or with the same message where C would crash) or a repository
//! that libgit2 returned and that has not been freed. Concurrent calls on
//! one repository are allowed as far as libgit2 allows them (the lazily
//! created odb, config, refdb and index, the caches); the setters that
//! replace a string field (`set_workdir`, `set_bare`, `set_namespace`) and
//! `git_repository_free` must not run while anything else uses the
//! repository, or while a string returned by `git_repository_path`,
//! `workdir`, `commondir` or `get_namespace` is still in use. Out-pointers
//! are writable; strings are NUL-terminated.

use crate::git_arg;
use crate::util::cabi::CStrArg;
use crate::util::cabi::Out;
use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::{self, NonNull};

use super::init::{InitOptions, InitOut};
use super::*;
use crate::buf::GitBuf;
use crate::config::{Config, GitConfig};
use crate::index::GitIndex;
use crate::odb::{GitOdb, Odb};
use crate::oid::Oid;
use crate::oidarray::GitOidArray;
use crate::refdb::GitRefdb;
use crate::util::errors::{self, ErrorCode, Result};
use crate::util::export::{barrier, cstr, status};
use crate::util::str::GitStr;
use crate::{git_assert, git_assert_arg};

/// The repository behind a C pointer.
///
/// # Safety
///
/// `p` is NULL or a live repository (the module's contract) for `'a`.
#[inline]
unsafe fn repo<'a>(p: *const GitRepository) -> Option<&'a GitRepository> {
    // SAFETY: the caller's contract; all fields are interior-mutable, so a
    // shared reference allows C's concurrent updates.
    unsafe { p.as_ref() }
}

/// `GIT_BUF_WRAP_PRIVATE(out, f)`.
///
/// # Safety
///
/// `out` is a valid `git_buf` (or NULL, rejected).
unsafe fn buf_wrap(out: *mut GitBuf, f: impl FnOnce(&mut GitStr) -> Result<()>) -> c_int {
    let mut s = GitStr::new();
    // SAFETY: the caller's contract; `s` is a local `git_str`.
    let e = unsafe { crate::buf::ffi::git_buf_tostr(&raw mut s, out) };
    if e != 0 {
        return e;
    }
    let e = status(f(&mut s));
    if e != 0 {
        return e;
    }
    // SAFETY: as above.
    unsafe { crate::buf::ffi::git_buf_fromstr(out, Some(&mut s)) }
}

/// `git_error__check_version(s, max, name)` of a versioned C struct.
///
/// # Safety
///
/// `s` is NULL or points to a struct whose first member is `unsigned int`.
unsafe fn check_version(s: *const c_uint, max: c_uint, name: &str) -> Result<()> {
    if s.is_null() {
        return Ok(());
    }
    // SAFETY: the caller's contract.
    let actual = unsafe { s.read() };
    if actual > 0 && actual <= max {
        return Ok(());
    }
    Err(crate::git_error_set!(
        errors::ErrorClass::INVALID,
        "invalid version {} on {}",
        actual as c_int,
        name
    ))
}

/// `git_repository__cleanup`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository__cleanup(repo_: Option<&GitRepository>) -> c_int {
    barrier(|| {
        let Some(r) = repo_ else {
            return errors::assert_arg_failed("repo").raw();
        };
        // SAFETY: the module's contract (C's: nothing else uses the
        // repository or a weak pointer of it during and after the call).
        status(unsafe { r.cleanup() })
    })
}

/// `git_repository_free` (NULL allowed).
///
/// # Safety
///
/// `repo` is NULL or a live repository nothing else uses.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_free(repo: *mut GitRepository) {
    barrier(|| {
        // SAFETY: the caller's contract.
        unsafe { GitRepository::free(repo) }
    })
}

/// `git_repository_new_options` (`git2/sys/repository.h`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GitRepositoryNewOptions {
    /// The version (1).
    pub version: c_uint,
    /// The object id type (0 for the default).
    pub oid_type: crate::oid::git_oid_t,
}

/// `git_repository_new_options_init`.
///
/// # Safety
///
/// `opts` is NULL (rejected) or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_new_options_init(
    opts: *mut GitRepositoryNewOptions,
    version: c_uint,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!opts.is_null(), "opts", -1);
        // SAFETY: `version` is a local.
        let r = unsafe { check_version(&raw const version, 1, "git_repository_new_options") };
        if let Err(e) = r {
            return e.raw();
        }
        // SAFETY: writable, as checked.
        unsafe {
            opts.write(GitRepositoryNewOptions {
                version: 1,
                oid_type: 0,
            })
        };
        0
    })
}

/// `git_repository_new_ext`.
///
/// # Safety
///
/// `out` is writable; `opts` is NULL or a valid options struct.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_new_ext(
    out: *mut *mut GitRepository,
    opts: *const GitRepositoryNewOptions,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        // SAFETY: NULL or a valid struct.
        if let Err(e) = unsafe { check_version(opts.cast(), 1, "git_repository_new_options") } {
            return e.raw();
        }
        // SAFETY: NULL or a valid struct.
        let t = unsafe { opts.as_ref() }.map_or(0, |o| o.oid_type);
        let t = if t != 0 {
            match OidType::from_raw(t) {
                Some(t) => Some(t),
                None => {
                    let e = errors::assert_arg_failed("git_oid_type_is_valid(opts->oid_type)");
                    return e.raw();
                }
            }
        } else {
            None
        };
        match open::new_ext(t) {
            Ok(r) => {
                // SAFETY: writable.
                unsafe { out.write(r.as_ptr()) };
                0
            }
            Err(e) => {
                // SAFETY: writable.
                unsafe { out.write(ptr::null_mut()) };
                e.raw()
            }
        }
    })
}

/// `git_repository_new`.
///
/// # Safety
///
/// `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_new(out: *mut *mut GitRepository) -> c_int {
    // SAFETY: forwarded.
    unsafe { git_repository_new_ext(out, ptr::null()) }
}

/// `git_repository_open_bare`.
///
/// # Safety
///
/// `repo_ptr` is writable; `bare_path` NULL (rejected) or a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_open_bare(
    mut repo_ptr: Out<'_, *mut GitRepository>,
    bare_path: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        let Some(p) = bare_path.get() else {
            return errors::assert_arg_failed("path").raw();
        };
        match open::open_bare(p) {
            Ok(r) => {
                repo_ptr.write(r.as_ptr());
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_repository_open_ext` (`repo_ptr` may be NULL: only find it).
///
/// # Safety
///
/// `repo_ptr` is NULL or writable; the strings NULL or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_open_ext(
    mut repo_ptr: Out<'_, *mut GitRepository>,
    start_path: CStrArg<'_>,
    flags: c_uint,
    ceiling_dirs: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        repo_ptr.write(ptr::null_mut());
        let s = start_path.get();
        let c = ceiling_dirs.get();
        match open::open_ext(s, flags, c, !repo_ptr.is_null()) {
            Ok(r) => {
                if let Some(r) = r {
                    repo_ptr.write(r.as_ptr());
                }
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_repository_open`.
///
/// # Safety
///
/// As [`git_repository_open_ext`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_open(
    repo_out: Out<'_, *mut GitRepository>,
    path: CStrArg<'_>,
) -> c_int {
    // SAFETY: forwarded.
    unsafe { git_repository_open_ext(repo_out, path, GIT_REPOSITORY_OPEN_NO_SEARCH, CStrArg::NULL) }
}

/// `git_repository_open_from_worktree`.
///
/// # Safety
///
/// `repo_out` writable; `wt` NULL or a live `git_worktree`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_open_from_worktree(
    repo_out: *mut *mut GitRepository,
    wt: *mut c_void,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!repo_out.is_null(), "repo_out", -1);
        git_assert_arg!(!wt.is_null(), "wt", -1);
        // SAFETY: writable.
        unsafe { repo_out.write(ptr::null_mut()) };
        let Some(wt) = NonNull::new(wt) else {
            return -1;
        };
        // SAFETY: a live worktree.
        let gl = unsafe { crate::sys::worktree::Worktree::gitlink_path_of(wt) };
        match head::open_from_worktree(gl) {
            Ok(r) => {
                // SAFETY: writable.
                unsafe { repo_out.write(r.as_ptr()) };
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_repository_wrap_odb`.
///
/// # Safety
///
/// `out` writable; `odb` a live `git_odb`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_wrap_odb(
    mut out: Out<'_, *mut GitRepository>,
    odb: *mut GitOdb,
) -> c_int {
    barrier(|| {
        let Some(o) = NonNull::new(odb) else {
            return errors::assert_arg_failed("odb").raw();
        };
        // SAFETY: a live odb; the new reference is released when `o` drops,
        // after the repository took its own.
        let o = unsafe { Odb::new_ref(o) };
        match open::wrap_odb(&o) {
            Ok(r) => {
                out.write(r.as_ptr());
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_repository_discover`.
///
/// # Safety
///
/// `out` a valid `git_buf`; strings NULL or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_discover(
    out: *mut GitBuf,
    start_path: *const c_char,
    across_fs: c_int,
    ceiling_dirs: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        // SAFETY: NULL or strings.
        let s = unsafe { cstr(start_path) };
        let c = ceiling_dirs.get();
        let Some(s) = s else {
            return errors::assert_arg_failed("start_path").raw();
        };
        match open::discover(s, across_fs != 0, c) {
            // SAFETY: a valid `git_buf` and our local string.
            Ok(mut gitdir) => unsafe { crate::buf::ffi::git_buf_fromstr(out, Some(&mut gitdir)) },
            Err(e) => e.raw(),
        }
    })
}

/// `git_repository_config__weakptr`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_config__weakptr(
    mut out: Out<'_, *mut GitConfig>,
    repo_: Option<&GitRepository>,
) -> c_int {
    // The fast path: already created. Loads and a store only (nothing
    // can panic or allocate), so no barrier.
    if !out.is_null()
        && let Some(r) = repo_
    {
        let p = r.config_ptr();
        if !p.is_null() {
            out.write(p);
            return 0;
        }
    }
    barrier(|| {
        let Some(r) = repo_ else {
            return errors::assert_arg_failed("repo").raw();
        };
        let e = r.config_weak().err();
        out.write(r.config_ptr());
        e.map_or(0, ErrorCode::raw)
    })
}

/// `git_repository_config`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_config(
    mut out: Out<'_, *mut GitConfig>,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        let Some(r) = repo_ else {
            return errors::assert_arg_failed("repo").raw();
        };
        match r.config_weak() {
            Ok(c) => {
                // SAFETY: the repository's live config: a new reference for
                // the caller.
                let owned = unsafe { Config::new_ref(NonNull::from(c)) };
                out.write(owned.into_raw().as_ptr());
                0
            }
            Err(_) => {
                out.write(r.config_ptr());
                -1
            }
        }
    })
}

/// `git_repository_config_snapshot`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_config_snapshot(
    out: *mut *mut GitConfig,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        let Some(r) = repo_ else {
            return errors::assert_arg_failed("repo").raw();
        };
        let weak = match r.config_weak() {
            Ok(w) => w,
            Err(e) => return e.raw(),
        };
        // `git_config_snapshot(out, weak)`: `*out` is NULL on failure.
        git_assert_arg!(!out.is_null(), "out", -1);
        // SAFETY: the caller's contract: writable.
        unsafe { out.write(ptr::null_mut()) };
        match weak.snapshot() {
            Ok(snap) => {
                // SAFETY: as above; the caller owns the new config.
                unsafe { out.write(Config::from_box(snap).into_raw().as_ptr()) };
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_repository_set_config`.
///
/// # Safety
///
/// The module's contract; `config` a live `git_config`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_set_config(
    repo_: Option<&GitRepository>,
    config: *mut GitConfig,
) -> c_int {
    barrier(|| {
        let Some(r) = repo_ else {
            return errors::assert_arg_failed("repo").raw();
        };
        git_assert_arg!(!config.is_null(), "config", -1);
        // SAFETY: the module's contract: `config` is NULL or a live object of
        // the caller's, and no weak pointer to the old one is used after.
        unsafe { r.set_config(NonNull::new(config)) };
        0
    })
}

/// `git_repository_odb__weakptr`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_odb__weakptr(
    out: *mut *mut GitOdb,
    repo_: Option<&GitRepository>,
) -> c_int {
    // The fast path: already created. Loads and a store only (nothing
    // can panic or allocate), so no barrier.
    if !out.is_null()
        && let Some(r) = repo_
    {
        let p = r.odb_ptr();
        if !p.is_null() {
            // SAFETY: writable.
            unsafe { out.write(p) };
            return 0;
        }
    }
    barrier(|| {
        let Some(r) = repo_ else {
            return errors::assert_arg_failed("repo").raw();
        };
        git_assert_arg!(!out.is_null(), "out", -1);
        match r.odb_weak() {
            Ok(p) => {
                // SAFETY: writable.
                unsafe { out.write(p.as_ptr()) };
                0
            }
            Err(e) => {
                // SAFETY: writable.
                unsafe { out.write(ptr::null_mut()) };
                e.raw()
            }
        }
    })
}

/// `git_repository_odb`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_odb(
    out: *mut *mut GitOdb,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        // SAFETY: forwarded.
        if unsafe { git_repository_odb__weakptr(out, repo_) } < 0 {
            return -1;
        }
        // SAFETY: the out-pointer just written.
        let Some(p) = NonNull::new(unsafe { *out }) else {
            return -1;
        };
        // SAFETY: the repository's live odb, which its reference keeps
        // live; the new reference goes to the caller.
        unsafe { ffibox::CShared::increment_count(p.as_ptr()) };
        0
    })
}

/// `git_repository_set_odb`.
///
/// # Safety
///
/// The module's contract; `odb` a live `git_odb`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_set_odb(
    repo_: Option<&GitRepository>,
    odb: *mut GitOdb,
) -> c_int {
    barrier(|| {
        let Some(r) = repo_ else {
            return errors::assert_arg_failed("repo").raw();
        };
        git_assert_arg!(!odb.is_null(), "odb", -1);
        // SAFETY: the module's contract: `odb` is NULL or a live object of
        // the caller's, and no weak pointer to the old one is used after.
        unsafe { r.set_odb(NonNull::new(odb)) };
        0
    })
}

/// `git_repository_refdb__weakptr`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_refdb__weakptr(
    out: *mut *mut GitRefdb,
    repo_: Option<&GitRepository>,
) -> c_int {
    // The fast path: already created. Loads and a store only (nothing
    // can panic or allocate), so no barrier.
    if !out.is_null()
        && let Some(r) = repo_
    {
        let p = r.refdb_ptr();
        if !p.is_null() {
            // SAFETY: writable.
            unsafe { out.write(p) };
            return 0;
        }
    }
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        let Some(r) = repo_ else {
            return errors::assert_arg_failed("repo").raw();
        };
        let e = r.refdb_weak().err();
        // SAFETY: writable.
        unsafe { out.write(r.refdb_ptr()) };
        e.map_or(0, ErrorCode::raw)
    })
}

/// `git_repository_refdb`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_refdb(
    out: *mut *mut GitRefdb,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        // SAFETY: forwarded.
        if unsafe { git_repository_refdb__weakptr(out, repo_) } < 0 {
            return -1;
        }
        {
            // SAFETY: the out-pointer just written.
            let Some(p) = NonNull::new(unsafe { *out }) else {
                return -1;
            };
            // SAFETY: the weak pointer: a live refdb, which the
            // repository's reference keeps live; the new reference goes to
            // the caller.
            unsafe { ffibox::CShared::increment_count(p.as_ptr()) };
        }
        0
    })
}

/// `git_repository_set_refdb`.
///
/// # Safety
///
/// The module's contract; `refdb` a live `git_refdb`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_set_refdb(
    repo_: Option<&GitRepository>,
    refdb: *mut GitRefdb,
) -> c_int {
    barrier(|| {
        let Some(r) = repo_ else {
            return errors::assert_arg_failed("repo").raw();
        };
        git_assert_arg!(!refdb.is_null(), "refdb", -1);
        // SAFETY: the module's contract: `refdb` is NULL or a live object of
        // the caller's, and no weak pointer to the old one is used after.
        unsafe { r.set_refdb(NonNull::new(refdb)) };
        0
    })
}

/// `git_repository_index__weakptr`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_index__weakptr(
    out: *mut *mut GitIndex,
    repo_: Option<&GitRepository>,
) -> c_int {
    // The fast path: already created. Loads and a store only (nothing
    // can panic or allocate), so no barrier.
    if !out.is_null()
        && let Some(r) = repo_
    {
        let p = r.index_ptr();
        if !p.is_null() {
            // SAFETY: writable.
            unsafe { out.write(p) };
            return 0;
        }
    }
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        let Some(r) = repo_ else {
            return errors::assert_arg_failed("repo").raw();
        };
        let e = r.index_weak().err();
        // SAFETY: writable.
        unsafe { out.write(r.index_ptr()) };
        e.map_or(0, ErrorCode::raw)
    })
}

/// `git_repository_index`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_index(
    out: *mut *mut GitIndex,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        // SAFETY: forwarded.
        if unsafe { git_repository_index__weakptr(out, repo_) } < 0 {
            return -1;
        }
        {
            // SAFETY: the out-pointer just written.
            let Some(p) = NonNull::new(unsafe { *out }) else {
                return -1;
            };
            // SAFETY: the weak pointer: a live index, which the
            // repository's reference keeps live; the new reference goes to
            // the caller.
            unsafe { ffibox::CShared::increment_count(p.as_ptr()) };
        }
        0
    })
}

/// `git_repository_set_index` (`index` may be NULL: drop the index).
///
/// # Safety
///
/// The module's contract; `index` NULL or a live `git_index`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_set_index(
    repo_: Option<&GitRepository>,
    index: *mut GitIndex,
) -> c_int {
    barrier(|| {
        let Some(r) = repo_ else {
            return errors::assert_arg_failed("repo").raw();
        };
        // SAFETY: the module's contract: `index` is NULL or a live object of
        // the caller's, and no weak pointer to the old one is used after.
        unsafe { r.set_index(NonNull::new(index)) };
        0
    })
}

/// `git_repository_grafts__weakptr`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_grafts__weakptr(
    out: *mut *mut crate::grafts::Grafts,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        let r = repo_;
        git_assert_arg!(!out.is_null() && r.is_some(), "out && repo", -1);
        let Some(r) = r else { return -1 };
        let g = r.grafts_ptr();
        git_assert!(!g.is_null(), "repo->grafts", -1);
        // SAFETY: writable.
        unsafe { out.write(g) };
        0
    })
}

/// `git_repository_shallow_grafts__weakptr`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_shallow_grafts__weakptr(
    out: *mut *mut crate::grafts::Grafts,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        let r = repo_;
        git_assert_arg!(!out.is_null() && r.is_some(), "out && repo", -1);
        let Some(r) = r else { return -1 };
        let g = r.shallow_grafts_ptr();
        git_assert!(!g.is_null(), "repo->shallow_grafts", -1);
        // SAFETY: writable.
        unsafe { out.write(g) };
        0
    })
}

/// `git_repository_set_namespace`.
///
/// # Safety
///
/// The module's contract (a string-replacing setter).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_set_namespace(
    repo_: Option<&GitRepository>,
    namespace: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        let Some(r) = repo_ else {
            return errors::assert_arg_failed("repo").raw();
        };
        let n = namespace.get();
        let v = match n {
            None => None,
            Some(n) => match alloc_cstr(n.to_bytes()) {
                Ok(v) => Some(v),
                Err(_) => {
                    // C frees the old one and leaves NULL.
                    // SAFETY: the module's contract.
                    unsafe { r.replace_string(StrField::Namespace, None) };
                    return -1;
                }
            },
        };
        // SAFETY: the module's contract.
        unsafe { r.replace_string(StrField::Namespace, v) };
        0
    })
}

/// `git_repository_get_namespace`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_get_namespace(
    repo_: Option<&GitRepository>,
) -> *const c_char {
    barrier(|| match repo_ {
        Some(r) => r.string_ptr(StrField::Namespace),
        None => {
            let _ = errors::assert_arg_failed("repo");
            ptr::null()
        }
    })
}

/// `git_repository__reserved_names` (Unix: the static lists).
///
/// # Safety
///
/// `out` and `outlen` are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository__reserved_names(
    mut out: Out<'_, *const StaticStr>,
    mut outlen: Out<'_, usize>,
    _repo: *mut GitRepository,
    include_ntfs: bool,
) -> bool {
    barrier(|| {
        let (p, n) = if include_ntfs {
            (
                git_repository__reserved_names_win32.as_ptr(),
                git_repository__reserved_names_win32.len(),
            )
        } else {
            (
                git_repository__reserved_names_posix.as_ptr(),
                git_repository__reserved_names_posix.len(),
            )
        };
        out.write(p);
        outlen.write(n);
        true
    })
}

/// `git_repository__extensions`.
///
/// # Safety
///
/// `out` and `out_len` are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository__extensions(
    out: *mut *mut *mut c_char,
    out_len: *mut usize,
) -> c_int {
    barrier(|| {
        let Ok(exts) = open::extensions() else {
            return -1;
        };
        let arr = crate::util::alloc::mallocarray(exts.len().max(1), size_of::<*mut c_char>())
            .cast::<*mut c_char>();
        if arr.is_null() {
            return -1;
        }
        for (i, e) in exts.iter().enumerate() {
            let s = crate::util::alloc::substrdup(e);
            if s.is_null() {
                // C leaks what it made; freed here.
                // SAFETY: the `i` strings made so far.
                let made = unsafe { core::slice::from_raw_parts(arr, i) };
                for p in made {
                    // SAFETY: a string made above, freed once.
                    unsafe { crate::util::alloc::free(p.cast()) };
                }
                // SAFETY: the array made above.
                unsafe { crate::util::alloc::free(arr.cast()) };
                return -1;
            }
            // SAFETY: `i < len <= capacity`.
            let slot = unsafe { arr.add(i) };
            // SAFETY: as above.
            unsafe { slot.write(s) };
        }
        // SAFETY: writable (the contract).
        unsafe { out.write(arr) };
        // SAFETY: writable (the contract).
        unsafe { out_len.write(exts.len()) };
        0
    })
}

/// `git_repository__set_extensions`.
///
/// # Safety
///
/// `extensions` points to `len` NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository__set_extensions(
    extensions: *const *const c_char,
    len: usize,
) -> c_int {
    barrier(|| {
        let ptrs: &[*const c_char] = if len == 0 || extensions.is_null() {
            &[]
        } else {
            // SAFETY: the caller's contract: that many pointers.
            unsafe { core::slice::from_raw_parts(extensions, len) }
        };
        let v: Vec<&[u8]> = ptrs
            .iter()
            // SAFETY: the caller's contract: each a string.
            .map(|&p| unsafe { crate::util::export::cstr_bytes(p) })
            .collect();
        open::set_extensions(&v);
        0
    })
}

/// `git_repository__free_extensions`.
///
/// # Safety
///
/// None.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository__free_extensions() {
    barrier(open::free_extensions)
}

/// `git_repository_init_options` (`include/git2/repository.h`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GitRepositoryInitOptions {
    /// The version (1).
    pub version: c_uint,
    /// `GIT_REPOSITORY_INIT_*` flags.
    pub flags: u32,
    /// The shared mode.
    pub mode: u32,
    /// The working directory.
    pub workdir_path: *const c_char,
    /// The description file's content.
    pub description: *const c_char,
    /// The template directory.
    pub template_path: *const c_char,
    /// The initial HEAD.
    pub initial_head: *const c_char,
    /// The URL of an `origin` remote to add.
    pub origin_url: *const c_char,
    /// `git_oid_t`.
    pub oid_type: crate::oid::git_oid_t,
    /// `git_refdb_t`.
    pub refdb_type: c_int,
}

/// `git_repository_init_options_init`.
///
/// # Safety
///
/// `opts` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_init_options_init(
    opts: *mut GitRepositoryInitOptions,
    version: c_uint,
) -> c_int {
    barrier(|| {
        // SAFETY: `version` is a local.
        let r = unsafe { check_version(&raw const version, 1, "git_repository_init_options") };
        if let Err(e) = r {
            return e.raw();
        }
        git_assert_arg!(!opts.is_null(), "opts", -1);
        // SAFETY: writable.
        unsafe {
            opts.write(GitRepositoryInitOptions {
                version: 1,
                flags: 0,
                mode: 0,
                workdir_path: ptr::null(),
                description: ptr::null(),
                template_path: ptr::null(),
                initial_head: ptr::null(),
                origin_url: ptr::null(),
                oid_type: 0,
                refdb_type: 0,
            })
        };
        0
    })
}

/// `git_repository_init_ext`.
///
/// # Safety
///
/// `out` writable; `given_repo` a string; `opts` a valid options struct
/// (C writes its `flags` back, as the C does).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_init_ext(
    out: *mut *mut GitRepository,
    given_repo: *const c_char,
    opts: *mut GitRepositoryInitOptions,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        // SAFETY: NULL or a string.
        let Some(given) = (unsafe { cstr(given_repo) }) else {
            return errors::assert_arg_failed("given_repo").raw();
        };
        git_assert_arg!(!opts.is_null(), "opts", -1);
        // SAFETY: a valid struct.
        if let Err(e) = unsafe { check_version(opts.cast(), 1, "git_repository_init_options") } {
            return e.raw();
        }
        // SAFETY: a valid struct with NULL or NUL-terminated strings.
        let o = unsafe { &*opts };
        // SAFETY: as above.
        let own = |p: *const c_char| unsafe { cstr(p) }.map(CStr::to_owned);
        let mut io = InitOptions {
            flags: o.flags,
            mode: o.mode,
            workdir_path: own(o.workdir_path),
            description: own(o.description),
            template_path: own(o.template_path),
            initial_head: own(o.initial_head),
            origin_url: own(o.origin_url),
            oid_type: o.oid_type,
            refdb_type: o.refdb_type,
        };
        let (r, o) = init::init_ext(given, &mut io);
        // C works on the caller's struct: its flags gain the internal bits.
        // SAFETY: a valid, writable struct.
        unsafe { (*opts).flags = io.flags };
        match o {
            InitOut::Unset => {}
            // SAFETY: writable.
            InitOut::Null => unsafe { out.write(ptr::null_mut()) },
            // SAFETY: writable.
            InitOut::Repo(p) => unsafe { out.write(p.as_ptr()) },
        }
        status(r)
    })
}

/// `git_repository_init`.
///
/// # Safety
///
/// As [`git_repository_init_ext`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_init(
    out: *mut *mut GitRepository,
    path: *const c_char,
    is_bare: c_uint,
) -> c_int {
    let mut opts = GitRepositoryInitOptions {
        version: 1,
        flags: GIT_REPOSITORY_INIT_MKPATH
            | if is_bare != 0 {
                GIT_REPOSITORY_INIT_BARE
            } else {
                0
            },
        mode: 0,
        workdir_path: ptr::null(),
        description: ptr::null(),
        template_path: ptr::null(),
        initial_head: ptr::null(),
        origin_url: ptr::null(),
        oid_type: 0,
        refdb_type: 0,
    };
    // SAFETY: forwarded, with local options.
    unsafe { git_repository_init_ext(out, path, &raw mut opts) }
}

/// `git_repository_reinit_filesystem`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_reinit_filesystem(
    repo_: Option<&GitRepository>,
    recurse: c_int,
) -> c_int {
    barrier(|| {
        let Some(r) = repo_ else {
            return errors::assert_arg_failed("repo").raw();
        };
        status(init::reinit_filesystem(r, recurse != 0))
    })
}

#[inline(always)]
fn call<R>(r: &GitRepository, f: impl FnOnce(&GitRepository) -> R) -> R {
    f(r)
}

macro_rules! with_repo {
    ($p:expr, $name:literal, $f:expr) => {{
        let p: Option<&GitRepository> = $p;
        match p {
            Some(r) => call(r, $f),
            None => errors::assert_arg_failed($name).raw(),
        }
    }};
}

/// `git_repository_head_detached`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_head_detached(repo_: Option<&GitRepository>) -> c_int {
    barrier(|| with_repo!(repo_, "repo", head::head_detached))
}

/// `git_repository_head_detached_for_worktree`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_head_detached_for_worktree(
    repo_: Option<&GitRepository>,
    name: *const c_char,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            // SAFETY: NULL or a string.
            let Some(n) = (unsafe { cstr(name) }) else {
                return errors::assert_arg_failed("name").raw();
            };
            match head::head_detached_for_worktree(r, n) {
                Ok(b) => c_int::from(b),
                Err(e) => e.raw(),
            }
        })
    })
}

/// `git_repository_head`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_head(
    head_out: *mut *mut c_void,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!head_out.is_null(), "head_out", -1);
        with_repo!(repo_, "repo", |r| match head::head(r) {
            Ok(h) => {
                // SAFETY: writable.
                unsafe { head_out.write(h.into_raw().cast()) };
                0
            }
            Err(e) => e.raw(),
        })
    })
}

/// `git_repository_head_for_worktree`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_head_for_worktree(
    out: *mut *mut c_void,
    repo_: Option<&GitRepository>,
    name: *const c_char,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        with_repo!(repo_, "repo", |r| {
            // SAFETY: NULL or a string.
            let Some(n) = (unsafe { cstr(name) }) else {
                return errors::assert_arg_failed("name").raw();
            };
            // SAFETY: writable.
            unsafe { out.write(ptr::null_mut()) };
            match head::with_head_for_worktree(r, n, |h| h.into_raw().cast()) {
                Ok(h) => {
                    // SAFETY: writable.
                    unsafe { out.write(h) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    })
}

/// `git_repository_foreach_worktree_cb`.
pub type ForeachWorktreeCb =
    unsafe extern "C" fn(repo: *mut GitRepository, payload: *mut c_void) -> c_int;

/// `git_repository_foreach_worktree`.
///
/// # Safety
///
/// The module's contract; `cb` is sound to call with `payload`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_foreach_worktree(
    repo_: Option<&GitRepository>,
    cb: Option<ForeachWorktreeCb>,
    payload: *mut c_void,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            let Some(cb) = cb else {
                return errors::assert_arg_failed("cb").raw();
            };
            head::foreach_worktree(r, &mut |w: &GitRepository| {
                // SAFETY: the caller's contract; `w` is a live repository
                // for the call.
                unsafe { cb(ptr::from_ref(w).cast_mut(), payload) }
            })
        })
    })
}

/// `git_repository_head_unborn`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_head_unborn(repo_: Option<&GitRepository>) -> c_int {
    barrier(|| with_repo!(repo_, "repo", head::head_unborn))
}

/// `git_repository_initialbranch`.
///
/// # Safety
///
/// The module's contract; `out` a valid `git_str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_initialbranch(
    out: Option<&mut GitStr>,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        let out = git_arg!(out, "out", -1);
        let o = out;
        with_repo!(repo_, "repo", |r| status(head::initialbranch(o, r)))
    })
}

/// `git_repository_is_empty`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_is_empty(repo_: Option<&GitRepository>) -> c_int {
    barrier(|| with_repo!(repo_, "repo", head::is_empty))
}

/// `git_repository_item_path`.
///
/// # Safety
///
/// The module's contract; `out` a valid `git_buf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_item_path(
    out: *mut GitBuf,
    repo_: Option<&GitRepository>,
    item: ItemT,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r: &GitRepository| {
            // SAFETY: a valid `git_buf`.
            unsafe { buf_wrap(out, |s| r.item_path(s, item)) }
        })
    })
}

/// `git_repository__item_path`.
///
/// # Safety
///
/// The module's contract; `out` a valid `git_str` that is not one of the
/// repository's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository__item_path(
    out: Option<&mut GitStr>,
    repo_: Option<&GitRepository>,
    item: ItemT,
) -> c_int {
    barrier(|| {
        let out = git_arg!(out, "out", -1);
        let o = out;
        with_repo!(repo_, "repo", |r: &GitRepository| status(
            r.item_path(o, item)
        ))
    })
}

macro_rules! string_getter {
    ($(#[$m:meta])* $name:ident, $get:expr) => {
        $(#[$m])*
        ///
        /// # Safety
        ///
        /// The module's contract.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(repo_: *const GitRepository) -> *const c_char {
            // SAFETY: the module's contract.
            if let Some(r) = unsafe { repo(repo_) } {
                // Loads only: nothing can panic, so no barrier.
                return $get(r);
            }
            barrier(|| {
                let _ = errors::assert_arg_failed("repo");
                ptr::null()
            })
        }
    };
}

string_getter!(
    /// `git_repository_path`.
    git_repository_path,
    |r: &GitRepository| r.string_ptr(StrField::Gitdir)
);
string_getter!(
    /// `git_repository_workdir`.
    git_repository_workdir,
    |r: &GitRepository| if r.is_bare() {
        ptr::null()
    } else {
        r.string_ptr(StrField::Workdir)
    }
);
string_getter!(
    /// `git_repository_commondir`.
    git_repository_commondir,
    |r: &GitRepository| r.string_ptr(StrField::Commondir)
);

/// `git_repository_workdir_path`.
///
/// # Safety
///
/// The module's contract; `out` a valid `git_str`; `path` a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_workdir_path(
    out: *mut GitStr,
    repo_: Option<&GitRepository>,
    path: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        let p = path.to_bytes();
        // `path` may point into `out`'s buffer: then it is copied first.
        // SAFETY: a valid `git_str`; only its fields are read.
        let o_ref = unsafe { &*out };
        let (base, cap) = (o_ref.as_ptr() as usize, o_ref.capacity());
        let a = p.as_ptr() as usize;
        let copy;
        let p: &[u8] = if a >= base && a < base.wrapping_add(cap) {
            copy = p.to_vec();
            &copy
        } else {
            p
        };
        // SAFETY: a valid `git_str`, not otherwise accessed during the call.
        let o = unsafe { &mut *out };
        with_repo!(repo_, "repo", |r| status(head::workdir_path(o, r, p)))
    })
}

/// `git_repository_set_workdir`.
///
/// # Safety
///
/// The module's contract (a string-replacing setter).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_set_workdir(
    repo_: Option<&GitRepository>,
    workdir: *const c_char,
    update_gitlink: c_int,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            // SAFETY: NULL or a string (which may be the current workdir:
            // copied before it is replaced).
            let Some(w) = (unsafe { cstr(workdir) }).map(CStr::to_owned) else {
                return errors::assert_arg_failed("workdir").raw();
            };
            // SAFETY: the module's contract (a string-replacing setter).
            status(unsafe { init::set_workdir(r, &w, update_gitlink != 0) })
        })
    })
}

/// `git_repository_is_bare`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_is_bare(repo_: Option<&GitRepository>) -> c_int {
    if let Some(r) = repo_ {
        // A load only: nothing can panic, so no barrier.
        return c_int::from(r.is_bare());
    }
    barrier(|| errors::assert_arg_failed("repo").raw())
}

/// `git_repository_is_worktree`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_is_worktree(repo_: Option<&GitRepository>) -> c_int {
    if let Some(r) = repo_ {
        // A load only: nothing can panic, so no barrier.
        return c_int::from(r.is_worktree());
    }
    barrier(|| errors::assert_arg_failed("repo").raw())
}

/// `git_repository_set_bare`.
///
/// # Safety
///
/// The module's contract (a string-replacing setter).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_set_bare(repo_: Option<&GitRepository>) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            // SAFETY: the module's contract (a string-replacing setter).
            status(unsafe { init::set_bare(r) })
        })
    })
}

/// `git_repository_head_commit`.
///
/// # Safety
///
/// The module's contract; `commit` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_head_commit(
    mut commit: Out<'_, *mut c_void>,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        with_repo!(
            repo_,
            "repo",
            |r| match head::head_peeled(r, head::PEEL_COMMIT) {
                Ok(o) => {
                    commit.write(o.into_raw().cast());
                    0
                }
                Err(e) => e.raw(),
            }
        )
    })
}

/// `git_repository_head_tree`.
///
/// # Safety
///
/// The module's contract; `tree` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_head_tree(
    mut tree: Out<'_, *mut c_void>,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        with_repo!(
            repo_,
            "repo",
            |r| match head::head_peeled(r, head::PEEL_TREE) {
                Ok(o) => {
                    tree.write(o.into_raw().cast());
                    0
                }
                Err(e) => e.raw(),
            }
        )
    })
}

/// `git_repository__set_orig_head`.
///
/// # Safety
///
/// The module's contract; `orig_head` a valid id.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository__set_orig_head(
    repo_: Option<&GitRepository>,
    orig_head: *const Oid,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            // SAFETY: NULL or a valid id.
            let Some(id) = (unsafe { orig_head.as_ref() }) else {
                return errors::assert_arg_failed("id").raw();
            };
            status(head::set_orig_head(r, id))
        })
    })
}

/// `git_repository_message`.
///
/// # Safety
///
/// The module's contract; `out` a valid `git_buf`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_message(
    out: *mut GitBuf,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            // SAFETY: a valid `git_buf`.
            unsafe { buf_wrap(out, |s| head::message(s, r)) }
        })
    })
}

/// `git_repository_message_remove`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_message_remove(repo_: Option<&GitRepository>) -> c_int {
    barrier(|| with_repo!(repo_, "repo", head::message_remove))
}

/// `git_repository_hashfile`.
///
/// # Safety
///
/// The module's contract; `out` writable; strings NULL or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_hashfile(
    out: *mut Oid,
    repo_: Option<&GitRepository>,
    path: *const c_char,
    object_type: c_int,
    as_path: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        let r = repo_;
        if r.is_none() {
            // `git_repository_workdir(repo)` runs first and asserts too.
            let _ = errors::assert_arg_failed("repo");
        }
        git_assert_arg!(!out.is_null(), "out", -1);
        // SAFETY: NULL or strings.
        let p = unsafe { cstr(path) };
        let a = as_path.get();
        let Some(p) = p else {
            return errors::assert_arg_failed("path").raw();
        };
        let Some(r) = r else {
            return errors::assert_arg_failed("repo").raw();
        };
        match head::hashfile(r, p, object_type, a) {
            Ok(id) => {
                // SAFETY: writable.
                unsafe { out.write(id) };
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_repository_set_head`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_set_head(
    repo_: Option<&GitRepository>,
    refname: *const c_char,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            // SAFETY: NULL or a string.
            let Some(n) = (unsafe { cstr(refname) }).map(CStr::to_owned) else {
                return errors::assert_arg_failed("refname").raw();
            };
            status(head::set_head(r, &n))
        })
    })
}

/// `git_repository_set_head_detached`.
///
/// # Safety
///
/// The module's contract; `committish` NULL or a valid id.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_set_head_detached(
    repo_: Option<&GitRepository>,
    committish: *const Oid,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            // SAFETY: NULL or a valid id (copied).
            let id = unsafe { committish.as_ref() }.copied();
            status(head::set_head_detached(r, id.as_ref()))
        })
    })
}

/// `git_repository_set_head_detached_from_annotated`.
///
/// # Safety
///
/// The module's contract; `committish` NULL or a live annotated commit.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_set_head_detached_from_annotated(
    repo_: Option<&GitRepository>,
    committish: *const c_void,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            let Some(c) = NonNull::new(committish.cast_mut()) else {
                return errors::assert_arg_failed("committish").raw();
            };
            // SAFETY: a live annotated commit.
            status(unsafe { head::set_head_detached_from_annotated(r, c) })
        })
    })
}

/// `git_repository_detach_head`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_detach_head(repo_: Option<&GitRepository>) -> c_int {
    barrier(|| with_repo!(repo_, "repo", |r| status(head::detach_head(r))))
}

/// `git_repository_state`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_state(repo_: Option<&GitRepository>) -> c_int {
    barrier(|| with_repo!(repo_, "repo", |r| head::repo_state(r).unwrap_or(-1)))
}

/// `git_repository__cleanup_files`.
///
/// # Safety
///
/// The module's contract; `files` points to `files_len` strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository__cleanup_files(
    repo_: Option<&GitRepository>,
    files: *const *const c_char,
    files_len: usize,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            let ptrs: &[*const c_char] = if files_len == 0 || files.is_null() {
                &[]
            } else {
                // SAFETY: the caller's contract: that many pointers.
                unsafe { core::slice::from_raw_parts(files, files_len) }
            };
            let v: Vec<&[u8]> = ptrs
                .iter()
                // SAFETY: the caller's contract: each a string.
                .map(|&p| unsafe { crate::util::export::cstr_bytes(p) })
                .collect();
            head::cleanup_files(r, &v)
        })
    })
}

/// `git_repository_state_cleanup`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_state_cleanup(repo_: Option<&GitRepository>) -> c_int {
    barrier(|| with_repo!(repo_, "repo", |r| status(head::state_cleanup(r))))
}

/// `git_repository__shallow_roots`.
///
/// # Safety
///
/// The module's contract; `out` and `out_len` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository__shallow_roots(
    mut out: Out<'_, *mut Oid>,
    mut out_len: Out<'_, usize>,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| match head::shallow_roots(r) {
            Ok((p, n)) => {
                out.write(p);
                out_len.write(n);
                0
            }
            Err(e) => e.raw(),
        })
    })
}

/// `git_repository__shallow_roots_write`.
///
/// # Safety
///
/// The module's contract; `roots` a valid `git_oidarray`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository__shallow_roots_write(
    repo_: Option<&GitRepository>,
    roots: *const GitOidArray,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            // SAFETY: NULL or a valid array.
            let Some(roots) = (unsafe { roots.as_ref() }) else {
                return errors::assert_arg_failed("roots").raw();
            };
            let ids: &[Oid] = if roots.count == 0 {
                &[]
            } else {
                // SAFETY: `count` ids at `ids`.
                unsafe { core::slice::from_raw_parts(roots.ids, roots.count) }
            };
            status(head::shallow_roots_write(r, ids))
        })
    })
}

/// `git_repository_is_shallow`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_is_shallow(repo_: Option<&GitRepository>) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| match head::is_shallow(r) {
            Ok(b) => c_int::from(b),
            Err(e) => e.raw(),
        })
    })
}

/// `git_repository_ident`.
///
/// # Safety
///
/// The module's contract; `name` and `email` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_ident(
    mut name: Out<'_, *const c_char>,
    mut email: Out<'_, *const c_char>,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r: &GitRepository| {
            name.write(r.ident_name().map_or(ptr::null(), CStr::as_ptr));
            email.write(r.ident_email().map_or(ptr::null(), CStr::as_ptr));
            0
        })
    })
}

/// `git_repository_set_ident`.
///
/// # Safety
///
/// The module's contract; strings NULL or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_set_ident(
    repo_: Option<&GitRepository>,
    name: CStrArg<'_>,
    email: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            let n = name.get().map(CStr::to_owned);
            let e = email.get().map(CStr::to_owned);
            status(r.set_ident(n.as_deref(), e.as_deref()))
        })
    })
}

/// `git_repository_submodule_cache_all`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_submodule_cache_all(
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| with_repo!(repo_, "repo", crate::sys::submodule::cache_init))
}

/// `git_repository_submodule_cache_clear`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_submodule_cache_clear(
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| with_repo!(repo_, "repo", GitRepository::submodule_cache_clear))
}

/// `git_repository_oid_type` (0 for NULL).
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_oid_type(
    repo_: Option<&GitRepository>,
) -> crate::oid::git_oid_t {
    barrier(|| repo_.map_or(0, GitRepository::oid_type_raw))
}

/// `git_commitarray` (`include/git2/commit.h`).
#[repr(C)]
pub struct GitCommitArray {
    /// The commits.
    pub commits: *mut *mut c_void,
    /// Their number.
    pub count: usize,
}

/// `git_repository_commit_parents`.
///
/// # Safety
///
/// The module's contract; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_commit_parents(
    out: *mut GitCommitArray,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        let r = repo_;
        git_assert_arg!(!out.is_null() && r.is_some(), "out && repo", -1);
        let Some(r) = r else { return -1 };
        // SAFETY: writable.
        unsafe {
            out.write(GitCommitArray {
                commits: ptr::null_mut(),
                count: 0,
            })
        };
        match head::commit_parents(r) {
            Ok(v) => {
                if v.is_empty() {
                    return 0;
                }
                let mut v = core::mem::ManuallyDrop::new(v);
                let (p, n) = (v.as_mut_ptr(), v.len());
                // SAFETY: writable; the array is `git__allocator`'s, which
                // `git_commitarray_dispose` frees.
                unsafe {
                    out.write(GitCommitArray {
                        commits: p.cast(),
                        count: n,
                    })
                };
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_repository__abbrev_length`.
///
/// # Safety
///
/// The module's contract; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository__abbrev_length(
    mut out: Out<'_, c_int>,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| match head::abbrev_length(r) {
            Ok(n) => {
                out.write(n);
                0
            }
            Err(e) => e.raw(),
        })
    })
}

/// `git_repository__set_objectformat`.
///
/// # Safety
///
/// The module's contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository__set_objectformat(
    repo_: Option<&GitRepository>,
    oid_type: crate::oid::git_oid_t,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            // SAFETY: the module's contract (no weak pointer of the old
            // index, odb or refdb is used after).
            status(unsafe { head::set_objectformat(r, oid_type) })
        })
    })
}
