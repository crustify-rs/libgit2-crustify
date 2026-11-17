//! The exports of `repository.c`.
//!
//! # Safety contract common to these exports
//!
//! A `git_repository *` argument is NULL (rejected as C's `GIT_ASSERT_ARG`
//! does, or with the same message where C would crash) or a repository
//! that libgit2 returned and that has not been freed; it is only read
//! through `&` (its state is interior-mutable), so the exports take it as
//! `Option<&GitRepository>`. Concurrent calls on one repository are
//! allowed as far as libgit2 allows them (the lazily created odb, config,
//! refdb and index, the caches). Not allowed, as in C:
//!
//! - the setters that replace a string field (`set_workdir`, `set_bare`,
//!   `set_namespace`, `set_ident`), `submodule_cache_clear` and
//!   `git_repository_free` / `__cleanup` while anything else uses the
//!   repository, or while a string returned by `git_repository_path`,
//!   `workdir`, `commondir`, `get_namespace` or `ident` is still in use;
//! - the setters that replace the odb, config, refdb or index
//!   (`set_odb`, `set_config`, `set_refdb`, `set_index`,
//!   `__set_objectformat`) while a weak pointer to the old object (from a
//!   `__weakptr` export, or held by a reference, iterator or reflog) is
//!   still used.
//!
//! Out-pointers are NULL (where C checks, or where `Out` skips it) or
//! writable; strings are NULL (where C checks) or NUL-terminated and
//! unwritten during the call.

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
use crate::util::export::{barrier, status};
use crate::util::str::GitStr;
use crate::{git_assert, git_assert_arg};

/// `GIT_BUF_WRAP_PRIVATE(out, f)`.
///
/// # Safety
///
/// `out` is NULL (rejected by `git_buf_tostr`) or a valid `git_buf` that
/// nothing else uses during the call.
unsafe fn buf_wrap(out: *mut GitBuf, f: impl FnOnce(&mut GitStr) -> Result<()>) -> c_int {
    let mut s = GitStr::new();
    // SAFETY: `out` is NULL or a valid, unshared `git_buf` (the contract);
    // `s` is our local `git_str`.
    let e = unsafe { crate::buf::ffi::git_buf_tostr(&raw mut s, out) };
    if e != 0 {
        return e;
    }
    let e = status(f(&mut s));
    if e != 0 {
        return e;
    }
    // SAFETY: as above (`git_buf_tostr` accepted `out`).
    unsafe { crate::buf::ffi::git_buf_fromstr(out, Some(&mut s)) }
}

/// `git_error__check_version` of a version C read from its struct.
fn check_version(actual: Option<c_uint>, max: c_uint, name: &str) -> Result<()> {
    let Some(actual) = actual else {
        return Ok(());
    };
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
/// The module's contract; nothing else uses the repository, or a weak
/// pointer (odb, config, refdb, index) of it, during or after the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository__cleanup(repo_: Option<&GitRepository>) -> c_int {
    barrier(|| {
        let Some(r) = repo_ else {
            return errors::assert_arg_failed("repo").raw();
        };
        // SAFETY: this export's contract is `cleanup`'s: nothing else uses
        // the repository or its weak pointers during and after the call.
        status(unsafe { r.cleanup() })
    })
}

/// `git_repository_free` (NULL allowed).
///
/// # Safety
///
/// `repo` is NULL or a live repository (from libgit2) that the caller
/// gives up: nothing uses it, or a weak pointer of it, after the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_free(repo: *mut GitRepository) {
    barrier(|| {
        // SAFETY: this export's contract is `GitRepository::free`'s.
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
/// `opts` is NULL (rejected) or writable for a `git_repository_new_options`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_new_options_init(
    mut opts: Out<'_, GitRepositoryNewOptions>,
    version: c_uint,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!opts.is_null(), "opts", -1);
        if let Err(e) = check_version(Some(version), 1, "git_repository_new_options") {
            return e.raw();
        }
        opts.write(GitRepositoryNewOptions {
            version: 1,
            oid_type: 0,
        });
        0
    })
}

