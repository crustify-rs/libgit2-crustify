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
    opaque!(git_reference, git_grafts, git_object, git_commit_graph_file,);

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
    /// `git_revwalk`.
    #[allow(non_camel_case_types)]
    pub type git_revwalk = crate::revwalk::GitRevwalk;
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

    }
}

/// What the revwalk (WP l-revwalk) calls of modules that other L work
/// packages port concurrently, through their C symbols, with the opaque
/// C types they take. The integration step replaces each with the Rust
/// call of its module.
pub mod l_revwalk {
    use core::ffi::{c_char, c_int};
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

    // The ported modules' types, under the names this module used.
    /// `git_repository`.
    #[allow(non_camel_case_types)]
    pub type git_repository = crate::repository::GitRepository;
    /// `git_odb`.
    #[allow(non_camel_case_types)]
    pub type git_odb = crate::odb::GitOdb;
    /// `git_commit`.
    #[allow(non_camel_case_types)]
    pub type git_commit = crate::commit::GitCommit;
    /// `git_tree`.
    #[allow(non_camel_case_types)]
    pub type git_tree = crate::tree::GitTree;
    /// `git_tree_entry`.
    #[allow(non_camel_case_types)]
    pub type git_tree_entry = crate::tree::GitTreeEntry;

    opaque! {
        /// `git_object`. PENDING(l-obj)
        git_object;
        /// `git_reference`. PENDING(l-refs)
        git_reference;
        /// `git_reference_iterator`. PENDING(l-refs)
        git_reference_iterator;
    }

    /// `GIT_OBJECT_ANY`.
    pub const GIT_OBJECT_ANY: c_int = -2;
    /// `GIT_OBJECT_COMMIT`.
    pub const GIT_OBJECT_COMMIT: c_int = 1;

    // The repository and commits are Rust types C sees as opaque; the
    // l-obj/l-refs symbols are also declared by other sections.
    #[allow(improper_ctypes, clashing_extern_declarations)]
    unsafe extern "C" {
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

        // PENDING(l-obj): object_api.c
        pub fn git_commit_lookup(
            out: *mut *mut git_commit,
            repo: *mut git_repository,
            id: *const Oid,
        ) -> c_int;
        pub fn git_commit_free(commit: *mut git_commit);
        pub fn git_tree_free(tree: *mut git_tree);
    }
}

// ---------------------------------------------------------------------------
// WP l-refs: what refs.c, refdb.c, refdb_fs.c and reflog.c call in modules
// that other L WPs are porting.
// ---------------------------------------------------------------------------

use core::ffi::{c_char, c_int, c_uint};

use crate::oid::Oid;
use crate::util::str::GitStr;

/// `git_repository` (repository.h), opaque here. PENDING(l-repo)
#[repr(C)]
pub struct git_repository {
    _opaque: [u8; 0],
}

/// `git_object` (object.h), opaque here. PENDING(l-obj)
#[repr(C)]
pub struct git_object {
    _opaque: [u8; 0],
}

/// `git_commit` (commit.h), opaque here. PENDING(l-commit)
#[repr(C)]
pub struct git_commit {
    _opaque: [u8; 0],
}

/// `git_time` (include/git2/types.h, public: mirrored). PENDING(l-obj)
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct git_time {
    /// Seconds since the epoch.
    pub time: i64,
    /// The timezone offset, in minutes.
    pub offset: c_int,
    /// `'-'` for a questionable `-0000` offset.
    pub sign: c_char,
}

/// `git_signature` (include/git2/types.h, public: mirrored). PENDING(l-obj)
#[repr(C)]
#[derive(Debug)]
pub struct git_signature {
    /// The name, NUL-terminated.
    pub name: *mut c_char,
    /// The email, NUL-terminated.
    pub email: *mut c_char,
    /// When.
    pub when: git_time,
}

/// `git_repository_foreach_worktree_cb`. PENDING(l-repo)
pub type ForeachWorktreeCb = unsafe extern "C" fn(*mut git_repository, *mut c_void) -> c_int;

