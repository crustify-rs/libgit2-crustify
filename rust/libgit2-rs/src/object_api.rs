//! Port of `src/libgit2/object_api.c`: the typed wrappers of the object
//! API (`git_commit_lookup`, `git_tree_free`, `git_blob_id`, …), each a
//! cast over [`crate::object`]'s functions.

/// The exports of `object_api.c`.
pub mod ffi {
    use core::ffi::{c_int, c_void};

    use crate::object::ffi as o;
    use crate::object::{GIT_OBJECT_BLOB, GIT_OBJECT_COMMIT, GIT_OBJECT_TAG, GIT_OBJECT_TREE, GitObject};
    use crate::oid::Oid;
    use crate::sys::pending::git_repository;

    /// `git_commit_lookup`.
    ///
    /// # Safety
    ///
    /// As `git_object_lookup`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_lookup(out: *mut *mut c_void, repo: *mut git_repository, id: *const Oid) -> c_int {
        // SAFETY: forwarded.
        unsafe { o::git_object_lookup(out.cast(), repo, id, GIT_OBJECT_COMMIT) }
    }

    /// `git_commit_lookup_prefix`.
    ///
    /// # Safety
    ///
    /// As `git_object_lookup_prefix`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_lookup_prefix(
        out: *mut *mut c_void,
        repo: *mut git_repository,
        id: *const Oid,
        len: usize,
    ) -> c_int {
        // SAFETY: forwarded.
        unsafe { o::git_object_lookup_prefix(out.cast(), repo, id, len, GIT_OBJECT_COMMIT) }
    }

    /// `git_commit_free`.
    ///
    /// # Safety
    ///
    /// As `git_object_free`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_free(obj: *mut c_void) {
        // SAFETY: forwarded.
        unsafe { o::git_object_free(obj.cast::<GitObject>()) }
    }

    /// `git_commit_id`.
    ///
    /// # Safety
    ///
    /// As `git_object_id`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_id(obj: *const c_void) -> *const Oid {
        // SAFETY: forwarded.
        unsafe { o::git_object_id(obj.cast::<GitObject>()) }
    }

    /// `git_commit_owner`.
    ///
    /// # Safety
    ///
    /// As `git_object_owner`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_owner(obj: *const c_void) -> *mut git_repository {
        // SAFETY: forwarded.
        unsafe { o::git_object_owner(obj.cast::<GitObject>()) }
    }

    /// `git_commit_dup`.
    ///
    /// # Safety
    ///
    /// As `git_object_dup`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_dup(out: *mut *mut c_void, obj: *mut c_void) -> c_int {
        // SAFETY: forwarded.
        unsafe { o::git_object_dup(out.cast(), obj.cast::<GitObject>()) }
    }

    /// `git_tree_lookup`.
    ///
    /// # Safety
    ///
    /// As `git_object_lookup`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_lookup(out: *mut *mut c_void, repo: *mut git_repository, id: *const Oid) -> c_int {
        // SAFETY: forwarded.
        unsafe { o::git_object_lookup(out.cast(), repo, id, GIT_OBJECT_TREE) }
    }

    /// `git_tree_lookup_prefix`.
    ///
    /// # Safety
    ///
    /// As `git_object_lookup_prefix`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_lookup_prefix(
        out: *mut *mut c_void,
        repo: *mut git_repository,
        id: *const Oid,
        len: usize,
    ) -> c_int {
        // SAFETY: forwarded.
        unsafe { o::git_object_lookup_prefix(out.cast(), repo, id, len, GIT_OBJECT_TREE) }
    }

    /// `git_tree_free`.
    ///
    /// # Safety
    ///
    /// As `git_object_free`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_free(obj: *mut c_void) {
        // SAFETY: forwarded.
        unsafe { o::git_object_free(obj.cast::<GitObject>()) }
    }

    /// `git_tree_id`.
    ///
    /// # Safety
    ///
    /// As `git_object_id`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_id(obj: *const c_void) -> *const Oid {
        // SAFETY: forwarded.
        unsafe { o::git_object_id(obj.cast::<GitObject>()) }
    }

    /// `git_tree_owner`.
    ///
    /// # Safety
    ///
    /// As `git_object_owner`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_owner(obj: *const c_void) -> *mut git_repository {
        // SAFETY: forwarded.
        unsafe { o::git_object_owner(obj.cast::<GitObject>()) }
    }

    /// `git_tree_dup`.
    ///
    /// # Safety
    ///
    /// As `git_object_dup`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_dup(out: *mut *mut c_void, obj: *mut c_void) -> c_int {
        // SAFETY: forwarded.
        unsafe { o::git_object_dup(out.cast(), obj.cast::<GitObject>()) }
    }

    /// `git_tag_lookup`.
    ///
    /// # Safety
    ///
    /// As `git_object_lookup`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_lookup(out: *mut *mut c_void, repo: *mut git_repository, id: *const Oid) -> c_int {
        // SAFETY: forwarded.
        unsafe { o::git_object_lookup(out.cast(), repo, id, GIT_OBJECT_TAG) }
    }

    /// `git_tag_lookup_prefix`.
    ///
    /// # Safety
    ///
    /// As `git_object_lookup_prefix`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_lookup_prefix(
        out: *mut *mut c_void,
        repo: *mut git_repository,
        id: *const Oid,
        len: usize,
    ) -> c_int {
        // SAFETY: forwarded.
        unsafe { o::git_object_lookup_prefix(out.cast(), repo, id, len, GIT_OBJECT_TAG) }
    }

    /// `git_tag_free`.
    ///
    /// # Safety
    ///
    /// As `git_object_free`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_free(obj: *mut c_void) {
        // SAFETY: forwarded.
        unsafe { o::git_object_free(obj.cast::<GitObject>()) }
    }

    /// `git_tag_id`.
    ///
    /// # Safety
    ///
    /// As `git_object_id`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_id(obj: *const c_void) -> *const Oid {
        // SAFETY: forwarded.
        unsafe { o::git_object_id(obj.cast::<GitObject>()) }
    }

    /// `git_tag_owner`.
    ///
    /// # Safety
    ///
    /// As `git_object_owner`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_owner(obj: *const c_void) -> *mut git_repository {
        // SAFETY: forwarded.
        unsafe { o::git_object_owner(obj.cast::<GitObject>()) }
    }

    /// `git_tag_dup`.
    ///
    /// # Safety
    ///
    /// As `git_object_dup`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_dup(out: *mut *mut c_void, obj: *mut c_void) -> c_int {
        // SAFETY: forwarded.
        unsafe { o::git_object_dup(out.cast(), obj.cast::<GitObject>()) }
    }

    /// `git_blob_lookup`.
    ///
    /// # Safety
    ///
    /// As `git_object_lookup`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_lookup(out: *mut *mut c_void, repo: *mut git_repository, id: *const Oid) -> c_int {
        // SAFETY: forwarded.
        unsafe { o::git_object_lookup(out.cast(), repo, id, GIT_OBJECT_BLOB) }
    }

    /// `git_blob_lookup_prefix`.
    ///
    /// # Safety
    ///
    /// As `git_object_lookup_prefix`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_lookup_prefix(
        out: *mut *mut c_void,
        repo: *mut git_repository,
        id: *const Oid,
        len: usize,
    ) -> c_int {
        // SAFETY: forwarded.
        unsafe { o::git_object_lookup_prefix(out.cast(), repo, id, len, GIT_OBJECT_BLOB) }
    }

    /// `git_blob_free`.
    ///
    /// # Safety
    ///
    /// As `git_object_free`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_free(obj: *mut c_void) {
        // SAFETY: forwarded.
        unsafe { o::git_object_free(obj.cast::<GitObject>()) }
    }

    /// `git_blob_id`.
    ///
    /// # Safety
    ///
    /// As `git_object_id`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_id(obj: *const c_void) -> *const Oid {
        // SAFETY: forwarded.
        unsafe { o::git_object_id(obj.cast::<GitObject>()) }
    }

    /// `git_blob_owner`.
    ///
    /// # Safety
    ///
    /// As `git_object_owner`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_owner(obj: *const c_void) -> *mut git_repository {
        // SAFETY: forwarded.
        unsafe { o::git_object_owner(obj.cast::<GitObject>()) }
    }

    /// `git_blob_dup`.
    ///
    /// # Safety
    ///
    /// As `git_object_dup`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_dup(out: *mut *mut c_void, obj: *mut c_void) -> c_int {
        // SAFETY: forwarded.
        unsafe { o::git_object_dup(out.cast(), obj.cast::<GitObject>()) }
    }
}
