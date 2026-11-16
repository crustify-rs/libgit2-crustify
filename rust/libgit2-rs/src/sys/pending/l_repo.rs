//! Functions and globals of the modules that other L-wave work packages
//! port concurrently, as `repository.c`, `libgit2.c` and `settings.c` call
//! them (WP l-repo). Each entry carries its `// PENDING(<wp>)`; the
//! integration step replaces them with calls of the Rust ports.
//!
//! The types are those of the C headers: opaque objects are `c_void`, the
//! public struct the repository builds (`git_object_id_options`) is
//! mirrored here.
//! The safe wrappers live there too, on the owner and handle types.

use core::ffi::{c_char, c_int, c_uint, c_void};
use core::sync::atomic::AtomicBool;

use crate::oid::{Oid, git_oid_t};

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

// PENDING(l-obj): object_api.c (`git_commit_lookup`).
unsafe extern "C" {
    pub fn git_commit_lookup(out: *mut *mut c_void, repo: *mut c_void, id: *const Oid) -> c_int;
}
