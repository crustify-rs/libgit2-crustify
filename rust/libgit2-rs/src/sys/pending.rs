//! Functions of modules that another work package is porting concurrently,
//! called through their C symbols until both have landed (PORTING.md §3).
//! Each entry carries a `// PENDING(<wp>)` comment.

// PENDING(l-config, l-odb, l-pack, l-refs, l-index, l-obj, l-commit): what
// the repository (WP l-repo) calls of them.
pub mod l_repo;
use core::ffi::c_void;

// ---------------------------------------------------------------------------
// WP l-odb (odb, odb_loose, odb_mempack, cache)
// ---------------------------------------------------------------------------

use core::ffi::{c_char, c_int};

// `git_odb_backend` holds a `git_odb *`, opaque to C (the Rust's).
#[allow(improper_ctypes)]
unsafe extern "C" {
    /// `git_object__free` (object.c). PENDING(l-obj)
    fn git_object__free(object: *mut c_void);

    /// `git_odb_backend_pack` (odb_pack.c). PENDING(l-pack)
    fn git_odb_backend_pack(
        out: *mut *mut crate::odb::GitOdbBackend,
        objects_dir: *const c_char,
        opts: *const crate::odb::GitOdbBackendPackOptions,
    ) -> c_int;
}

/// Free a parsed object (`git_object__free`), PENDING(l-obj).
///
/// # Safety
///
/// `object` is a `git_object` whose last reference was just dropped.
pub unsafe fn object_free(object: core::ptr::NonNull<c_void>) {
    // SAFETY: the caller's contract.
    unsafe { git_object__free(object.as_ptr()) }
}

/// `git_odb_backend_pack(&out, objects_dir, opts)`, PENDING(l-pack).
pub fn odb_backend_pack(
    objects_dir: &core::ffi::CStr,
    opts: &crate::odb::GitOdbBackendPackOptions,
) -> crate::util::errors::Result<core::ptr::NonNull<crate::odb::GitOdbBackend>> {
    let mut out = core::ptr::null_mut();
    // SAFETY: `out` is writable, `objects_dir` and `opts` are valid for the
    // call (C copies what it keeps).
    let r = unsafe { git_odb_backend_pack(&raw mut out, objects_dir.as_ptr(), opts) };
    crate::util::errors::ErrorCode::check_zero(r)?;
    core::ptr::NonNull::new(out).ok_or(crate::util::errors::ErrorCode::GENERIC)
}

// PENDING(l-pack): `src/libgit2/pack.c`. `git_delta_read_header_fromstream`
// reads a delta's header through a pack stream (`git_packfile_stream *`,
// opaque here).
unsafe extern "C" {
    /// `git_packfile_stream_read`. PENDING(l-pack)
    pub fn git_packfile_stream_read(obj: *mut c_void, buffer: *mut c_void, len: usize) -> isize;
}

// ---------------------------------------------------------------------------
// PENDING(l-repo): `src/libgit2/repository.c`, for the config modules.
// ---------------------------------------------------------------------------

/// `git_repository`, opaque here. PENDING(l-repo)
#[repr(C)]
pub struct git_repository {
    _private: [u8; 0],
}

/// `git_refdb`, opaque here. PENDING(l-refs)
#[repr(C)]
pub struct git_refdb {
    _private: [u8; 0],
}

/// `git_reference`, opaque here. PENDING(l-refs)
#[repr(C)]
pub struct git_reference {
    _private: [u8; 0],
}

/// `GIT_REPOSITORY_ITEM_GITDIR`.
pub const GIT_REPOSITORY_ITEM_GITDIR: core::ffi::c_int = 0;

/// `GIT_REFERENCE_SYMBOLIC`.
pub const GIT_REFERENCE_SYMBOLIC: core::ffi::c_int = 2;

/// `GIT_CONFIGMAP_CACHE_MAX`: the length of `git_repository`'s
/// `configmap_cache`.
pub const GIT_CONFIGMAP_CACHE_MAX: usize = 15;

/// `offsetof(git_repository, configmap_cache)`, as the header lays it out
/// (checked by `config_cache`'s layout test). PENDING(l-repo): the
/// repository WP owns the struct; the integration replaces this accessor
/// with a field of its Rust type.
pub const GIT_REPOSITORY_CONFIGMAP_CACHE_OFFSET: usize = 272;

/// `repo->configmap_cache`: the repository's cache of configmap values,
/// which C reads and writes with atomic operations (`intptr_t` slots).
/// PENDING(l-repo)
///
/// # Safety
///
/// `repo` is a live repository.
pub unsafe fn repository_configmap_cache<'a>(
    repo: *mut git_repository,
) -> &'a [core::sync::atomic::AtomicIsize; GIT_CONFIGMAP_CACHE_MAX] {
    // The cache is an array of `intptr_t` at this offset (layout test),
    // which C accesses only atomically (`git_atomic_load`,
    // `git_atomic_compare_and_swap`) or while nothing else uses the
    // repository (`git_repository_new`/`_free`), so an array of atomics over
    // it is sound; it lives as long as `repo`.
    // SAFETY: the repository is live (the contract) and the field is within
    // it.
    let field = unsafe { repo.byte_add(GIT_REPOSITORY_CONFIGMAP_CACHE_OFFSET) };
    // The field is an `intptr_t` array, so aligned as one (layout test).
    #[allow(clippy::cast_ptr_alignment)]
    let cache = field.cast::<[core::sync::atomic::AtomicIsize; GIT_CONFIGMAP_CACHE_MAX]>();
    // SAFETY: as above: a live, aligned array that C only accesses
    // atomically.
    unsafe { &*cache }
}