/// `git_repository_new_ext`.
///
/// # Safety
///
/// `out` is NULL (rejected) or writable; `opts` NULL or a valid options
/// struct, unwritten during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_new_ext(
    mut out: Out<'_, *mut GitRepository>,
    opts: Option<&GitRepositoryNewOptions>,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        if let Err(e) = check_version(opts.map(|o| o.version), 1, "git_repository_new_options") {
            return e.raw();
        }
        let t = opts.map_or(0, |o| o.oid_type);
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
                out.write(r.as_ptr());
                0
            }
            Err(e) => {
                out.write(ptr::null_mut());
                e.raw()
            }
        }
    })
}

/// `git_repository_new`.
///
/// # Safety
///
/// `out` is NULL (rejected) or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_new(out: Out<'_, *mut GitRepository>) -> c_int {
    // SAFETY: `out` as `git_repository_new_ext` takes it; no options.
    unsafe { git_repository_new_ext(out, None) }
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
    // SAFETY: the arguments as `git_repository_open_ext` takes them, with
    // this export's (identical) contract.
    unsafe { git_repository_open_ext(repo_out, path, GIT_REPOSITORY_OPEN_NO_SEARCH, CStrArg::NULL) }
}

/// `git_repository_open_from_worktree`.
///
/// # Safety
///
/// `repo_out` is NULL (rejected) or writable; `wt` NULL (rejected) or a
/// live `git_worktree`, unwritten during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_open_from_worktree(
    mut repo_out: Out<'_, *mut GitRepository>,
    wt: *mut c_void,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!repo_out.is_null(), "repo_out", -1);
        git_assert_arg!(!wt.is_null(), "wt", -1);
        repo_out.write(ptr::null_mut());
        let Some(wt) = NonNull::new(wt) else {
            return -1;
        };
        // SAFETY: a live worktree for the call (this export's contract).
        let gl = unsafe { crate::sys::worktree::Worktree::gitlink_path_of(wt) };
        match head::open_from_worktree(gl) {
            Ok(r) => {
                repo_out.write(r.as_ptr());
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
/// `out` is NULL (rejected) or a valid `git_buf` that nothing else uses
/// during the call; the strings NULL or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_discover(
    out: *mut GitBuf,
    start_path: CStrArg<'_>,
    across_fs: c_int,
    ceiling_dirs: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        let c = ceiling_dirs.get();
        let Some(s) = start_path.get() else {
            return errors::assert_arg_failed("start_path").raw();
        };
        match open::discover(s, across_fs != 0, c) {
            // SAFETY: `out` NULL (rejected there) or a valid, unshared
            // `git_buf` (this export's contract) and our local string.
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
/// The module's contract; `out` NULL or writable.
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
            Ok(_) => {
                // The published pointer, with the allocation's provenance
                // (the caller frees through it; one made from the
                // `&GitConfig` may not be).
                let Some(c) = NonNull::new(r.config_ptr()) else {
                    return -1;
                };
                // SAFETY: the repository's live config (just published;
                // the repository's count keeps it live): a new reference
                // for the caller.
                let owned = unsafe { Config::new_ref(c) };
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
/// The module's contract; `out` NULL (rejected) or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_config_snapshot(
    out: *mut *mut GitConfig,
    repo_: Option<&GitRepository>,
) -> c_int {
    // SAFETY: `out` NULL or writable, with no reference to its pointee
    // (this export's contract).
    let mut out = unsafe { Out::from_ptr(out) };
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
        out.write(ptr::null_mut());
        match weak.snapshot() {
            Ok(snap) => {
                // The caller owns the new config.
                out.write(Config::from_box(snap).into_raw().as_ptr());
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
        // SAFETY: `config` is NULL or a live object on which the caller holds
        // a reference (this export's contract); no weak pointer to the old
        // one is used after the call and no borrow of it is live (the
        // module's contract for the replacing setters), as `set_config`
        // requires.
        unsafe { r.set_config(NonNull::new(config)) };
        0
    })
}

/// `git_repository_odb__weakptr`.
///
/// # Safety
///
/// The module's contract; `out` NULL (rejected) or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_odb__weakptr(
    out: *mut *mut GitOdb,
    repo_: Option<&GitRepository>,
) -> c_int {
    // SAFETY: `out` NULL or writable, with no reference to its pointee
    // (this export's contract).
    let mut out = unsafe { Out::from_ptr(out) };
    // The fast path: already created. Loads and a store only (nothing
    // can panic or allocate), so no barrier.
    if !out.is_null()
        && let Some(r) = repo_
    {
        let p = r.odb_ptr();
        if !p.is_null() {
            out.write(p);
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
                out.write(p.as_ptr());
                0
            }
            Err(e) => {
                out.write(ptr::null_mut());
                e.raw()
            }
        }
    })
}

