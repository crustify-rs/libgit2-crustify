//! Functions and globals of the modules that other L-wave work packages
//! port concurrently, as `repository.c`, `libgit2.c` and `settings.c` call
//! them (WP l-repo). Each entry carries its `// PENDING(<wp>)`; the
//! integration step replaces them with calls of the Rust ports.
//!
//! The types are those of the C headers: opaque objects are `c_void`, the
//! public structs the repository builds (`git_odb_options`,
//! `git_index_options`, `git_object_id_options`) and reads
//! (`git_config_entry`) are mirrored in `crate::repository::handles`.
//! The safe wrappers live there too, on the owner and handle types.

use core::ffi::{c_char, c_int, c_uint, c_void};
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicUsize};

use crate::oid::{Oid, git_oid_t};
use crate::util::str::GitStr;

/// `git_config_foreach_cb`.
pub type ConfigForeachCb =
    unsafe extern "C" fn(entry: *const ConfigEntryRaw, payload: *mut c_void) -> c_int;

/// `git_config_entry` (`include/git2/config.h`), mirrored (public).
#[repr(C)]
#[derive(Debug)]
pub struct ConfigEntryRaw {
    /// Name of the configuration entry (normalized).
    pub name: *const c_char,
    /// Literal (string) value of the entry.
    pub value: *const c_char,
    /// The type of backend that this entry exists in (eg, "file").
    pub backend_type: *const c_char,
    /// The path to the origin of this entry.
    pub origin_path: *const c_char,
    /// Depth of includes where this variable was found.
    pub include_depth: c_uint,
    /// Configuration level for the file this was found in.
    pub level: c_int,
}

/// `git_odb_options` / `git_index_options` (same layout).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct OidTypeOptions {
    /// The version (1).
    pub version: c_uint,
    /// The object id type, or 0 for the default.
    pub oid_type: git_oid_t,
}

/// `git_object_id_options` (`include/git2/object.h`).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ObjectIdOptions {
    /// The version (1).
    pub version: c_uint,
    /// `git_object_t`.
    pub object_type: c_int,
    /// `git_oid_t`.
    pub oid_type: git_oid_t,
    /// `git_filter_list *`.
    pub filters: *mut c_void,
}

// PENDING(l-config): config.c, config_cache.c.
unsafe extern "C" {
    pub fn git_config_new(out: *mut *mut c_void) -> c_int;
    pub fn git_config_free(cfg: *mut c_void);
    pub fn git_config_open_ondisk(out: *mut *mut c_void, path: *const c_char) -> c_int;
    pub fn git_config_open_default(out: *mut *mut c_void) -> c_int;
    pub fn git_config_open_level(
        out: *mut *mut c_void,
        parent: *const c_void,
        level: c_int,
    ) -> c_int;
    pub fn git_config_add_file_ondisk(
        cfg: *mut c_void,
        path: *const c_char,
        level: c_int,
        repo: *const c_void,
        force: c_int,
    ) -> c_int;
    pub fn git_config_set_writeorder(cfg: *mut c_void, levels: *mut c_int, len: usize) -> c_int;
    pub fn git_config_snapshot(out: *mut *mut c_void, cfg: *mut c_void) -> c_int;
    pub fn git_config_get_bool(out: *mut c_int, cfg: *const c_void, name: *const c_char) -> c_int;
    pub fn git_config_get_int32(out: *mut i32, cfg: *const c_void, name: *const c_char) -> c_int;
    pub fn git_config_get_entry(
        out: *mut *mut ConfigEntryRaw,
        cfg: *const c_void,
        name: *const c_char,
    ) -> c_int;
    pub fn git_config__lookup_entry(
        out: *mut *mut ConfigEntryRaw,
        cfg: *const c_void,
        key: *const c_char,
        no_errors: bool,
    ) -> c_int;
    pub fn git_config_entry_free(entry: *mut ConfigEntryRaw);
    pub fn git_config_get_multivar_foreach(
        cfg: *const c_void,
        name: *const c_char,
        regexp: *const c_char,
        cb: ConfigForeachCb,
        payload: *mut c_void,
    ) -> c_int;
    pub fn git_config_foreach_match(
        cfg: *const c_void,
        regexp: *const c_char,
        cb: ConfigForeachCb,
        payload: *mut c_void,
    ) -> c_int;
    pub fn git_config_set_int32(cfg: *mut c_void, name: *const c_char, value: i32) -> c_int;
    pub fn git_config_set_bool(cfg: *mut c_void, name: *const c_char, value: c_int) -> c_int;
    pub fn git_config_set_string(
        cfg: *mut c_void,
        name: *const c_char,
        value: *const c_char,
    ) -> c_int;
    pub fn git_config_delete_entry(cfg: *mut c_void, name: *const c_char) -> c_int;
    pub fn git_config__update_entry(
        cfg: *mut c_void,
        key: *const c_char,
        value: *const c_char,
        overwrite_existing: bool,
        only_if_existing: bool,
    ) -> c_int;
    pub fn git_config__get_path(out: *mut GitStr, cfg: *const c_void, name: *const c_char)
    -> c_int;
    pub fn git_config__get_string_buf(
        out: *mut GitStr,
        cfg: *const c_void,
        name: *const c_char,
    ) -> c_int;
    pub fn git_config_parse_bool(out: *mut c_int, value: *const c_char) -> c_int;
    pub fn git_config__find_system(path: *mut GitStr) -> c_int;
    pub fn git_config__find_global(path: *mut GitStr) -> c_int;
    pub fn git_config__find_xdg(path: *mut GitStr) -> c_int;
    pub fn git_config__global_location(buf: *mut GitStr) -> c_int;
    pub fn git_repository__configmap_lookup(
        out: *mut c_int,
        repo: *mut c_void,
        item: c_int,
    ) -> c_int;
    pub fn git_repository__configmap_lookup_cache_clear(repo: *mut c_void);
}