unsafe extern "C" {
    /// `git_repository_config__weakptr`. PENDING(l-repo)
    pub fn git_repository_config__weakptr(
        out: *mut *mut core::ffi::c_void,
        repo: *mut git_repository,
    ) -> core::ffi::c_int;
    /// `git_repository__item_path`. PENDING(l-repo)
    pub fn git_repository__item_path(
        out: *mut crate::util::str::GitStr,
        repo: *const git_repository,
        item: core::ffi::c_int,
    ) -> core::ffi::c_int;
    /// `git_repository_refdb`. PENDING(l-repo)
    pub fn git_repository_refdb(
        out: *mut *mut git_refdb,
        repo: *mut git_repository,
    ) -> core::ffi::c_int;
}

// ---------------------------------------------------------------------------
// PENDING(l-refs): `src/libgit2/refdb.c`, `refs.c`, for the config modules.
// ---------------------------------------------------------------------------

unsafe extern "C" {
    /// `git_refdb_lookup`. PENDING(l-refs)
    pub fn git_refdb_lookup(
        out: *mut *mut git_reference,
        refdb: *mut git_refdb,
        ref_name: *const core::ffi::c_char,
    ) -> core::ffi::c_int;
    /// `git_refdb_free`. PENDING(l-refs)
    pub fn git_refdb_free(refdb: *mut git_refdb);
    /// `git_reference_type`. PENDING(l-refs)
    pub fn git_reference_type(r: *const git_reference) -> core::ffi::c_int;
    /// `git_reference_symbolic_target`. PENDING(l-refs)
    pub fn git_reference_symbolic_target(r: *const git_reference) -> *const core::ffi::c_char;
    /// `git_reference_free`. PENDING(l-refs)
    pub fn git_reference_free(r: *mut git_reference);
}

/// The repository's git directory (`git_repository__item_path(...,
/// GIT_REPOSITORY_ITEM_GITDIR)`). PENDING(l-repo)
///
/// # Safety
///
/// `repo` is a live repository.
pub unsafe fn repository_gitdir(
    out: &mut crate::util::str::GitStr,
    repo: *const git_repository,
) -> crate::util::errors::Result<()> {
    // SAFETY: `out` is an exclusive `git_str`; `repo` is live (contract).
    let r = unsafe { git_repository__item_path(out, repo, GIT_REPOSITORY_ITEM_GITDIR) };
    crate::util::errors::ErrorCode::check(r).map(drop)
}

/// The repository's config, borrowed (`git_repository_config__weakptr`).
/// PENDING(l-repo)
///
/// # Safety
///
/// `repo` is a live repository.
pub unsafe fn repository_config_weakptr(
    repo: *mut git_repository,
) -> crate::util::errors::Result<*mut crate::config::GitConfig> {
    let mut out = core::ptr::null_mut();
    // SAFETY: `out` is writable; `repo` is live (contract).
    let r = unsafe { git_repository_config__weakptr(&raw mut out, repo) };
    crate::util::errors::ErrorCode::check(r)?;
    Ok(out.cast())
}

/// The symbolic target of the repository's `HEAD`, read from its refdb
/// directly (`None` if `HEAD` is not symbolic). PENDING(l-repo, l-refs)
///
/// # Safety
///
/// `repo` is a live repository.
pub unsafe fn repository_head_symbolic_target(
    repo: *mut git_repository,
) -> crate::util::errors::Result<Option<Vec<u8>>> {
    use crate::util::errors::ErrorCode;
    let mut refdb = core::ptr::null_mut();
    // SAFETY: `refdb` is writable; `repo` is live (contract).
    ErrorCode::check(unsafe { git_repository_refdb(&raw mut refdb, repo) })?;
    let mut r = core::ptr::null_mut();
    // SAFETY: `refdb` is the live refdb just returned; the name is
    // NUL-terminated.
    let e = unsafe { git_refdb_lookup(&raw mut r, refdb, c"HEAD".as_ptr()) };
    let result = if e < 0 {
        Err(ErrorCode::check(e).unwrap_err())
    } else {
        // SAFETY: `r` is the live reference just returned.
        let t = unsafe { git_reference_type(r) };
        if t != GIT_REFERENCE_SYMBOLIC {
            Ok(None)
        } else {
            // SAFETY: as above; the target is NUL-terminated (or NULL) and
            // lives as long as `r`, which it is copied before.
            let p = unsafe { git_reference_symbolic_target(r) };
            Ok(Some(if p.is_null() {
                Vec::new()
            } else {
                // SAFETY: as above.
                unsafe { core::ffi::CStr::from_ptr(p) }.to_bytes().to_vec()
            }))
        }
    };
    // SAFETY: `r` (NULL on failure) and `refdb` are ours, released once.
    unsafe {
        git_reference_free(r);
    }
    // SAFETY: as above.
    unsafe { git_refdb_free(refdb) };
    result
}
