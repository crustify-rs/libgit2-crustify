//! Functions of modules that another work package is porting concurrently,
//! called through their C symbols until both have landed (PORTING.md §3).
//! Each entry carries a `// PENDING(<wp>)` comment.

/// What `commit.c`, `commit_list.c` and `tree.c` (WP l-commit) call in
/// modules that other L work packages port concurrently, through their C
/// symbols, and the fields of those modules' structs that they read, at the
/// offsets the C headers give (checked by `commit::tests::c_foreign_layout`).
pub mod l_commit {
    use core::ffi::{c_char, c_int, c_void};
    use core::marker::{PhantomData, PhantomPinned};

    use crate::commit::{GitCommit, GitSignature};
    use crate::commit_list::GitCommitListNode;
    use crate::oid::{Oid, git_oid_t};
    use crate::tree::GitTree;
    use crate::util::array::GitArray;
    use crate::util::pool::GitPool;
    use crate::util::thread::Atomic32;

    macro_rules! opaque {
        ($($name:ident),* $(,)?) => {$(
            #[doc = concat!("`", stringify!($name), "`, opaque here.")]
            #[repr(C)]
            #[allow(non_camel_case_types)]
            pub struct $name {
                _private: [u8; 0],
                _marker: PhantomData<(*mut u8, PhantomPinned)>,
            }
        )*};
    }
    opaque!(
        git_repository,
        git_odb,
        git_odb_object,
        git_reference,
        git_index,
        git_tree_cache,
        git_grafts,
        git_revwalk,
        git_object,
        git_commit_graph_file,
    );

    /// `git_cached_obj` (`cache.h`). PENDING(l-odb)
    #[repr(C)]
    pub struct GitCachedObj {
        pub oid: Oid,
        /// `git_object_t`.
        pub type_: i16,
        pub flags: u16,
        pub size: usize,
        /// Written by the cache, from any thread.
        pub refcount: Atomic32,
    }

    /// `git_object` (`object.h`): the head of every object. PENDING(l-obj)
    #[repr(C)]
    pub struct GitObjectHead {
        pub cached: GitCachedObj,
        pub repo: *mut git_repository,
    }

    /// `git_commit_graft` (`grafts.h`). PENDING(l-obj)
    #[repr(C)]
    pub struct GitCommitGraft {
        pub oid: Oid,
        pub parents: GitArray<Oid>,
    }

    /// `git_index_entry` (`include/git2/index.h`, public). PENDING(l-index)
    #[repr(C)]
    pub struct GitIndexEntry {
        pub ctime: [u32; 2],
        pub mtime: [u32; 2],
        pub dev: u32,
        pub ino: u32,
        pub mode: u32,
        pub uid: u32,
        pub gid: u32,
        pub file_size: u32,
        pub id: Oid,
        pub flags: u16,
        pub flags_extended: u16,
        pub path: *const c_char,
    }

    /// `git_commitarray` (`include/git2/commit.h`, public; its dispose is
    /// `commit.c`'s).
    #[repr(C)]
    pub struct GitCommitArray {
        pub commits: *const *mut GitCommit,
        pub count: usize,
    }

