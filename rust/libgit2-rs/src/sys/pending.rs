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

// `git_odb_backend` holds a `git_odb *`, opaque to C (the Rust's).
#[allow(improper_ctypes)]
unsafe extern "C" {
    /// `git_object__free` (object.c). PENDING(l-obj)
    fn git_object__free(object: *mut c_void);
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
    use core::ffi::{c_char, c_int, c_void};

    use crate::oid::Oid;

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

        /// `git_tree_owner` (`object_api.c`). PENDING(l-obj)
        pub fn git_tree_owner(tree: *const c_void) -> *mut c_void;
        /// `git_tree_id` (`object_api.c`). PENDING(l-obj)
        pub fn git_tree_id(tree: *const c_void) -> *const Oid;
        /// `git_tree_lookup` (`object_api.c`). PENDING(l-obj)
        pub fn git_tree_lookup(out: *mut *mut c_void, repo: *mut c_void, id: *const Oid) -> c_int;
        /// `git_tree_free` (`object_api.c`). PENDING(l-obj)
        pub fn git_tree_free(tree: *mut c_void);
    }
}

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
        git_reference,
        git_grafts,
        git_revwalk,
        git_object,
        git_commit_graph_file,
    );

    // The ported modules' types, under the names this module used for them.
    /// `git_repository`.
    #[allow(non_camel_case_types)]
    pub type git_repository = crate::repository::GitRepository;
    /// `git_odb`.
    #[allow(non_camel_case_types)]
    pub type git_odb = crate::odb::GitOdb;
    /// `git_odb_object`.
    #[allow(non_camel_case_types)]
    pub type git_odb_object = crate::odb::GitOdbObject;
    /// `git_index`.
    #[allow(non_camel_case_types)]
    pub type git_index = crate::index::GitIndex;
    /// `git_tree_cache`.
    #[allow(non_camel_case_types)]
    pub type git_tree_cache = crate::tree_cache::GitTreeCache;
    pub use crate::cache::GitCachedObj;

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

    /// `git_commitarray` (`include/git2/commit.h`, public; its dispose is
    /// `commit.c`'s).
    #[repr(C)]
    pub struct GitCommitArray {
        pub commits: *const *mut GitCommit,
        pub count: usize,
    }

    // Offsets of the fields read here (`commit::tests::c_foreign_layout`):
    // `git_revwalk`'s, until l-revwalk is merged.
    /// `offsetof(git_revwalk, repo)`. PENDING(l-revwalk)
    pub const REVWALK_REPO: usize = 0;
    /// `offsetof(git_revwalk, odb)`. PENDING(l-revwalk)
    pub const REVWALK_ODB: usize = 8;
    /// `offsetof(git_revwalk, commit_pool)`. PENDING(l-revwalk)
    pub const REVWALK_COMMIT_POOL: usize = 56;

    /// Read the field at `off` of `base`.
    ///
    /// # Safety
    ///
    /// `base` points to a live object of the C type whose field of type `T`
    /// is at offset `off`, and nothing writes that field concurrently.
    #[inline]
    pub unsafe fn field<T: Copy, B>(base: *const B, off: usize) -> T {
        // SAFETY: `base + off` is inside the object (the caller's contract).
        let p = unsafe { base.cast::<u8>().add(off) }.cast::<T>();
        // SAFETY: the caller's contract: an initialized, aligned `T` there,
        // not written concurrently.
        unsafe { p.read() }
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

    // The repository, odb objects and the other ported types are Rust
    // types C sees as opaque. The l-obj and l-refs symbols are also
    // declared by the l-repo and l-index sections, with their own opaque
    // pointer types (same ABI); they go when those WPs land.
    #[allow(improper_ctypes, clashing_extern_declarations)]
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

        // PENDING(l-revwalk): revwalk.c
        pub fn git_revwalk__commit_lookup(
            walk: *mut git_revwalk,
            oid: *const Oid,
        ) -> *mut GitCommitListNode;
    }
}