// PENDING(l-odb): odb.c, cache.c.
unsafe extern "C" {
    pub fn git_odb_new_ext(out: *mut *mut c_void, opts: *const OidTypeOptions) -> c_int;
    pub fn git_odb_free(db: *mut c_void);
    pub fn git_odb_add_disk_alternate(odb: *mut c_void, path: *const c_char) -> c_int;
    pub fn git_odb__set_caps(odb: *mut c_void, caps: c_int) -> c_int;
    pub fn git_odb__add_default_backends(
        db: *mut c_void,
        objects_dir: *const c_char,
        as_alternates: bool,
        alternate_depth: c_int,
    ) -> c_int;
    pub fn git_odb_exists(db: *mut c_void, id: *const Oid) -> c_int;
    pub fn git_cache_init(cache: *mut c_void) -> c_int;
    pub fn git_cache_clear(cache: *mut c_void);
    pub fn git_cache_dispose(cache: *mut c_void);
    pub fn git_cache_set_max_object_size(object_type: c_int, size: usize) -> c_int;

    /// `git_cache__enabled` (`bool`).
    pub static git_cache__enabled: AtomicBool;
    /// `git_cache__max_storage` (`ssize_t`).
    pub static git_cache__max_storage: AtomicIsize;
    /// `git_cache__current_storage` (`git_atomic_ssize`).
    pub static git_cache__current_storage: AtomicIsize;
    /// `git_odb__strict_hash_verification` (`bool`).
    pub static git_odb__strict_hash_verification: AtomicBool;
    /// `git_odb__packed_priority`.
    pub static git_odb__packed_priority: AtomicI32;
    /// `git_odb__loose_priority`.
    pub static git_odb__loose_priority: AtomicI32;
}

// PENDING(l-pack): mwindow.c, pack.c.
unsafe extern "C" {
    pub fn git_mwindow_global_init() -> c_int;
    /// `git_mwindow__window_size`.
    pub static git_mwindow__window_size: AtomicUsize;
    /// `git_mwindow__mapped_limit`.
    pub static git_mwindow__mapped_limit: AtomicUsize;
    /// `git_mwindow__file_limit`.
    pub static git_mwindow__file_limit: AtomicUsize;
    /// `git_disable_pack_keep_file_checks` (`bool`, pack.c).
    pub static git_disable_pack_keep_file_checks: AtomicBool;
}