/// `git_configmap_item` values (repository.h). PENDING(l-repo)
pub mod configmap {
    use core::ffi::c_int;
    /// `GIT_CONFIGMAP_IGNORECASE`.
    pub const IGNORECASE: c_int = 3;
    /// `GIT_CONFIGMAP_PRECOMPOSE`.
    pub const PRECOMPOSE: c_int = 8;
    /// `GIT_CONFIGMAP_LOGALLREFUPDATES`.
    pub const LOGALLREFUPDATES: c_int = 10;
    /// `GIT_CONFIGMAP_FSYNCOBJECTFILES`.
    pub const FSYNCOBJECTFILES: c_int = 13;
}

/// `offsetof(git_repository, refdb_type)` (asserted by `refdb`'s layout
/// test). PENDING(l-repo): read through the integrated `Repository`.
pub const REPO_REFDB_TYPE_OFFSET: usize = 240;

unsafe extern "C" {
    // repository.c, PENDING(l-repo)
    /// `git_repository_refdb__weakptr`. PENDING(l-repo)
    pub fn git_repository_refdb__weakptr(out: *mut *mut c_void, repo: *mut git_repository)
    -> c_int;
    /// `git_repository_refdb`. PENDING(l-repo)
    pub fn git_repository_refdb(out: *mut *mut c_void, repo: *mut git_repository) -> c_int;
    /// `git_repository__configmap_lookup`. PENDING(l-repo)
    pub fn git_repository__configmap_lookup(
        out: *mut c_int,
        repo: *mut git_repository,
        item: c_int,
    ) -> c_int;
    /// `git_repository_foreach_worktree`. PENDING(l-repo)
    pub fn git_repository_foreach_worktree(
        repo: *mut git_repository,
        cb: ForeachWorktreeCb,
        payload: *mut c_void,
    ) -> c_int;
    /// `git_repository_path` (`repo->gitdir`). PENDING(l-repo)
    pub fn git_repository_path(repo: *const git_repository) -> *const c_char;
    /// `git_repository_commondir` (`repo->commondir`). PENDING(l-repo)
    pub fn git_repository_commondir(repo: *const git_repository) -> *const c_char;
    /// `git_repository_get_namespace` (`repo->namespace`). PENDING(l-repo)
    pub fn git_repository_get_namespace(repo: *mut git_repository) -> *const c_char;
    /// `git_repository_ident` (`repo->ident_name`, `repo->ident_email`). PENDING(l-repo)
    pub fn git_repository_ident(
        name: *mut *const c_char,
        email: *mut *const c_char,
        repo: *const git_repository,
    ) -> c_int;
    /// `git_repository_is_bare`. PENDING(l-repo)
    pub fn git_repository_is_bare(repo: *const git_repository) -> c_int;
    /// `git_repository_is_worktree`. PENDING(l-repo)
    pub fn git_repository_is_worktree(repo: *const git_repository) -> c_int;
    /// `git_repository_oid_type`. PENDING(l-repo)
    pub fn git_repository_oid_type(repo: *const git_repository) -> c_int;
    /// `git_repository_open`. PENDING(l-repo)
    pub fn git_repository_open(out: *mut *mut git_repository, path: *const c_char) -> c_int;
    /// `git_repository_init`. PENDING(l-repo)
    pub fn git_repository_init(
        out: *mut *mut git_repository,
        path: *const c_char,
        is_bare: c_uint,
    ) -> c_int;
    /// `git_repository_free`. PENDING(l-repo)
    pub fn git_repository_free(repo: *mut git_repository);
    /// `git_repository__fsync_gitdir`, written by `git_libgit2_opts`. PENDING(l-repo)
    pub static git_repository__fsync_gitdir: core::sync::atomic::AtomicBool;

    // object.c, object_api.c, tag.c, PENDING(l-obj)
    /// `git_object__is_valid`. PENDING(l-obj)
    pub fn git_object__is_valid(repo: *mut git_repository, id: *const Oid, kind: c_int) -> bool;
    /// `git_object_lookup`. PENDING(l-obj)
    pub fn git_object_lookup(
        out: *mut *mut git_object,
        repo: *mut git_repository,
        id: *const Oid,
        kind: c_int,
    ) -> c_int;
    /// `git_object_type`. PENDING(l-obj)
    pub fn git_object_type(obj: *const git_object) -> c_int;
    /// `git_object_id`. PENDING(l-obj)
    pub fn git_object_id(obj: *const git_object) -> *const Oid;
    /// `git_object_free`. PENDING(l-obj)
    pub fn git_object_free(obj: *mut git_object);
    /// `git_object_peel`. PENDING(l-obj)
    pub fn git_object_peel(out: *mut *mut git_object, obj: *const git_object, kind: c_int)
    -> c_int;
    /// `git_object_dup`. PENDING(l-obj)
    pub fn git_object_dup(out: *mut *mut git_object, obj: *mut git_object) -> c_int;
    /// `git_tag_target_id`. PENDING(l-obj)
    pub fn git_tag_target_id(tag: *const git_object) -> *const Oid;

    // signature.c, PENDING(l-obj)
    /// `git_signature_new`. PENDING(l-obj)
    pub fn git_signature_new(
        out: *mut *mut git_signature,
        name: *const c_char,
        email: *const c_char,
        time: i64,
        offset: c_int,
    ) -> c_int;
    /// `git_signature_now`. PENDING(l-obj)
    pub fn git_signature_now(
        out: *mut *mut git_signature,
        name: *const c_char,
        email: *const c_char,
    ) -> c_int;
    /// `git_signature_default`. PENDING(l-obj)
    pub fn git_signature_default(out: *mut *mut git_signature, repo: *mut git_repository) -> c_int;
    /// `git_signature_dup`. PENDING(l-obj)
    pub fn git_signature_dup(out: *mut *mut git_signature, sig: *const git_signature) -> c_int;
    /// `git_signature_free`. PENDING(l-obj)
    pub fn git_signature_free(sig: *mut git_signature);
    /// `git_signature__parse`. PENDING(l-obj)
    pub fn git_signature__parse(
        sig: *mut git_signature,
        buffer_out: *mut *const c_char,
        buffer_end: *const c_char,
        header: *const c_char,
        ender: c_char,
    ) -> c_int;
    /// `git_signature__writebuf`. PENDING(l-obj)
    pub fn git_signature__writebuf(
        buf: *mut GitStr,
        header: *const c_char,
        sig: *const git_signature,
    );

    // commit.c, PENDING(l-commit)
    /// `git_commit_lookup` (object_api.c). PENDING(l-obj)
    pub fn git_commit_lookup(
        out: *mut *mut git_commit,
        repo: *mut git_repository,
        id: *const Oid,
    ) -> c_int;
    /// `git_commit_parentcount`. PENDING(l-commit)
    pub fn git_commit_parentcount(commit: *const git_commit) -> c_uint;
    /// `git_commit_summary`. PENDING(l-commit)
    pub fn git_commit_summary(commit: *mut git_commit) -> *const c_char;
    /// `git_commit_committer`. PENDING(l-commit)
    pub fn git_commit_committer(commit: *const git_commit) -> *const git_signature;
    /// `git_commit_free` (object_api.c). PENDING(l-obj)
    pub fn git_commit_free(commit: *mut git_commit);
}

