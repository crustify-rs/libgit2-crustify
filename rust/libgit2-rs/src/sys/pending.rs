//! Functions of modules that another work package is porting concurrently,
//! called through their C symbols until both have landed (PORTING.md §3).
//! Each entry carries a `// PENDING(<wp>)` comment.

// PENDING(l-obj): what the repository (WP l-repo) calls of grafts.c,
// object.c and object_api.c.
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

/// The symbolic target of the repository's `HEAD`, read from its refdb
/// directly (`None` if `HEAD` is not symbolic): config_file.c's
/// `conditional_match_onbranch` (the higher-level functions read the
/// repository's config, which is being read).
pub fn repository_head_symbolic_target(
    repo: &crate::repository::GitRepository,
) -> crate::util::errors::Result<Option<Vec<u8>>> {
    use crate::refs::ffi as r;
    use crate::util::errors::ErrorCode;
    // C takes a reference (`git_repository_refdb`) and drops it at the
    // end; the repository keeps the refdb live meanwhile either way.
    let refdb = repo.refdb_weak()?;
    let mut head = core::ptr::null_mut();
    // SAFETY: `head` is writable; `refdb` is the repository's live refdb;
    // the name is NUL-terminated.
    ErrorCode::check(unsafe {
        crate::refdb::ffi::git_refdb_lookup(&raw mut head, refdb.as_ptr(), c"HEAD".as_ptr())
    })?;
    // SAFETY: `head` is the live reference just returned.
    let result = if unsafe { r::git_reference_type(head) }
        != crate::repository::handles::GIT_REFERENCE_SYMBOLIC
    {
        None
    } else {
        // SAFETY: as above; the target is NUL-terminated (or NULL) and
        // lives as long as `head`, which it is copied before.
        let p = unsafe { r::git_reference_symbolic_target(head) };
        Some(if p.is_null() {
            Vec::new()
        } else {
            // SAFETY: as above.
            unsafe { core::ffi::CStr::from_ptr(p) }.to_bytes().to_vec()
        })
    };
    // SAFETY: `head` is ours, released once.
    unsafe { r::git_reference_free(head) };
    Ok(result)
}

// PENDING(l-obj): what the index (WP l-index) calls of
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
    opaque!(git_grafts, git_object, git_commit_graph_file,);

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
    /// `git_reference`.
    #[allow(non_camel_case_types)]
    pub type git_reference = crate::refs::GitReference;
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

    }
}

/// What the revwalk (WP l-revwalk) calls of modules that other L work
/// packages port concurrently, through their C symbols, with the opaque
/// C types they take. The integration step replaces each with the Rust
/// call of its module.
pub mod l_revwalk {
    use core::ffi::c_int;
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
    /// `git_reference`.
    #[allow(non_camel_case_types)]
    pub type git_reference = crate::refs::GitReference;
    /// `git_reference_iterator`.
    #[allow(non_camel_case_types)]
    pub type git_reference_iterator = crate::refdb::git_reference_iterator;
    /// `git_tree_entry`.
    #[allow(non_camel_case_types)]
    pub type git_tree_entry = crate::tree::GitTreeEntry;

    opaque! {
        /// `git_object`. PENDING(l-obj)
        git_object;
    }

    pub use crate::odb::{GIT_OBJECT_ANY, GIT_OBJECT_COMMIT};

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

/// `git_repository`: the repository module's type.
#[allow(non_camel_case_types)]
pub type git_repository = crate::repository::GitRepository;

/// `git_object` (object.h), opaque here. PENDING(l-obj)
#[repr(C)]
pub struct git_object {
    _opaque: [u8; 0],
}

/// `git_commit`: the commit module's type.
#[allow(non_camel_case_types)]
pub type git_commit = crate::commit::GitCommit;

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

/// `git_configmap_item` values (repository.h).
pub mod configmap {
    use core::ffi::c_int;
    /// `GIT_CONFIGMAP_IGNORECASE`.
    pub const IGNORECASE: c_int = crate::repository::GIT_CONFIGMAP_IGNORECASE;
    /// `GIT_CONFIGMAP_PRECOMPOSE`.
    pub const PRECOMPOSE: c_int = 8;
    /// `GIT_CONFIGMAP_LOGALLREFUPDATES`.
    pub const LOGALLREFUPDATES: c_int = 10;
    /// `GIT_CONFIGMAP_FSYNCOBJECTFILES`.
    pub const FSYNCOBJECTFILES: c_int = crate::repository::GIT_CONFIGMAP_FSYNCOBJECTFILES;
}

// The repository and commits are Rust types C sees as opaque; the l-obj
// symbols are also declared by other sections.
#[allow(improper_ctypes, clashing_extern_declarations)]
unsafe extern "C" {
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

    /// `git_commit_lookup` (object_api.c). PENDING(l-obj)
    pub fn git_commit_lookup(
        out: *mut *mut git_commit,
        repo: *mut git_repository,
        id: *const Oid,
    ) -> c_int;
    /// `git_commit_free` (object_api.c). PENDING(l-obj)
    pub fn git_commit_free(commit: *mut git_commit);
}

// ---------------------------------------------------------------------------
// Safe wrappers of the above (PORTING.md §4.6)
// ---------------------------------------------------------------------------

use core::ffi::CStr;
use core::ptr::NonNull;

use crate::util::errors::{ErrorCode, Result};

/// A live repository, as the C passes one around (a `git_repository *` of
/// the repository module).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepoPtr(NonNull<git_repository>);

// SAFETY: libgit2's repository may be passed between threads; the
// functions called through `RepoPtr` are those libgit2 lets several
// threads use on one repository (its caches, refdb and config are
// thread-safe; `GitRepository` is `Sync`), and the contract of
// `RepoPtr::new` holds for each use.
unsafe impl Send for RepoPtr {}
// SAFETY: as above.
unsafe impl Sync for RepoPtr {}

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