/// `git_repository_odb`.
///
/// # Safety
///
/// The module's contract; `out` NULL (rejected) or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_odb(
    out: *mut *mut GitOdb,
    repo_: Option<&GitRepository>,
) -> c_int {
    // SAFETY: `out` NULL or writable, with no reference to its pointee
    // (this export's contract).
    let mut out = unsafe { Out::from_ptr(out) };
    barrier(|| {
        // `git_repository_odb__weakptr`, then a count for the caller.
        let Some(r) = repo_ else {
            let _ = errors::assert_arg_failed("repo");
            return -1;
        };
        git_assert_arg!(!out.is_null(), "out", -1);
        match r.odb_weak() {
            Ok(p) => {
                // SAFETY: the repository's live odb (its count keeps it
                // live for the call); the new count goes to the caller.
                unsafe { ffibox::CShared::increment_count(p.as_ptr()) };
                out.write(p.as_ptr());
                0
            }
            Err(_) => {
                out.write(ptr::null_mut());
                -1
            }
        }
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
        // SAFETY: `odb` is NULL or a live object on which the caller holds
        // a reference (this export's contract); no weak pointer to the old
        // one is used after the call and no borrow of it is live (the
        // module's contract for the replacing setters), as `set_odb`
        // requires.
        unsafe { r.set_odb(NonNull::new(odb)) };
        0
    })
}

/// `git_repository_refdb__weakptr`.
///
/// # Safety
///
/// The module's contract; `out` NULL (rejected) or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_refdb__weakptr(
    mut out: Out<'_, *mut GitRefdb>,
    repo_: Option<&GitRepository>,
) -> c_int {
    // The fast path: already created. Loads and a store only (nothing
    // can panic or allocate), so no barrier.
    if !out.is_null()
        && let Some(r) = repo_
    {
        let p = r.refdb_ptr();
        if !p.is_null() {
            out.write(p);
            return 0;
        }
    }
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        let Some(r) = repo_ else {
            return errors::assert_arg_failed("repo").raw();
        };
        let e = r.refdb_weak().err();
        out.write(r.refdb_ptr());
        e.map_or(0, ErrorCode::raw)
    })
}

/// `git_repository_refdb`.
///
/// # Safety
///
/// The module's contract; `out` NULL (rejected) or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_refdb(
    out: *mut *mut GitRefdb,
    repo_: Option<&GitRepository>,
) -> c_int {
    // SAFETY: `out` NULL or writable, with no reference to its pointee
    // (this export's contract).
    let mut out = unsafe { Out::from_ptr(out) };
    barrier(|| {
        // `git_repository_refdb__weakptr`, then a count for the caller.
        git_assert_arg!(!out.is_null(), "out", -1);
        let Some(r) = repo_ else {
            let _ = errors::assert_arg_failed("repo");
            return -1;
        };
        let e = r.refdb_weak();
        out.write(r.refdb_ptr());
        match e {
            Ok(p) => {
                // SAFETY: the repository's live refdb (its count keeps it
                // live for the call); the new count goes to the caller.
                unsafe { ffibox::CShared::increment_count(p.as_ptr()) };
                0
            }
            Err(_) => -1,
        }
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
        // SAFETY: `refdb` is NULL or a live object on which the caller holds
        // a reference (this export's contract); no weak pointer to the old
        // one is used after the call and no borrow of it is live (the
        // module's contract for the replacing setters), as `set_refdb`
        // requires.
        unsafe { r.set_refdb(NonNull::new(refdb)) };
        0
    })
}