// ---------------------------------------------------------------------------
// Safe wrappers of the above (PORTING.md §4.6)
// ---------------------------------------------------------------------------

use core::ffi::CStr;
use core::ptr::NonNull;

use crate::util::errors::{ErrorCode, Result};

/// A live repository, as the C passes one around. PENDING(l-repo): becomes
/// the integrated `Repository`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepoPtr(NonNull<git_repository>);

// SAFETY: libgit2's repository may be passed between threads; the
// functions called through `RepoPtr` are those libgit2 lets several
// threads use on one repository (its caches, refdb and config are
// thread-safe), and the contract of `RepoPtr::new` holds for each use.
unsafe impl Send for RepoPtr {}
// SAFETY: as above.
unsafe impl Sync for RepoPtr {}

/// A C string the repository owns, copied out.
fn owned(p: *const c_char) -> Option<Vec<u8>> {
    if p.is_null() {
        return None;
    }
    // SAFETY: a NUL-terminated string of the live repository, read now.
    Some(unsafe { CStr::from_ptr(p) }.to_bytes().to_vec())
}

impl RepoPtr {
    /// The repository at `repo`.
    ///
    /// # Safety
    ///
    /// `repo` is a live repository that stays live while this value (or
    /// a copy) is used, and that may be used on the threads that use it.
    pub unsafe fn new(repo: NonNull<git_repository>) -> RepoPtr {
        RepoPtr(repo)
    }

