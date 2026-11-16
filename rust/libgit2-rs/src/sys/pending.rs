//! Functions of modules that another work package is porting concurrently,
//! called through their C symbols until both have landed (PORTING.md §3).
//! Each entry carries a `// PENDING(<wp>)` comment.

// PENDING(l-repo), PENDING(l-refs), PENDING(l-obj), PENDING(l-commit): what
// the index (WP l-index) calls of `repository.c`, `refs.c`, `blob.c`,
// `object.c` and `tree.c`. Safe wrappers: `crate::index::foreign`.
pub mod l_index {
    use core::ffi::{c_char, c_int, c_uint, c_void};

    use crate::oid::Oid;
    use crate::util::str::GitStr;

    /// `git_treewalk_cb`.
    pub type TreewalkCb = unsafe extern "C" fn(
        root: *const c_char,
        entry: *const c_void,
        payload: *mut c_void,
    ) -> c_int;

    unsafe extern "C" {
        /// `git_repository__configmap_lookup`. PENDING(l-repo)
        pub fn git_repository__configmap_lookup(
            out: *mut c_int,
            repo: *mut c_void,
            item: c_int,
        ) -> c_int;
        /// `git_repository_workdir`. PENDING(l-repo)
        pub fn git_repository_workdir(repo: *const c_void) -> *const c_char;
        /// `git_repository_is_bare`. PENDING(l-repo)
        pub fn git_repository_is_bare(repo: *const c_void) -> c_int;
        /// `git_repository_workdir_path`. PENDING(l-repo)
        pub fn git_repository_workdir_path(
            out: *mut GitStr,
            repo: *mut c_void,
            path: *const c_char,
        ) -> c_int;
        /// `git_repository_open`. PENDING(l-repo)
        pub fn git_repository_open(out: *mut *mut c_void, path: *const c_char) -> c_int;
        /// `git_repository_head`. PENDING(l-repo)
        pub fn git_repository_head(out: *mut *mut c_void, repo: *mut c_void) -> c_int;
        /// `git_repository_free`. PENDING(l-repo)
        pub fn git_repository_free(repo: *mut c_void);
        /// `git_repository_index__weakptr`. PENDING(l-repo)
        pub fn git_repository_index__weakptr(out: *mut *mut c_void, repo: *mut c_void) -> c_int;

        /// `git_reference_target`. PENDING(l-refs)
        pub fn git_reference_target(r: *const c_void) -> *const Oid;
        /// `git_reference_free`. PENDING(l-refs)
        pub fn git_reference_free(r: *mut c_void);

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
}