/// `git_repository_index__weakptr`.
///
/// # Safety
///
/// The module's contract; `out` NULL (rejected) or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_index__weakptr(
    out: *mut *mut GitIndex,
    repo_: Option<&GitRepository>,
) -> c_int {
    // SAFETY: `out` NULL or writable, with no reference to its pointee
    // (this export's contract).
    let mut out = unsafe { Out::from_ptr(out) };
    // The fast path: already created. Loads and a store only (nothing
    // can panic or allocate), so no barrier.
    if !out.is_null()
        && let Some(r) = repo_
    {
        let p = r.index_ptr();
        if !p.is_null() {
            out.write(p);
            return 0;
        }
    }
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        let Some(r) = repo_ else {
            return errors::assert_arg_failed("repo").raw();
        };
        let e = r.index_weak().err();
        out.write(r.index_ptr());
        e.map_or(0, ErrorCode::raw)
    })
}

/// `git_repository_index`.
///
/// # Safety
///
/// The module's contract; `out` NULL (rejected) or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_index(
    out: *mut *mut GitIndex,
    repo_: Option<&GitRepository>,
) -> c_int {
    // SAFETY: `out` NULL or writable, with no reference to its pointee
    // (this export's contract).
    let mut out = unsafe { Out::from_ptr(out) };
    barrier(|| {
        // `git_repository_index__weakptr`, then a count for the caller.
        git_assert_arg!(!out.is_null(), "out", -1);
        let Some(r) = repo_ else {
            let _ = errors::assert_arg_failed("repo");
            return -1;
        };
        let e = r.index_weak();
        out.write(r.index_ptr());
        match e {
            Ok(p) => {
                // SAFETY: the repository's live index (its count keeps it
                // live for the call); the new count goes to the caller.
                unsafe { ffibox::CShared::increment_count(p.as_ptr()) };
                0
            }
            Err(_) => -1,
        }
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
        // SAFETY: `index` is NULL or a live object on which the caller holds
        // a reference (this export's contract); no weak pointer to the old
        // one is used after the call and no borrow of it is live (the
        // module's contract for the replacing setters), as `set_index`
        // requires.
        unsafe { r.set_index(NonNull::new(index)) };
        0
    })
}

/// `git_repository_grafts__weakptr`.
///
/// # Safety
///
/// The module's contract; `out` NULL (rejected) or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_grafts__weakptr(
    mut out: Out<'_, *mut crate::grafts::Grafts>,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        let r = repo_;
        git_assert_arg!(!out.is_null() && r.is_some(), "out && repo", -1);
        let Some(r) = r else { return -1 };
        let g = r.grafts_ptr();
        git_assert!(!g.is_null(), "repo->grafts", -1);
        out.write(g);
        0
    })
}

/// `git_repository_shallow_grafts__weakptr`.
///
/// # Safety
///
/// The module's contract; `out` NULL (rejected) or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_shallow_grafts__weakptr(
    mut out: Out<'_, *mut crate::grafts::Grafts>,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        let r = repo_;
        git_assert_arg!(!out.is_null() && r.is_some(), "out && repo", -1);
        let Some(r) = r else { return -1 };
        let g = r.shallow_grafts_ptr();
        git_assert!(!g.is_null(), "repo->shallow_grafts", -1);
        out.write(g);
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
                    // SAFETY: a string-replacing setter: no string borrowed
                    // from the field is used after the call (the module's
                    // contract), as `replace_string` requires.
                    unsafe { r.replace_string(StrField::Namespace, None) };
                    return -1;
                }
            },
        };
        // SAFETY: as above.
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
/// `out` and `out_len` are NULL (skipped; C writes them unchecked) or
/// writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository__extensions(
    out: *mut *mut *mut c_char,
    out_len: *mut usize,
) -> c_int {
    // SAFETY: NULL or writable, with no reference to the pointee (this
    // export's contract).
    let mut out = unsafe { Out::from_ptr(out) };
    // SAFETY: as above.
    let mut out_len = unsafe { Out::from_ptr(out_len) };
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
                // SAFETY: the first `i` slots of our array (`git__allocator`,
                // aligned for pointers) were written below; `i` <= the
                // array's length, a `Vec`'s length, so `<= isize::MAX`.
                let made = unsafe { core::slice::from_raw_parts(arr, i) };
                for p in made {
                    // SAFETY: a string made above, freed once.
                    unsafe { crate::util::alloc::free(p.cast()) };
                }
                // SAFETY: the array made above, unused after, freed once.
                unsafe { crate::util::alloc::free(arr.cast()) };
                return -1;
            }
            // SAFETY: `i < exts.len()`, within the array of `exts.len()`
            // pointers, aligned (`git__allocator` memory), which nothing
            // else references.
            unsafe { arr.wrapping_add(i).write(s) };
        }
        out.write(arr);
        out_len.write(exts.len());
        0
    })
}