    // Offsets of the fields read here (`commit::tests::c_foreign_layout`).
    /// `offsetof(git_repository, oid_type)`. PENDING(l-repo)
    pub const REPO_OID_TYPE: usize = 236;
    /// `offsetof(git_repository, grafts)`. PENDING(l-repo)
    pub const REPO_GRAFTS: usize = 248;
    /// `offsetof(git_repository, shallow_grafts)`. PENDING(l-repo)
    pub const REPO_SHALLOW_GRAFTS: usize = 256;
    /// `offsetof(git_revwalk, repo)`. PENDING(l-revwalk)
    pub const REVWALK_REPO: usize = 0;
    /// `offsetof(git_revwalk, odb)`. PENDING(l-revwalk)
    pub const REVWALK_ODB: usize = 8;
    /// `offsetof(git_revwalk, commit_pool)`. PENDING(l-revwalk)
    pub const REVWALK_COMMIT_POOL: usize = 56;
    /// `offsetof(git_index, oid_type)`. PENDING(l-index)
    pub const INDEX_OID_TYPE: usize = 220;
    /// The `unsigned int` of `git_index`'s bitfields (`on_disk` is bit 0,
    /// `ignore_case` bit 1). PENDING(l-index)
    pub const INDEX_FLAGS: usize = 224;
    /// `offsetof(git_index, tree)`. PENDING(l-index)
    pub const INDEX_TREE: usize = 232;
    /// `offsetof(git_index, tree_pool)`. PENDING(l-index)
    pub const INDEX_TREE_POOL: usize = 240;
    /// `offsetof(git_tree_cache, entry_count)`. PENDING(l-index)
    pub const TREE_CACHE_ENTRY_COUNT: usize = 24;
    /// `offsetof(git_tree_cache, oid)`. PENDING(l-index)
    pub const TREE_CACHE_OID: usize = 32;

    /// Read the field at `off` of `base`.
    ///
    /// # Safety
    ///
    /// `base` points to a live object of the C type whose field of type `T`
    /// is at offset `off`, and nothing writes that field concurrently.
    #[inline]
    pub unsafe fn field<T: Copy, B>(base: *const B, off: usize) -> T {
        // SAFETY: the caller's contract: an initialized, aligned `T` at
        // `base + off`, inside the same allocation.
        unsafe { base.cast::<u8>().add(off).cast::<T>().read() }
    }

    /// The address of the field at `off` of `base`.
    ///
    /// # Safety
    ///
    /// As [`field`] (without the read).
    #[inline]
    pub unsafe fn field_ptr<T, B>(base: *mut B, off: usize) -> *mut T {
        // SAFETY: the caller's contract: `base + off` is inside the object.
        unsafe { base.cast::<u8>().add(off).cast::<T>() }
    }

    pub type GitCommitParentCallback =
        Option<unsafe extern "C" fn(idx: usize, payload: *mut c_void) -> *const Oid>;

