//! Functions of modules that another work package is porting concurrently,
//! called through their C symbols until both have landed (PORTING.md §3).
//! Each entry carries a `// PENDING(<wp>)` comment.

use core::ffi::c_void;

// PENDING(pool): `src/util/pool.c` (in PORTING.md §1's list; not yet
// ported). `git_futils_mkdir_relative`'s path cache allocates from the
// caller's (C-owned) pool.
unsafe extern "C" {
    /// `git_pool_malloc`. PENDING(pool)
    pub fn git_pool_malloc(pool: *mut c_void, items: usize) -> *mut c_void;
}

// ---------------------------------------------------------------------------
// WP l-obj (object, object_api, blob, tag, signature, grafts): the functions
// of the modules other L WPs port concurrently. The opaque types stand for
// the C structs those WPs own; the integration step replaces each with the
// owning module's Rust type.
// ---------------------------------------------------------------------------

use core::ffi::{c_char, c_int};

use crate::oid::Oid;
use crate::util::str::GitStr;

/// Declares opaque C types (pointers only; PENDING(<owner>)).
macro_rules! opaque_c_types {
    ($($(#[$m:meta])* $name:ident;)*) => {$(
        $(#[$m])*
        #[repr(C)]
        #[allow(non_camel_case_types)]
        pub struct $name {
            _private: [u8; 0],
            _not_send_sync: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
        }
    )*};
}

opaque_c_types! {
    /// `git_repository`. PENDING(l-repo)
    git_repository;
    /// `git_odb`. PENDING(l-odb)
    git_odb;
    /// `git_odb_object` (its `git_cached_obj` prefix is read through
    /// `object::GitCachedObj`). PENDING(l-odb)
    git_odb_object;
    /// `git_odb_stream`. PENDING(l-odb)
    git_odb_stream;
    /// `git_cache`. PENDING(l-odb)
    git_cache;
    /// `git_config`. PENDING(l-config)
    git_config;
    /// `git_reference`. PENDING(l-refs)
    git_reference;
    /// `git_tree_entry`. PENDING(l-commit)
    git_tree_entry;
}

/// `git_reference_foreach_name_cb`. PENDING(l-refs)
#[allow(non_camel_case_types)]
pub type git_reference_foreach_name_cb =
    Option<unsafe extern "C" fn(name: *const c_char, payload: *mut c_void) -> c_int>;

// PENDING(l-odb): cache.c and odb.c.
unsafe extern "C" {
    /// `git_cache_get_any`: the cached object (a new reference) or NULL.
    /// PENDING(l-odb)
    pub fn git_cache_get_any(cache: *mut git_cache, oid: *const Oid) -> *mut c_void;
    /// `git_cache_store_parsed`: store `entry` (taking a reference), and
    /// return the cached object (a new reference). PENDING(l-odb)
    pub fn git_cache_store_parsed(cache: *mut git_cache, entry: *mut c_void) -> *mut c_void;
    /// `git_cached_obj_decref`. PENDING(l-odb)
    pub fn git_cached_obj_decref(obj: *mut c_void);
    /// `git_odb_read`. PENDING(l-odb)
    pub fn git_odb_read(out: *mut *mut git_odb_object, db: *mut git_odb, id: *const Oid) -> c_int;
    /// `git_odb_read_prefix`. PENDING(l-odb)
    pub fn git_odb_read_prefix(
        out: *mut *mut git_odb_object,
        db: *mut git_odb,
        short_id: *const Oid,
        len: usize,
    ) -> c_int;
    /// `git_odb_read_header`. PENDING(l-odb)
    pub fn git_odb_read_header(
        len_out: *mut usize,
        type_out: *mut c_int,
        db: *mut git_odb,
        id: *const Oid,
    ) -> c_int;
    /// `git_odb_exists_prefix`. PENDING(l-odb)
    pub fn git_odb_exists_prefix(
        out: *mut Oid,
        db: *mut git_odb,
        short_id: *const Oid,
        len: usize,
    ) -> c_int;
    /// `git_odb_object_free`. PENDING(l-odb)
    pub fn git_odb_object_free(object: *mut git_odb_object);
    /// `git_odb_object_data`. PENDING(l-odb)
    pub fn git_odb_object_data(object: *mut git_odb_object) -> *const c_void;
    /// `git_odb_free`. PENDING(l-odb)
    pub fn git_odb_free(db: *mut git_odb);
    /// `git_odb_open_wstream`. PENDING(l-odb)
    pub fn git_odb_open_wstream(
        out: *mut *mut git_odb_stream,
        db: *mut git_odb,
        size: u64,
        kind: c_int,
    ) -> c_int;
    /// `git_odb_stream_write`. PENDING(l-odb)
    pub fn git_odb_stream_write(
        stream: *mut git_odb_stream,
        buffer: *const c_char,
        len: usize,
    ) -> c_int;
    /// `git_odb_stream_finalize_write`. PENDING(l-odb)
    pub fn git_odb_stream_finalize_write(out: *mut Oid, stream: *mut git_odb_stream) -> c_int;
    /// `git_odb_stream_free`. PENDING(l-odb)
    pub fn git_odb_stream_free(stream: *mut git_odb_stream);
    /// `git_odb_write`. PENDING(l-odb)
    pub fn git_odb_write(
        out: *mut Oid,
        db: *mut git_odb,
        data: *const c_void,
        len: usize,
        kind: c_int,
    ) -> c_int;
}

// PENDING(l-repo): repository.c.
unsafe extern "C" {
    /// `git_repository_open`. PENDING(l-repo)
    pub fn git_repository_open(out: *mut *mut git_repository, path: *const c_char) -> c_int;
    /// `git_repository_free`. PENDING(l-repo)
    pub fn git_repository_free(repo: *mut git_repository);
    /// `git_repository_odb__weakptr`. PENDING(l-repo)
    pub fn git_repository_odb__weakptr(out: *mut *mut git_odb, repo: *mut git_repository) -> c_int;
    /// `git_repository_odb`. PENDING(l-repo)
    pub fn git_repository_odb(out: *mut *mut git_odb, repo: *mut git_repository) -> c_int;
    /// `git_repository__abbrev_length`. PENDING(l-repo)
    pub fn git_repository__abbrev_length(out: *mut c_int, repo: *mut git_repository) -> c_int;
    /// `git_repository_config_snapshot`. PENDING(l-repo)
    pub fn git_repository_config_snapshot(
        out: *mut *mut git_config,
        repo: *mut git_repository,
    ) -> c_int;
    /// `git_repository_workdir_path`. PENDING(l-repo)
    pub fn git_repository_workdir_path(
        out: *mut GitStr,
        repo: *mut git_repository,
        path: *const c_char,
    ) -> c_int;
    /// `git_repository_workdir`. PENDING(l-repo)
    pub fn git_repository_workdir(repo: *const git_repository) -> *const c_char;
    /// `git_repository__item_path`. PENDING(l-repo)
    pub fn git_repository__item_path(
        out: *mut GitStr,
        repo: *const git_repository,
        item: c_int,
    ) -> c_int;
}

// PENDING(l-config): config.c.
unsafe extern "C" {
    /// `git_config_get_string`. PENDING(l-config)
    pub fn git_config_get_string(
        out: *mut *const c_char,
        cfg: *const git_config,
        name: *const c_char,
    ) -> c_int;
    /// `git_config_free`. PENDING(l-config)
    pub fn git_config_free(cfg: *mut git_config);
}

// PENDING(l-commit): commit.c and tree.c (the object table's commit and
// tree entries, and what peeling and path lookup need).
unsafe extern "C" {
    /// `git_commit__parse`. PENDING(l-commit)
    pub fn git_commit__parse(
        commit: *mut c_void,
        obj: *mut git_odb_object,
        oid_type: crate::oid::git_oid_t,
    ) -> c_int;
    /// `git_commit__parse_raw`. PENDING(l-commit)
    pub fn git_commit__parse_raw(
        commit: *mut c_void,
        data: *const c_char,
        size: usize,
        oid_type: crate::oid::git_oid_t,
    ) -> c_int;
    /// `git_commit__free`. PENDING(l-commit)
    pub fn git_commit__free(commit: *mut c_void);
    /// `git_tree__parse`. PENDING(l-commit)
    pub fn git_tree__parse(
        tree: *mut c_void,
        obj: *mut git_odb_object,
        oid_type: crate::oid::git_oid_t,
    ) -> c_int;
    /// `git_tree__parse_raw`. PENDING(l-commit)
    pub fn git_tree__parse_raw(
        tree: *mut c_void,
        data: *const c_char,
        size: usize,
        oid_type: crate::oid::git_oid_t,
    ) -> c_int;
    /// `git_tree__free`. PENDING(l-commit)
    pub fn git_tree__free(tree: *mut c_void);
    /// `git_commit_tree`. PENDING(l-commit)
    pub fn git_commit_tree(out: *mut *mut c_void, commit: *const c_void) -> c_int;
    /// `git_tree_entry_bypath`. PENDING(l-commit)
    pub fn git_tree_entry_bypath(
        out: *mut *mut git_tree_entry,
        root: *const c_void,
        path: *const c_char,
    ) -> c_int;
    /// `git_tree_entry_type`. PENDING(l-commit)
    pub fn git_tree_entry_type(entry: *const git_tree_entry) -> c_int;
    /// `git_tree_entry_to_object`. PENDING(l-commit)
    pub fn git_tree_entry_to_object(
        out: *mut *mut c_void,
        repo: *mut git_repository,
        entry: *const git_tree_entry,
    ) -> c_int;
    /// `git_tree_entry_free`. PENDING(l-commit)
    pub fn git_tree_entry_free(entry: *mut git_tree_entry);
}

// PENDING(l-refs): refs.c.
unsafe extern "C" {
    /// `git_reference_lookup`. PENDING(l-refs)
    pub fn git_reference_lookup(
        out: *mut *mut git_reference,
        repo: *mut git_repository,
        name: *const c_char,
    ) -> c_int;
    /// `git_reference_name_to_id`. PENDING(l-refs)
    pub fn git_reference_name_to_id(
        out: *mut Oid,
        repo: *mut git_repository,
        name: *const c_char,
    ) -> c_int;
    /// `git_reference_create`. PENDING(l-refs)
    pub fn git_reference_create(
        out: *mut *mut git_reference,
        repo: *mut git_repository,
        name: *const c_char,
        id: *const Oid,
        force: c_int,
        log_message: *const c_char,
    ) -> c_int;
    /// `git_reference_free`. PENDING(l-refs)
    pub fn git_reference_free(r: *mut git_reference);
    /// `git_reference_delete`. PENDING(l-refs)
    pub fn git_reference_delete(r: *mut git_reference) -> c_int;
    /// `git_reference_foreach_name`. PENDING(l-refs)
    pub fn git_reference_foreach_name(
        repo: *mut git_repository,
        callback: git_reference_foreach_name_cb,
        payload: *mut c_void,
    ) -> c_int;
    /// `git_reference_name_is_valid`. PENDING(l-refs)
    pub fn git_reference_name_is_valid(valid: *mut c_int, refname: *const c_char) -> c_int;
}