/// `git_repository__set_extensions`.
///
/// # Safety
///
/// `extensions` is NULL (with any `len`: none) or points to `len` aligned
/// pointers, each NULL or a NUL-terminated string, all unwritten during
/// the call; `len * size_of::<*const c_char>() <= isize::MAX`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository__set_extensions(
    extensions: *const *const c_char,
    len: usize,
) -> c_int {
    barrier(|| {
        let ptrs: &[*const c_char] = if len == 0 || extensions.is_null() {
            &[]
        } else {
            // SAFETY: `len` readable, aligned pointers at `extensions`,
            // within `isize::MAX` bytes and unwritten during the call (this
            // export's contract).
            unsafe { core::slice::from_raw_parts(extensions, len) }
        };
        let v: Vec<&[u8]> = ptrs
            .iter()
            // SAFETY: each NULL or a string unwritten during the call (this
            // export's contract).
            .map(|&p| unsafe { CStrArg::from_ptr(p) }.to_bytes())
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
/// `opts` is NULL (rejected) or writable for a `git_repository_init_options`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_init_options_init(
    mut opts: Out<'_, GitRepositoryInitOptions>,
    version: c_uint,
) -> c_int {
    barrier(|| {
        if let Err(e) = check_version(Some(version), 1, "git_repository_init_options") {
            return e.raw();
        }
        git_assert_arg!(!opts.is_null(), "opts", -1);
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
        });
        0
    })
}

/// `git_repository_init_ext`.
///
/// # Safety
///
/// `out` is NULL (rejected) or writable; `given_repo` NULL (rejected) or a
/// NUL-terminated string; `opts` NULL (rejected) or a valid options struct
/// whose strings are NULL or NUL-terminated, which nothing else uses during
/// the call (its `flags` are written back, as the C does).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_init_ext(
    mut out: Out<'_, *mut GitRepository>,
    given_repo: CStrArg<'_>,
    opts: Option<&mut GitRepositoryInitOptions>,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        let Some(given) = given_repo.get() else {
            return errors::assert_arg_failed("given_repo").raw();
        };
        let o = git_arg!(opts, "opts", -1);
        if let Err(e) = check_version(Some(o.version), 1, "git_repository_init_options") {
            return e.raw();
        }
        // SAFETY: each string field is NULL or a NUL-terminated string,
        // unwritten during the call (this export's contract).
        let own = |p: *const c_char| unsafe { CStrArg::from_ptr(p) }.get().map(CStr::to_owned);
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
        let (r, res) = init::init_ext(given, &mut io);
        // C works on the caller's struct: its flags gain the internal bits.
        o.flags = io.flags;
        match res {
            InitOut::Unset => {}
            InitOut::Null => out.write(ptr::null_mut()),
            InitOut::Repo(p) => out.write(p.as_ptr()),
        }
        status(r)
    })
}

