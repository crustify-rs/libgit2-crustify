//! Port of `src/libgit2/object_api.c`: the typed wrappers of the object
//! API (`git_commit_lookup`, `git_tree_free`, `git_blob_id`, …), each a
//! cast over [`crate::object`]'s functions.

/// The exports of `object_api.c`.
pub mod ffi {
    use crate::util::cabi::Out;
    use core::ffi::c_int;

    use crate::object::deps::git_repository;
    use crate::object::ffi as o;
    use crate::object::{
        GIT_OBJECT_BLOB, GIT_OBJECT_COMMIT, GIT_OBJECT_TAG, GIT_OBJECT_TREE, GitObject,
    };
    use crate::oid::Oid;

    /// The six typed exports of one object type. A `git_commit *` (and the
    /// others) is a `git_object *` (its header is the first member), so
    /// the parameters are the object's: the same ABI (a pointer).
    macro_rules! typed_object_api {
        (
            $t:expr, $name:literal,
            $lookup:ident, $lookup_prefix:ident, $free:ident, $id:ident, $owner:ident, $dup:ident
        ) => {
            #[doc = concat!("`git_", $name, "_lookup`.")]
            ///
            /// # Safety
            ///
            /// As `git_object_lookup`: `out` is NULL or writable; `repo`
            /// NULL or a live repository; `id` NULL or a readable id that
            /// nothing writes during the call.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $lookup(
                out: Out<'_, *mut GitObject>,
                repo: Option<&git_repository>,
                id: Option<&Oid>,
            ) -> c_int {
                o::lookup_c(out, repo, id, $t)
            }

            #[doc = concat!("`git_", $name, "_lookup_prefix`.")]
            ///
            /// # Safety
            ///
            /// As `git_object_lookup_prefix` (see the lookup above).
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $lookup_prefix(
                out: Out<'_, *mut GitObject>,
                repo: Option<&git_repository>,
                id: Option<&Oid>,
                len: usize,
            ) -> c_int {
                o::lookup_prefix_c(out, repo, id, len, $t)
            }

            #[doc = concat!("`git_", $name, "_free`.")]
            ///
            /// # Safety
            ///
            /// As `git_object_free`: `obj` is NULL or an object the caller
            /// holds a reference to, not used again by the caller.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $free(obj: *mut GitObject) {
                // SAFETY: this export's contract is `git_object_free`'s.
                unsafe { o::git_object_free(obj) }
            }

            #[doc = concat!("`git_", $name, "_id`.")]
            ///
            /// # Safety
            ///
            /// As `git_object_id`: `obj` is NULL or a live object.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $id(obj: Option<&GitObject>) -> *const Oid {
                o::object_id(obj)
            }

            #[doc = concat!("`git_", $name, "_owner`.")]
            ///
            /// # Safety
            ///
            /// As `git_object_owner`: `obj` is NULL or a live object.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $owner(obj: Option<&GitObject>) -> *mut git_repository {
                o::object_owner(obj)
            }

            #[doc = concat!("`git_", $name, "_dup`.")]
            ///
            /// # Safety
            ///
            /// As `git_object_dup`: `out` is NULL or writable, `obj` NULL
            /// or a live object.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $dup(
                out: Out<'_, *mut GitObject>,
                obj: *mut GitObject,
            ) -> c_int {
                // SAFETY: this export's contract is `git_object_dup`'s.
                unsafe { o::git_object_dup(out, obj) }
            }
        };
    }

    typed_object_api!(
        GIT_OBJECT_COMMIT,
        "commit",
        git_commit_lookup,
        git_commit_lookup_prefix,
        git_commit_free,
        git_commit_id,
        git_commit_owner,
        git_commit_dup
    );
    typed_object_api!(
        GIT_OBJECT_TREE,
        "tree",
        git_tree_lookup,
        git_tree_lookup_prefix,
        git_tree_free,
        git_tree_id,
        git_tree_owner,
        git_tree_dup
    );
    typed_object_api!(
        GIT_OBJECT_TAG,
        "tag",
        git_tag_lookup,
        git_tag_lookup_prefix,
        git_tag_free,
        git_tag_id,
        git_tag_owner,
        git_tag_dup
    );
    typed_object_api!(
        GIT_OBJECT_BLOB,
        "blob",
        git_blob_lookup,
        git_blob_lookup_prefix,
        git_blob_free,
        git_blob_id,
        git_blob_owner,
        git_blob_dup
    );
}