    /// The repository.
    fn get<'a>(self) -> &'a git_repository {
        // SAFETY: live while this value is used (the constructor's
        // contract); `GitRepository` is `Sync`.
        unsafe { self.0.as_ref() }
    }

    /// `repo->gitdir`.
    pub fn gitdir(self) -> Option<Vec<u8>> {
        self.get().gitdir().map(|s| s.to_bytes().to_vec())
    }

    /// `repo->commondir`.
    pub fn commondir(self) -> Option<Vec<u8>> {
        self.get().commondir().map(|s| s.to_bytes().to_vec())
    }

    /// `repo->namespace`.
    pub fn namespace(self) -> Option<Vec<u8>> {
        self.get().namespace().map(|s| s.to_bytes().to_vec())
    }

    /// `repo->ident_name` and `repo->ident_email`.
    pub fn ident(self) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        let r = self.get();
        (
            r.ident_name().map(|s| s.to_bytes().to_vec()),
            r.ident_email().map(|s| s.to_bytes().to_vec()),
        )
    }

    /// `repo->oid_type`.
    pub fn oid_type(self) -> c_int {
        self.get().oid_type_raw() as c_int
    }

    /// `repo->refdb_type`.
    pub fn refdb_type(self) -> c_int {
        self.get().refdb_type()
    }

    /// `git_repository_is_bare`.
    pub fn is_bare(self) -> bool {
        self.get().is_bare()
    }

    /// `git_repository_is_worktree`.
    pub fn is_worktree(self) -> bool {
        self.get().is_worktree()
    }

    /// `git_repository__configmap_lookup`.
    pub fn configmap(self, item: c_int) -> Result<c_int> {
        self.get().configmap_lookup(item)
    }

    /// `git_repository_refdb__weakptr`: the repository's refdb, without a
    /// count of the caller's.
    pub fn refdb_weak(self) -> Result<*mut crate::refdb::GitRefdb> {
        self.get().refdb_weak().map(NonNull::as_ptr)
    }

    /// `git_repository_refdb`: the refdb, with a count the caller drops.
    pub fn refdb(self) -> Result<*mut crate::refdb::GitRefdb> {
        let p = self.get().refdb_weak()?;
        // SAFETY: the repository's live refdb, which starts with a
        // `git_refcount`; the reference goes to the caller.
        unsafe { crate::repository::handles::refcount(p.cast()) }.inc();
        Ok(p.as_ptr())
    }
}

/// `git_repository__fsync_gitdir`.
pub fn fsync_gitdir() -> bool {
    crate::repository::fsync_gitdir()
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

/// An owned `git_commit` (looked up and freed through object_api.c,
/// PENDING(l-obj)).
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
        unsafe { crate::commit::ffi::git_commit_parentcount(self.0.as_ptr()) }
    }

    /// `git_commit_summary`'s value, computed here: C's caches it in the
    /// commit, a cached object other threads of the (shared) repository may
    /// hold too; computing it locally writes nothing shared.
    pub fn summary(&self) -> Option<Vec<u8>> {
        // SAFETY: a live commit; its message is set by the parse and
        // unchanged after.
        let m = unsafe { crate::commit::message_ptr(self.0.as_ptr()) };
        if m.is_null() {
            return Some(Vec::new());
        }
        // SAFETY: the commit's NUL-terminated message, live with the commit.
        let msg = unsafe { CStr::from_ptr(m) }.to_bytes();
        Some(crate::commit::summary_of(msg))
    }

    /// `git_commit_committer`, alive as long as the commit.
    pub fn committer(&self) -> *const git_signature {
        // SAFETY: a live commit (l-commit's `GitSignature` and this
        // `git_signature` both mirror the public `git_signature`).
        unsafe { crate::commit::ffi::git_commit_committer(self.0.as_ptr()) }.cast()
    }
}

impl Drop for CommitBox {
    fn drop(&mut self) {
        // SAFETY: a commit we own.
        unsafe { git_commit_free(self.0.as_ptr()) };
    }
}