    unsafe extern "C" {
        // PENDING(l-obj): object.c, object_api.c, signature.c, grafts.c
        pub fn git_object_lookup(
            out: *mut *mut git_object,
            repo: *mut git_repository,
            id: *const Oid,
            kind: c_int,
        ) -> c_int;
        pub fn git_object_free(obj: *mut git_object);
        pub fn git_object_dup(out: *mut *mut git_object, source: *mut git_object) -> c_int;
        pub fn git_object__from_raw(
            out: *mut *mut git_object,
            data: *const c_char,
            size: usize,
            kind: c_int,
            oid_type: git_oid_t,
        ) -> c_int;
        pub fn git_object_type2string(kind: c_int) -> *const c_char;
        pub fn git_object__is_valid(repo: *mut git_repository, id: *const Oid, kind: c_int)
        -> bool;
        pub fn git_commit_lookup(
            out: *mut *mut GitCommit,
            repo: *mut git_repository,
            id: *const Oid,
        ) -> c_int;
        pub fn git_commit_free(commit: *mut GitCommit);
        pub fn git_commit_dup(out: *mut *mut GitCommit, source: *mut GitCommit) -> c_int;
        pub fn git_tree_lookup(
            out: *mut *mut GitTree,
            repo: *mut git_repository,
            id: *const Oid,
        ) -> c_int;
        pub fn git_tree_free(tree: *mut GitTree);
        pub fn git_tree_dup(out: *mut *mut GitTree, source: *mut GitTree) -> c_int;
        pub fn git_signature_default(
            out: *mut *mut GitSignature,
            repo: *mut git_repository,
        ) -> c_int;
        pub fn git_signature_free(sig: *mut GitSignature);
        pub fn git_grafts_get(
            out: *mut *mut GitCommitGraft,
            grafts: *mut git_grafts,
            oid: *const Oid,
        ) -> c_int;

        // PENDING(l-odb): odb.c
        pub fn git_odb_read(
            out: *mut *mut git_odb_object,
            db: *mut git_odb,
            id: *const Oid,
        ) -> c_int;
        pub fn git_odb_object_free(obj: *mut git_odb_object);
        pub fn git_odb_object_dup(
            out: *mut *mut git_odb_object,
            source: *mut git_odb_object,
        ) -> c_int;
        pub fn git_odb_object_data(obj: *mut git_odb_object) -> *const c_void;
        pub fn git_odb_object_size(obj: *mut git_odb_object) -> usize;
        pub fn git_odb_object_id(obj: *mut git_odb_object) -> *const Oid;
        pub fn git_odb_object_type(obj: *mut git_odb_object) -> c_int;
        pub fn git_odb_write(
            out: *mut Oid,
            db: *mut git_odb,
            data: *const c_void,
            len: usize,
            kind: c_int,
        ) -> c_int;
        pub fn git_odb__freshen(db: *mut git_odb, id: *const Oid) -> c_int;
        pub fn git_odb__get_commit_graph_file(
            out: *mut *mut git_commit_graph_file,
            db: *mut git_odb,
        ) -> c_int;

        // PENDING(l-repo): repository.c
        pub fn git_repository_open(out: *mut *mut git_repository, path: *const c_char) -> c_int;
        pub fn git_repository_free(repo: *mut git_repository);
        pub fn git_repository_odb__weakptr(
            out: *mut *mut git_odb,
            repo: *mut git_repository,
        ) -> c_int;
        pub fn git_repository_head_tree(
            tree: *mut *mut GitTree,
            repo: *mut git_repository,
        ) -> c_int;
        pub fn git_repository_head(
            out: *mut *mut git_reference,
            repo: *mut git_repository,
        ) -> c_int;
        pub fn git_repository_commit_parents(
            commits: *mut GitCommitArray,
            repo: *mut git_repository,
        ) -> c_int;
        pub fn git_repository_index(out: *mut *mut git_index, repo: *mut git_repository) -> c_int;

        // PENDING(l-refs): refs.c
        pub fn git_reference_lookup_resolved(
            out: *mut *mut git_reference,
            repo: *mut git_repository,
            name: *const c_char,
            max_deref: c_int,
        ) -> c_int;
        pub fn git_reference_target(r: *const git_reference) -> *const Oid;
        pub fn git_reference_free(r: *mut git_reference);
        pub fn git_reference__update_for_commit(
            repo: *mut git_repository,
            r: *mut git_reference,
            ref_name: *const c_char,
            id: *const Oid,
            operation: *const c_char,
        ) -> c_int;
        pub fn git_reference_peel(
            out: *mut *mut git_object,
            r: *const git_reference,
            kind: c_int,
        ) -> c_int;

        // PENDING(l-index): index.c, tree-cache.c
        pub fn git_index_entrycount(index: *const git_index) -> usize;
        pub fn git_index_get_byindex(index: *mut git_index, n: usize) -> *const GitIndexEntry;
        pub fn git_index_has_conflicts(index: *const git_index) -> c_int;
        pub fn git_index__set_ignore_case(index: *mut git_index, ignore_case: bool);
        pub fn git_index_write_tree(out: *mut Oid, index: *mut git_index) -> c_int;
        pub fn git_index_free(index: *mut git_index);
        pub fn git_tree_cache_get(
            tree: *const git_tree_cache,
            path: *const c_char,
        ) -> *const git_tree_cache;
        pub fn git_tree_cache_read_tree(
            out: *mut *mut git_tree_cache,
            tree: *const GitTree,
            oid_type: git_oid_t,
            pool: *mut GitPool,
        ) -> c_int;

        // PENDING(l-revwalk): revwalk.c
        pub fn git_revwalk__commit_lookup(
            walk: *mut git_revwalk,
            oid: *const Oid,
        ) -> *mut GitCommitListNode;
    }
}