    /// The repository at `repo`, which C dereferences unchecked (the
    /// barrier turns the panic of a NULL into an error).
    ///
    /// # Safety
    ///
    /// As [`new`](Self::new), for a non-NULL `repo`.
    pub unsafe fn from_raw(repo: *mut git_repository) -> RepoPtr {
        RepoPtr(NonNull::new(repo).expect("NULL repository"))
    }

    /// The raw pointer.
    pub fn as_ptr(self) -> *mut git_repository {
        self.0.as_ptr()
    }

    /// `repo->gitdir`.
    pub fn gitdir(self) -> Option<Vec<u8>> {
        // SAFETY: a live repository (the constructor's contract).
        owned(unsafe { git_repository_path(self.as_ptr()) })
    }

    /// `repo->commondir`.
    pub fn commondir(self) -> Option<Vec<u8>> {
        // SAFETY: as above.
        owned(unsafe { git_repository_commondir(self.as_ptr()) })
    }

    /// `repo->namespace`.
    pub fn namespace(self) -> Option<Vec<u8>> {
        // SAFETY: as above.
        owned(unsafe { git_repository_get_namespace(self.as_ptr()) })
    }

    /// `repo->ident_name` and `repo->ident_email`.
    pub fn ident(self) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        let (mut n, mut e) = (core::ptr::null(), core::ptr::null());
        // SAFETY: as above; the out-parameters are writable.
        unsafe { git_repository_ident(&raw mut n, &raw mut e, self.as_ptr()) };
        (owned(n), owned(e))
    }

    /// `repo->oid_type`.
    pub fn oid_type(self) -> c_int {
        // SAFETY: as above.
        unsafe { git_repository_oid_type(self.as_ptr()) }
    }

    /// `repo->refdb_type`.
    pub fn refdb_type(self) -> c_int {
        // SAFETY: a live repository, whose `refdb_type` (a `git_refdb_t`,
        // an `int`) is at `REPO_REFDB_TYPE_OFFSET` (layout-tested); C sets
        // it when the repository is opened, before any refdb is made.
        unsafe {
            self.as_ptr()
                .wrapping_byte_add(REPO_REFDB_TYPE_OFFSET)
                .cast::<c_int>()
                .read_unaligned()
        }
    }

    /// `git_repository_is_bare`.
    pub fn is_bare(self) -> bool {
        // SAFETY: as above.
        unsafe { git_repository_is_bare(self.as_ptr()) != 0 }
    }

    /// `git_repository_is_worktree`.
    pub fn is_worktree(self) -> bool {
        // SAFETY: as above.
        unsafe { git_repository_is_worktree(self.as_ptr()) != 0 }
    }

    /// `git_repository__configmap_lookup`.
    pub fn configmap(self, item: c_int) -> Result<c_int> {
        let mut v = 0;
        // SAFETY: as above; `v` is writable.
        ErrorCode::check(unsafe {
            git_repository__configmap_lookup(&raw mut v, self.as_ptr(), item)
        })?;
        Ok(v)
    }

    /// `git_repository_refdb__weakptr`: the repository's refdb, without a
    /// count of the caller's.
    pub fn refdb_weak(self) -> Result<*mut c_void> {
        let mut db = core::ptr::null_mut();
        // SAFETY: as above; `db` is writable.
        ErrorCode::check(unsafe { git_repository_refdb__weakptr(&raw mut db, self.as_ptr()) })?;
        Ok(db)
    }

    /// `git_repository_refdb`: the refdb, with a count the caller drops.
    pub fn refdb(self) -> Result<*mut c_void> {
        let mut db = core::ptr::null_mut();
        // SAFETY: as above.
        ErrorCode::check(unsafe { git_repository_refdb(&raw mut db, self.as_ptr()) })?;
        Ok(db)
    }
}

