//! Functions of modules that another work package is porting concurrently,
//! called through their C symbols until both have landed (PORTING.md §3).
//! Each entry carries a `// PENDING(<wp>)` comment.

/// What the revwalk (WP l-revwalk) calls of modules that other L work
/// packages port concurrently, through their C symbols, with the opaque
/// C types they take. The integration step replaces each with the Rust
/// call of its module.
pub mod l_revwalk {
    use core::ffi::{c_char, c_int, c_uint};
    use core::marker::{PhantomData, PhantomPinned};

    use crate::oid::Oid;

    macro_rules! opaque {
        ($($(#[$m:meta])* $name:ident;)*) => {$(
            $(#[$m])*
            #[repr(C)]
            pub struct $name {
                _opaque: [u8; 0],
                _marker: PhantomData<(*mut u8, PhantomPinned)>,
            }
        )*};
    }

    opaque! {
        /// `git_repository`. PENDING(l-repo)
        git_repository;
        /// `git_odb`. PENDING(l-odb)
        git_odb;
        /// `git_object`. PENDING(l-obj)
        git_object;
        /// `git_reference`. PENDING(l-refs)
        git_reference;
        /// `git_reference_iterator`. PENDING(l-refs)
        git_reference_iterator;
        /// `git_commit`. PENDING(l-commit)
        git_commit;
        /// `git_tree`. PENDING(l-commit)
        git_tree;
        /// `git_tree_entry`. PENDING(l-commit)
        git_tree_entry;
    }

    /// `GIT_OBJECT_ANY`.
    pub const GIT_OBJECT_ANY: c_int = -2;
    /// `GIT_OBJECT_COMMIT`.
    pub const GIT_OBJECT_COMMIT: c_int = 1;

    unsafe extern "C" {
        // PENDING(l-repo)
        pub fn git_repository_odb(out: *mut *mut git_odb, repo: *mut git_repository) -> c_int;
        pub fn git_repository_open(out: *mut *mut git_repository, path: *const c_char) -> c_int;
        pub fn git_repository_free(repo: *mut git_repository);

        // PENDING(l-odb)
        pub fn git_odb_free(db: *mut git_odb);

        // PENDING(l-obj)
        pub fn git_object_lookup(
            out: *mut *mut git_object,
            repo: *mut git_repository,
            id: *const Oid,
            kind: c_int,
        ) -> c_int;
        pub fn git_object_peel(
            out: *mut *mut git_object,
            object: *const git_object,
            kind: c_int,
        ) -> c_int;
        pub fn git_object_free(object: *mut git_object);
        pub fn git_object_id(object: *const git_object) -> *const Oid;

        // PENDING(l-refs)
        pub fn git_reference_name_to_id(
            out: *mut Oid,
            repo: *mut git_repository,
            name: *const c_char,
        ) -> c_int;
        pub fn git_reference_iterator_glob_new(
            out: *mut *mut git_reference_iterator,
            repo: *mut git_repository,
            glob: *const c_char,
        ) -> c_int;
        pub fn git_reference_next(
            out: *mut *mut git_reference,
            iter: *mut git_reference_iterator,
        ) -> c_int;
        pub fn git_reference_name(r: *const git_reference) -> *const c_char;
        pub fn git_reference_free(r: *mut git_reference);
        pub fn git_reference_iterator_free(iter: *mut git_reference_iterator);

        // PENDING(l-commit): commit_list.c. `walk` is a `git_revwalk *`,
        // `commit` a `git_commit_list_node *` of its pool.
        pub fn git_commit_list_parse(
            walk: *mut core::ffi::c_void,
            commit: *mut core::ffi::c_void,
        ) -> c_int;

        // PENDING(l-commit): commit.c and tree.c
        pub fn git_commit_lookup(
            out: *mut *mut git_commit,
            repo: *mut git_repository,
            id: *const Oid,
        ) -> c_int;
        pub fn git_commit_tree(out: *mut *mut git_tree, commit: *const git_commit) -> c_int;
        pub fn git_commit_parentcount(commit: *const git_commit) -> c_uint;
        pub fn git_commit_parent(
            out: *mut *mut git_commit,
            commit: *const git_commit,
            n: c_uint,
        ) -> c_int;
        pub fn git_commit_free(commit: *mut git_commit);
        pub fn git_tree_free(tree: *mut git_tree);
        pub fn git_tree_entry_bypath(
            out: *mut *mut git_tree_entry,
            root: *const git_tree,
            path: *const c_char,
        ) -> c_int;
        pub fn git_tree_entry_free(entry: *mut git_tree_entry);
        pub fn git_tree_entry_id(entry: *const git_tree_entry) -> *const Oid;
    }
}