/// `git_repository_init`.
///
/// # Safety
///
/// `out` is NULL (rejected) or writable; `path` NULL (rejected) or a
/// NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_init(
    out: Out<'_, *mut GitRepository>,
    path: CStrArg<'_>,
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
    // SAFETY: `out` and `path` as `git_repository_init_ext` takes them (this
    // export's contract), and our local options (NULL strings), borrowed
    // for the call only.
    unsafe { git_repository_init_ext(out, path, Some(&mut opts)) }
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
/// The module's contract; `name` NULL (rejected) or a NUL-terminated
/// string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_head_detached_for_worktree(
    repo_: Option<&GitRepository>,
    name: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            let Some(n) = name.get() else {
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
/// The module's contract; `head_out` NULL (rejected) or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_head(
    head_out: *mut *mut c_void,
    repo_: Option<&GitRepository>,
) -> c_int {
    // SAFETY: `head_out` NULL or writable, with no reference to its pointee
    // (this export's contract).
    let mut head_out = unsafe { Out::from_ptr(head_out) };
    barrier(|| {
        git_assert_arg!(!head_out.is_null(), "head_out", -1);
        with_repo!(repo_, "repo", |r| match head::head(r) {
            Ok(h) => {
                head_out.write(h.into_raw().cast());
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
/// The module's contract; `out` NULL (rejected) or writable; `name` NULL
/// (rejected) or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_head_for_worktree(
    mut out: Out<'_, *mut c_void>,
    repo_: Option<&GitRepository>,
    name: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        with_repo!(repo_, "repo", |r| {
            let Some(n) = name.get() else {
                return errors::assert_arg_failed("name").raw();
            };
            out.write(ptr::null_mut());
            match head::with_head_for_worktree(r, n, |h| h.into_raw().cast()) {
                Ok(h) => {
                    out.write(h);
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
/// The module's contract; `cb` is sound to call with a live repository
/// (valid during the call) and `payload`, which is passed through unread.
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
                // SAFETY: the callback with what this export's contract
                // allows: `w`, a repository open for the call (only read
                // through `&`, its state interior-mutable), and `payload`.
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
            // SAFETY: this export's contract on `out` is `buf_wrap`'s.
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
        pub unsafe extern "C" fn $name(repo_: Option<&GitRepository>) -> *const c_char {
            if let Some(r) = repo_ {
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
/// The module's contract (a string-replacing setter); `workdir` NULL
/// (rejected) or a NUL-terminated string (it may be the current workdir).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_set_workdir(
    repo_: Option<&GitRepository>,
    workdir: CStrArg<'_>,
    update_gitlink: c_int,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            // It may be the current workdir: copied before it is replaced.
            let Some(w) = workdir.get().map(CStr::to_owned) else {
                return errors::assert_arg_failed("workdir").raw();
            };
            // SAFETY: a string-replacing setter, which this export's
            // contract (the module's) keeps from running beside any other
            // use of the repository or of a string it returned.
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
            // SAFETY: as in `git_repository_set_workdir`.
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
/// The module's contract; `orig_head` NULL (rejected) or a valid id,
/// unwritten during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository__set_orig_head(
    repo_: Option<&GitRepository>,
    orig_head: Option<&Oid>,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            let Some(id) = orig_head else {
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
/// The module's contract; `out` NULL (rejected) or a valid `git_buf` that
/// nothing else uses during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_message(
    out: *mut GitBuf,
    repo_: Option<&GitRepository>,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            // SAFETY: this export's contract on `out` is `buf_wrap`'s.
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
/// The module's contract; `out` NULL (rejected) or writable; the strings
/// NULL or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_hashfile(
    mut out: Out<'_, Oid>,
    repo_: Option<&GitRepository>,
    path: CStrArg<'_>,
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
        let a = as_path.get();
        let Some(p) = path.get() else {
            return errors::assert_arg_failed("path").raw();
        };
        let Some(r) = r else {
            return errors::assert_arg_failed("repo").raw();
        };
        match head::hashfile(r, p, object_type, a) {
            Ok(id) => {
                out.write(id);
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
/// The module's contract; `refname` NULL (rejected) or a NUL-terminated
/// string (it may be a reference's own name).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_set_head(
    repo_: Option<&GitRepository>,
    refname: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            let Some(n) = refname.get().map(CStr::to_owned) else {
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
/// The module's contract; `committish` NULL or a valid id, readable at the
/// start of the call (it is copied then).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_set_head_detached(
    repo_: Option<&GitRepository>,
    committish: *const Oid,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            // SAFETY: NULL or a valid id (align 1) readable now (this
            // export's contract); copied, so no reference outlives this
            // read.
            let id = unsafe { committish.as_ref() }.copied();
            status(head::set_head_detached(r, id.as_ref()))
        })
    })
}

/// `git_repository_set_head_detached_from_annotated`.
///
/// # Safety
///
/// The module's contract; `committish` NULL (rejected) or a live annotated
/// commit, not freed during the call.
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
            // SAFETY: a live annotated commit for the call (this export's
            // contract), as `set_head_detached_from_annotated` takes it.
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
/// The module's contract; `files` NULL (with any `files_len`: none) or
/// `files_len` aligned pointers, each NULL or a NUL-terminated string, all
/// unwritten during the call, within `isize::MAX` bytes.
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
                // SAFETY: `files_len` readable, aligned pointers, within
                // `isize::MAX` bytes and unwritten during the call (this
                // export's contract).
                unsafe { core::slice::from_raw_parts(files, files_len) }
            };
            let v: Vec<&[u8]> = ptrs
                .iter()
                // SAFETY: each NULL or a string unwritten during the call
                // (this export's contract).
                .map(|&p| unsafe { CStrArg::from_ptr(p) }.to_bytes())
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
/// The module's contract; `roots` NULL (rejected) or a valid
/// `git_oidarray` (`count` ids at `ids`), unwritten during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository__shallow_roots_write(
    repo_: Option<&GitRepository>,
    roots: Option<&GitOidArray>,
) -> c_int {
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            let Some(roots) = roots else {
                return errors::assert_arg_failed("roots").raw();
            };
            let ids: &[Oid] = if roots.count == 0 {
                &[]
            } else {
                // SAFETY: a valid array (this export's contract): `count`
                // readable ids (align 1) at `ids`, unwritten during the
                // call; `count * size_of::<Oid>()` is the size of a live
                // allocation, so `<= isize::MAX`.
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
            // SAFETY: no string `git_repository_ident` returned is used
            // after the call (the module's contract for the replacing
            // setters).
            status(unsafe { r.set_ident(n.as_deref(), e.as_deref()) })
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
    barrier(|| {
        with_repo!(repo_, "repo", |r| {
            // SAFETY: nothing else uses the submodule cache during the call
            // (the module's contract for the replacing setters).
            unsafe { r.submodule_cache_clear() }
        })
    })
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
/// The module's contract; `out` NULL (rejected) or writable for a
/// `git_commitarray` (its old contents are not read).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_repository_commit_parents(
    out: *mut GitCommitArray,
    repo_: Option<&GitRepository>,
) -> c_int {
    // SAFETY: `out` NULL or writable, with no reference to its pointee
    // (this export's contract).
    let mut out = unsafe { Out::from_ptr(out) };
    barrier(|| {
        let r = repo_;
        git_assert_arg!(!out.is_null() && r.is_some(), "out && repo", -1);
        let Some(r) = r else { return -1 };
        out.write(GitCommitArray {
            commits: ptr::null_mut(),
            count: 0,
        });
        match head::commit_parents(r) {
            Ok(v) => {
                if v.is_empty() {
                    return 0;
                }
                let mut v = core::mem::ManuallyDrop::new(v);
                let (p, n) = (v.as_mut_ptr(), v.len());
                // The array is `git__allocator`'s (a `Vec` in `GitAlloc`),
                // which `git_commitarray_dispose` frees.
                out.write(GitCommitArray {
                    commits: p.cast(),
                    count: n,
                });
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
            // SAFETY: no weak pointer of the old index, odb or refdb is used
            // after the call (the module's contract for the replacing
            // setters), as `set_objectformat` requires.
            status(unsafe { head::set_objectformat(r, oid_type) })
        })
    })
}