/// `git_repository__fsync_gitdir`.
pub fn fsync_gitdir() -> bool {
    // SAFETY: a C `bool` global with the layout of `AtomicBool`; C writes
    // it only through `git_libgit2_opts`.
    unsafe { git_repository__fsync_gitdir.load(core::sync::atomic::Ordering::Relaxed) }
}

/// An owned `git_signature`, freed with `git_signature_free`. PENDING(l-obj)
#[derive(Debug)]
pub struct SigBox(NonNull<git_signature>);

// SAFETY: a signature owns its strings; nothing ties it to a thread.
unsafe impl Send for SigBox {}
// SAFETY: shared access only reads.
unsafe impl Sync for SigBox {}

impl SigBox {
    /// Adopt `out` from a C constructor's result.
    fn adopt(r: c_int, out: *mut git_signature) -> Result<SigBox> {
        if r < 0 {
            // SAFETY: NULL or a signature the failed call left us.
            unsafe { git_signature_free(out) };
            ErrorCode::check(r)?;
        }
        NonNull::new(out).map(SigBox).ok_or(ErrorCode::GENERIC)
    }

    /// `git_signature_now(name, email)`.
    pub fn now(name: &CStr, email: &CStr) -> Result<SigBox> {
        let mut out = core::ptr::null_mut();
        // SAFETY: NUL-terminated strings; `out` writable.
        let r = unsafe { git_signature_now(&raw mut out, name.as_ptr(), email.as_ptr()) };
        Self::adopt(r, out)
    }

    /// `git_signature_new(name, email, time, offset)`.
    pub fn new(name: &CStr, email: &CStr, time: i64, offset: c_int) -> Result<SigBox> {
        let mut out = core::ptr::null_mut();
        // SAFETY: as above.
        let r =
            unsafe { git_signature_new(&raw mut out, name.as_ptr(), email.as_ptr(), time, offset) };
        Self::adopt(r, out)
    }

    /// `git_signature_default(repo)`.
    pub fn default_for(repo: RepoPtr) -> Result<SigBox> {
        let mut out = core::ptr::null_mut();
        // SAFETY: a live repository; `out` writable.
        let r = unsafe { git_signature_default(&raw mut out, repo.as_ptr()) };
        Self::adopt(r, out)
    }

    /// `git_signature_dup(sig)`.
    ///
    /// # Safety
    ///
    /// `sig` is a valid signature.
    pub unsafe fn dup(sig: *const git_signature) -> Result<SigBox> {
        let mut out = core::ptr::null_mut();
        // SAFETY: per the contract.
        let r = unsafe { git_signature_dup(&raw mut out, sig) };
        Self::adopt(r, out)
    }

    /// Adopt a signature.
    ///
    /// # Safety
    ///
    /// `p` is NULL or a signature from libgit2's allocator that the caller
    /// gives up.
    pub unsafe fn from_raw(p: *mut git_signature) -> Option<SigBox> {
        NonNull::new(p).map(SigBox)
    }

    /// The raw pointer.
    pub fn as_ptr(&self) -> *mut git_signature {
        self.0.as_ptr()
    }

    /// Give the signature away.
    pub fn into_raw(self) -> *mut git_signature {
        let p = self.0.as_ptr();
        core::mem::forget(self);
        p
    }
}

impl Drop for SigBox {
    fn drop(&mut self) {
        // SAFETY: a signature we own.
        unsafe { git_signature_free(self.0.as_ptr()) };
    }
}

/// An owned `git_object`, freed with `git_object_free`. PENDING(l-obj)
pub struct ObjBox(NonNull<git_object>);

impl ObjBox {
    /// `git_object_lookup(repo, id, kind)`.
    pub fn lookup(repo: RepoPtr, id: &Oid, kind: c_int) -> Result<ObjBox> {
        let mut out = core::ptr::null_mut();
        // SAFETY: a live repository; `id` valid for the call; `out` writable.
        ErrorCode::check(unsafe { git_object_lookup(&raw mut out, repo.as_ptr(), id, kind) })?;
        NonNull::new(out).map(ObjBox).ok_or(ErrorCode::GENERIC)
    }

    /// `git_object_type`.
    pub fn kind(&self) -> c_int {
        // SAFETY: a live object.
        unsafe { git_object_type(self.0.as_ptr()) }
    }