// PENDING(l-refs): refdb.c, refs.c.
unsafe extern "C" {
    pub fn git_refdb_open(out: *mut *mut c_void, repo: *mut c_void) -> c_int;
    pub fn git_refdb_free(refdb: *mut c_void);
    pub fn git_refdb_init(
        refdb: *mut c_void,
        head_target: *const c_char,
        mode: libc::mode_t,
        flags: u32,
    ) -> c_int;
    pub fn git_reference_lookup(
        out: *mut *mut c_void,
        repo: *mut c_void,
        name: *const c_char,
    ) -> c_int;
    pub fn git_reference_lookup_resolved(
        out: *mut *mut c_void,
        repo: *mut c_void,
        name: *const c_char,
        max_deref: c_int,
    ) -> c_int;
    pub fn git_reference_free(r: *mut c_void);
    pub fn git_reference_type(r: *const c_void) -> c_int;
    pub fn git_reference_target(r: *const c_void) -> *const Oid;
    pub fn git_reference_symbolic_target(r: *const c_void) -> *const c_char;
    pub fn git_reference_name(r: *const c_void) -> *const c_char;
    pub fn git_reference_peel(out: *mut *mut c_void, r: *const c_void, t: c_int) -> c_int;
    pub fn git_reference_create(
        out: *mut *mut c_void,
        repo: *mut c_void,
        name: *const c_char,
        id: *const Oid,
        force: c_int,
        log_message: *const c_char,
    ) -> c_int;
    pub fn git_reference_symbolic_create(
        out: *mut *mut c_void,
        repo: *mut c_void,
        name: *const c_char,
        target: *const c_char,
        force: c_int,
        log_message: *const c_char,
    ) -> c_int;
    pub fn git_reference_is_branch(r: *const c_void) -> c_int;
    pub fn git_reference_is_tag(r: *const c_void) -> c_int;
    pub fn git_reference_is_remote(r: *const c_void) -> c_int;
    pub fn git_reference__is_branch(name: *const c_char) -> c_int;
    pub fn git_reference__is_tag(name: *const c_char) -> c_int;
    pub fn git_reference__is_remote(name: *const c_char) -> c_int;
    pub fn git_reference__shorthand(name: *const c_char) -> *const c_char;
    pub fn git_reference_name_is_valid(valid: *mut c_int, name: *const c_char) -> c_int;
    pub fn git_reference_iterator_new(out: *mut *mut c_void, repo: *mut c_void) -> c_int;
    pub fn git_reference_next_name(out: *mut *const c_char, iter: *mut c_void) -> c_int;
    pub fn git_reference_iterator_free(iter: *mut c_void);
    pub fn git_reference_remove(repo: *mut c_void, name: *const c_char) -> c_int;

    /// `git_reference__enable_symbolic_ref_target_validation` (`bool`).
    pub static git_reference__enable_symbolic_ref_target_validation: AtomicBool;
}

// PENDING(l-index): index.c.
unsafe extern "C" {
    pub fn git_index_open_ext(
        out: *mut *mut c_void,
        path: *const c_char,
        opts: *const OidTypeOptions,
    ) -> c_int;
    pub fn git_index_free(index: *mut c_void);
    pub fn git_index_set_caps(index: *mut c_void, caps: c_int) -> c_int;

    /// `git_index__enforce_unsaved_safety` (`bool`).
    pub static git_index__enforce_unsaved_safety: AtomicBool;
}

// PENDING(l-obj): grafts.c, object.c.
unsafe extern "C" {
    pub fn git_grafts_new(out: *mut *mut c_void, oid_type: git_oid_t) -> c_int;
    pub fn git_grafts_free(grafts: *mut c_void);
    pub fn git_grafts_refresh(grafts: *mut c_void) -> c_int;
    pub fn git_grafts_open_or_refresh(
        out: *mut *mut c_void,
        path: *const c_char,
        oid_type: git_oid_t,
    ) -> c_int;
    pub fn git_grafts_oids(out: *mut *mut Oid, out_len: *mut usize, grafts: *mut c_void) -> c_int;
    pub fn git_object_lookup(
        out: *mut *mut c_void,
        repo: *mut c_void,
        id: *const Oid,
        t: c_int,
    ) -> c_int;
    pub fn git_object_peel(out: *mut *mut c_void, obj: *const c_void, t: c_int) -> c_int;
    pub fn git_object_id(obj: *const c_void) -> *const Oid;
    pub fn git_object_free(obj: *mut c_void);
    pub fn git_object_id_from_fd(
        id: *mut Oid,
        fd: c_int,
        size: usize,
        opts: *mut ObjectIdOptions,
    ) -> c_int;

    /// `git_object__strict_input_validation` (`bool`).
    pub static git_object__strict_input_validation: AtomicBool;
}

// PENDING(l-commit): commit.c.
unsafe extern "C" {
    pub fn git_commit_lookup(out: *mut *mut c_void, repo: *mut c_void, id: *const Oid) -> c_int;
}
