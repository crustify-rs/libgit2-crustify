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
use core::ffi::c_int;
use core::sync::atomic::AtomicBool;

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
    /// `git_repository__configmap_lookup` (repository.c). PENDING(l-repo)
    fn git_repository__configmap_lookup(out: *mut c_int, repo: *mut c_void, item: c_int) -> c_int;

    /// `git_repository__fsync_gitdir` (repository.c; settings.c writes it).
    /// PENDING(l-repo)
    static git_repository__fsync_gitdir: AtomicBool;

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
// PENDING(l-refs): `src/libgit2/refdb.c`, `refs.c`, for the config modules.
// ---------------------------------------------------------------------------

unsafe extern "C" {
    /// `git_refdb_lookup`. PENDING(l-refs)
    fn git_refdb_lookup(
        out: *mut *mut c_void,
        refdb: *mut c_void,
        ref_name: *const core::ffi::c_char,
    ) -> core::ffi::c_int;
}

/// The symbolic target of the repository's `HEAD`, read from its refdb
/// directly (`None` if `HEAD` is not symbolic): config_file.c's
/// `conditional_match_onbranch`. PENDING(l-refs)
pub fn repository_head_symbolic_target(
    repo: &crate::repository::GitRepository,
) -> crate::util::errors::Result<Option<Vec<u8>>> {
    use crate::util::errors::ErrorCode;
    use l_repo::{git_reference_free, git_reference_symbolic_target, git_reference_type};
    // C takes a reference (`git_repository_refdb`) and drops it at the
    // end; the repository keeps the refdb live meanwhile either way.
    let refdb = repo.refdb_weak()?;
    let mut r = core::ptr::null_mut();
    // SAFETY: `r` is writable; `refdb` is the repository's live refdb; the
    // name is NUL-terminated.
    ErrorCode::check(unsafe { git_refdb_lookup(&raw mut r, refdb.as_ptr(), c"HEAD".as_ptr()) })?;
    // SAFETY: `r` is the live reference just returned.
    let result =
        if unsafe { git_reference_type(r) } != crate::repository::handles::GIT_REFERENCE_SYMBOLIC {
            None
        } else {
            // SAFETY: as above; the target is NUL-terminated (or NULL) and
            // lives as long as `r`, which it is copied before.
            let p = unsafe { git_reference_symbolic_target(r) };
            Some(if p.is_null() {
                Vec::new()
            } else {
                // SAFETY: as above.
                unsafe { core::ffi::CStr::from_ptr(p) }.to_bytes().to_vec()
            })
        };
    // SAFETY: `r` is ours, released once.
    unsafe { git_reference_free(r) };
    Ok(result)
}

// PENDING(l-obj), PENDING(l-commit): what the index (WP l-index) calls of
// `blob.c`, `object.c` and `tree.c`. Safe wrappers: `crate::index::foreign`.
pub mod l_index {
    use core::ffi::{c_char, c_int, c_uint, c_void};

    use crate::oid::Oid;

    /// `git_treewalk_cb`.
    pub type TreewalkCb = unsafe extern "C" fn(
        root: *const c_char,
        entry: *const c_void,
        payload: *mut c_void,
    ) -> c_int;

    unsafe extern "C" {

        /// `git_blob__create_from_paths`. PENDING(l-obj)
        pub fn git_blob__create_from_paths(
            out_oid: *mut Oid,
            out_st: *mut libc::stat,
            repo: *mut c_void,
            full_path: *const c_char,
            hint_path: *const c_char,
            hint_mode: libc::mode_t,
            apply_filters: bool,
        ) -> c_int;
        /// `git_blob_create_from_buffer`. PENDING(l-obj)
        pub fn git_blob_create_from_buffer(
            id: *mut Oid,
            repo: *mut c_void,
            buffer: *const c_void,
            len: usize,
        ) -> c_int;
        /// `git_object__is_valid`. PENDING(l-obj)
        pub fn git_object__is_valid(
            repo: *mut c_void,
            id: *const Oid,
            expected_type: c_int,
        ) -> bool;

        /// `git_tree__write_index`. PENDING(l-commit)
        pub fn git_tree__write_index(oid: *mut Oid, index: *mut c_void, repo: *mut c_void)
        -> c_int;
        /// `git_tree_walk`. PENDING(l-commit)
        pub fn git_tree_walk(
            tree: *const c_void,
            mode: c_int,
            callback: TreewalkCb,
            payload: *mut c_void,
        ) -> c_int;
        /// `git_tree_owner` (`object_api.c`). PENDING(l-obj)
        pub fn git_tree_owner(tree: *const c_void) -> *mut c_void;
        /// `git_tree_id` (`object_api.c`). PENDING(l-obj)
        pub fn git_tree_id(tree: *const c_void) -> *const Oid;
        /// `git_tree_entrycount`. PENDING(l-commit)
        pub fn git_tree_entrycount(tree: *const c_void) -> usize;
        /// `git_tree_entry_byindex`. PENDING(l-commit)
        pub fn git_tree_entry_byindex(tree: *const c_void, idx: usize) -> *const c_void;
        /// `git_tree_entry_filemode`. PENDING(l-commit)
        pub fn git_tree_entry_filemode(entry: *const c_void) -> c_uint;
        /// `git_tree_entry_filemode_raw`. PENDING(l-commit)
        pub fn git_tree_entry_filemode_raw(entry: *const c_void) -> c_uint;
        /// `git_tree_entry_name`. PENDING(l-commit)
        pub fn git_tree_entry_name(entry: *const c_void) -> *const c_char;
        /// `git_tree_entry_id`. PENDING(l-commit)
        pub fn git_tree_entry_id(entry: *const c_void) -> *const Oid;
        /// `git_tree_lookup` (`object_api.c`). PENDING(l-obj)
        pub fn git_tree_lookup(out: *mut *mut c_void, repo: *mut c_void, id: *const Oid) -> c_int;
        /// `git_tree_free` (`object_api.c`). PENDING(l-obj)
        pub fn git_tree_free(tree: *mut c_void);
    }
/// `GIT_CONFIGMAP_FSYNCOBJECTFILES` (repository.h).
pub const GIT_CONFIGMAP_FSYNCOBJECTFILES: c_int = 13;

/// `git_repository__configmap_lookup(&out, repo, item)`, PENDING(l-repo).
///
/// # Safety
///
/// `repo` is a live `git_repository *`.
pub unsafe fn repository_configmap_lookup(
    repo: *mut c_void,
    item: c_int,
) -> crate::util::errors::Result<c_int> {
    let mut out = 0;
    // SAFETY: the caller's contract; `out` is writable.
    let r = unsafe { git_repository__configmap_lookup(&raw mut out, repo, item) };
    crate::util::errors::ErrorCode::check_zero(r).map(|()| out)
}

/// `git_repository__fsync_gitdir` (`GIT_OPT_ENABLE_FSYNC_GITDIR`),
/// PENDING(l-repo).
pub fn repository_fsync_gitdir() -> bool {
    // SAFETY: a C `bool` global, read atomically (C writes it as a plain
    // store; an atomic of the same size and alignment).
    unsafe { &git_repository__fsync_gitdir }.load(core::sync::atomic::Ordering::Relaxed)
}