    /// `git_object_id`.
    pub fn id(&self) -> Oid {
        // SAFETY: a live object.
        let id = unsafe { git_object_id(self.0.as_ptr()) };
        // SAFETY: the object's id, alive as long as it.
        unsafe { *id }
    }

    /// `git_tag_target_id` of a tag object.
    pub fn tag_target_id(&self) -> Option<Oid> {
        if self.kind() != 4 {
            return None;
        }
        // SAFETY: a live tag object.
        let p = unsafe { git_tag_target_id(self.0.as_ptr()) };
        // SAFETY: NULL or the tag's id, alive with it.
        unsafe { p.as_ref() }.copied()
    }

    /// `git_object_peel(self, kind)`.
    pub fn peel(&self, kind: c_int) -> Result<ObjBox> {
        let mut out = core::ptr::null_mut();
        // SAFETY: a live object; `out` writable.
        ErrorCode::check(unsafe { git_object_peel(&raw mut out, self.0.as_ptr(), kind) })?;
        NonNull::new(out).map(ObjBox).ok_or(ErrorCode::GENERIC)
    }

    /// `git_object_dup(self)`.
    pub fn dup(&self) -> Result<ObjBox> {
        let mut out = core::ptr::null_mut();
        // SAFETY: as above.
        ErrorCode::check(unsafe { git_object_dup(&raw mut out, self.0.as_ptr()) })?;
        NonNull::new(out).map(ObjBox).ok_or(ErrorCode::GENERIC)
    }

    /// Give the object away.
    pub fn into_raw(self) -> *mut git_object {
        let p = self.0.as_ptr();
        core::mem::forget(self);
        p
    }
}

impl Drop for ObjBox {
    fn drop(&mut self) {
        // SAFETY: an object we own.
        unsafe { git_object_free(self.0.as_ptr()) };
    }
}

/// `git_object__is_valid(repo, id, kind)`.
pub fn object_is_valid(repo: RepoPtr, id: &Oid, kind: c_int) -> bool {
    // SAFETY: a live repository; `id` valid for the call.
    unsafe { git_object__is_valid(repo.as_ptr(), id, kind) }
}

/// An owned `git_commit`. PENDING(l-commit)
pub struct CommitBox(NonNull<git_commit>);

impl CommitBox {
    /// `git_commit_lookup`.
    pub fn lookup(repo: RepoPtr, id: &Oid) -> Result<CommitBox> {
        let mut out = core::ptr::null_mut();
        // SAFETY: a live repository; `out` writable.
        ErrorCode::check(unsafe { git_commit_lookup(&raw mut out, repo.as_ptr(), id) })?;
        NonNull::new(out).map(CommitBox).ok_or(ErrorCode::GENERIC)
    }

    /// `git_commit_parentcount`.
    pub fn parentcount(&self) -> c_uint {
        // SAFETY: a live commit.
        unsafe { git_commit_parentcount(self.0.as_ptr()) }
    }

    /// `git_commit_summary`, copied (empty if NULL).
    pub fn summary(&self) -> Option<Vec<u8>> {
        // SAFETY: a live commit; the summary is cached in it.
        owned(unsafe { git_commit_summary(self.0.as_ptr()) })
    }

    /// `git_commit_committer`, alive as long as the commit.
    pub fn committer(&self) -> *const git_signature {
        // SAFETY: a live commit.
        unsafe { git_commit_committer(self.0.as_ptr()) }
    }
}

impl Drop for CommitBox {
    fn drop(&mut self) {
        // SAFETY: a commit we own.
        unsafe { git_commit_free(self.0.as_ptr()) };
    }
}

unsafe extern "C" {
    /// `git_libgit2_init` (libgit2.c). PENDING(l-repo)
    fn git_libgit2_init() -> c_int;
}

/// Initialize the library once for the safe API's users (a count C never
/// drops: the library stays initialized). PENDING(l-repo)
pub fn libgit2_init_once() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    // SAFETY: `git_libgit2_init` may be called at any time, on any thread.
    ONCE.call_once(|| unsafe {
        git_libgit2_init();
    });
}
