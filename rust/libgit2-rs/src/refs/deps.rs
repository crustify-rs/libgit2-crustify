//! What refs.c, refdb.c, refdb_fs.c and reflog.c use of the other
//! modules: the owners' types under the C names, their `ffi` exports, and
//! small owning wrappers over them.

use crate::util::cabi::Out;
use core::ffi::{c_int, c_uint};

use crate::oid::Oid;

/// `git_repository`: the repository module's type.
#[allow(non_camel_case_types)]
pub type git_repository = crate::repository::GitRepository;

/// `git_object`: the object module's type.
#[allow(non_camel_case_types)]
pub type git_object = crate::object::GitObject;

/// `git_commit`: the commit module's type.
#[allow(non_camel_case_types)]
pub type git_commit = crate::commit::GitCommit;

/// `git_signature`: the signature module's type.
#[allow(non_camel_case_types)]
pub type git_signature = crate::signature::GitSignature;

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

pub use crate::object::ffi::{
    git_object__is_valid, git_object_dup, git_object_free, git_object_id, git_object_lookup,
    git_object_peel, git_object_type,
};
pub use crate::signature::ffi::{
    git_signature__parse, git_signature__writebuf, git_signature_default, git_signature_dup,
    git_signature_free, git_signature_new, git_signature_now,
};

/// `git_tag_target_id` of a tag object.
///
/// # Safety
///
/// `tag` is a live tag object.
pub unsafe fn git_tag_target_id(tag: *const git_object) -> *const Oid {
    // SAFETY: `tag` is a live tag object (the contract), and a tag object
    // is a `git_tag`, only read once published.
    let tag = unsafe { tag.cast::<crate::tag::GitTag>().as_ref() };
    // SAFETY: NULL or a live tag, as the export requires.
    unsafe { crate::tag::ffi::git_tag_target_id(tag) }
}

/// `git_commit_lookup` (object_api.c).
///
/// # Safety
///
/// As the export's.
pub unsafe fn git_commit_lookup(
    out: *mut *mut git_commit,
    repo: *mut git_repository,
    id: Option<&Oid>,
) -> c_int {
    // SAFETY: per the contract.
    unsafe { crate::object_api::ffi::git_commit_lookup(out.cast(), repo, id) }
}

/// `git_commit_free` (object_api.c).
///
/// # Safety
///
/// As the export's.
pub unsafe fn git_commit_free(commit: *mut git_commit) {
    // SAFETY: per the contract.
    unsafe { crate::object_api::ffi::git_commit_free(commit.cast()) }
}

// Safe wrappers of the above (PORTING.md §4.6).

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
        // SAFETY: the repository's live refdb, which its count keeps live;
        // the reference goes to the caller.
        unsafe { ffibox::CShared::increment_count(p.as_ptr()) };
        Ok(p.as_ptr())
    }
}

/// `git_repository__fsync_gitdir`.
pub fn fsync_gitdir() -> bool {
    crate::repository::fsync_gitdir()
}

/// An owned `git_signature`, freed with `git_signature_free`.
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
        let r = unsafe { git_signature_now(&raw mut out, name.into(), email.into()) };
        Self::adopt(r, out)
    }

    /// `git_signature_new(name, email, time, offset)`.
    pub fn new(name: &CStr, email: &CStr, time: i64, offset: c_int) -> Result<SigBox> {
        let mut out = core::ptr::null_mut();
        // SAFETY: as above.
        let r = unsafe { git_signature_new(&raw mut out, name.into(), email.into(), time, offset) };
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
        let r = unsafe { git_signature_dup(Out::from(&mut out), sig) };
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

/// An owned `git_object`, freed with `git_object_free`.
pub struct ObjBox(NonNull<git_object>);

impl ObjBox {
    /// `git_object_lookup(repo, id, kind)`.
    pub fn lookup(repo: RepoPtr, id: &Oid, kind: c_int) -> Result<ObjBox> {
        let mut out = core::ptr::null_mut();
        // SAFETY: a live repository; `id` valid for the call; `out` writable.
        ErrorCode::check(unsafe {
            git_object_lookup(&raw mut out, repo.as_ptr(), Some(id), kind)
        })?;
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
        if self.kind() != crate::object::GIT_OBJECT_TAG {
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
        ErrorCode::check(unsafe { git_object_dup(Out::from(&mut out), self.0.as_ptr()) })?;
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
    unsafe { git_object__is_valid(repo.as_ptr(), Some(id), kind) }
}

/// An owned `git_commit` (looked up and freed through object_api.c,
///).
pub struct CommitBox(NonNull<git_commit>);

impl CommitBox {
    /// `git_commit_lookup`.
    pub fn lookup(repo: RepoPtr, id: &Oid) -> Result<CommitBox> {
        let mut out = core::ptr::null_mut();
        // SAFETY: a live repository; `out` writable.
        ErrorCode::check(unsafe { git_commit_lookup(&raw mut out, repo.as_ptr(), Some(id)) })?;
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
        // SAFETY: a live commit.
        unsafe { crate::commit::ffi::git_commit_committer(self.0.as_ptr()) }
    }
}

impl Drop for CommitBox {
    fn drop(&mut self) {
        // SAFETY: a commit we own.
        unsafe { git_commit_free(self.0.as_ptr()) };
    }
}
