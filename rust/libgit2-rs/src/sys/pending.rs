//! Functions of modules that another work package is porting concurrently,
//! called through their C symbols until both have landed (PORTING.md §3).
//! Each entry carries a `// PENDING(<wp>)` comment.

use core::ffi::c_void;

// PENDING(pool): `src/util/pool.c` (in PORTING.md §1's list; not yet
// ported). `git_futils_mkdir_relative`'s path cache allocates from the
// caller's (C-owned) pool.
unsafe extern "C" {
    /// `git_pool_malloc`. PENDING(pool)
    pub fn git_pool_malloc(pool: *mut c_void, items: usize) -> *mut c_void;
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
    pub fn git_repository_refdb__weakptr(out: *mut *mut c_void, repo: *mut git_repository) -> c_int;
    /// `git_repository_refdb`. PENDING(l-repo)
    pub fn git_repository_refdb(out: *mut *mut c_void, repo: *mut git_repository) -> c_int;
    /// `git_repository__configmap_lookup`. PENDING(l-repo)
    pub fn git_repository__configmap_lookup(out: *mut c_int, repo: *mut git_repository, item: c_int) -> c_int;
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
    pub fn git_repository_init(out: *mut *mut git_repository, path: *const c_char, is_bare: c_uint) -> c_int;
    /// `git_repository_free`. PENDING(l-repo)
    pub fn git_repository_free(repo: *mut git_repository);
    /// `git_repository__fsync_gitdir`, written by `git_libgit2_opts`. PENDING(l-repo)
    pub static git_repository__fsync_gitdir: core::sync::atomic::AtomicBool;

    // object.c, object_api.c, tag.c, PENDING(l-obj)
    /// `git_object__is_valid`. PENDING(l-obj)
    pub fn git_object__is_valid(repo: *mut git_repository, id: *const Oid, kind: c_int) -> bool;
    /// `git_object_lookup`. PENDING(l-obj)
    pub fn git_object_lookup(out: *mut *mut git_object, repo: *mut git_repository, id: *const Oid, kind: c_int) -> c_int;
    /// `git_object_type`. PENDING(l-obj)
    pub fn git_object_type(obj: *const git_object) -> c_int;
    /// `git_object_id`. PENDING(l-obj)
    pub fn git_object_id(obj: *const git_object) -> *const Oid;
    /// `git_object_free`. PENDING(l-obj)
    pub fn git_object_free(obj: *mut git_object);
    /// `git_object_peel`. PENDING(l-obj)
    pub fn git_object_peel(out: *mut *mut git_object, obj: *const git_object, kind: c_int) -> c_int;
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
    pub fn git_signature_now(out: *mut *mut git_signature, name: *const c_char, email: *const c_char) -> c_int;
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
    pub fn git_signature__writebuf(buf: *mut GitStr, header: *const c_char, sig: *const git_signature);

    // commit.c, PENDING(l-commit)
    /// `git_commit_lookup` (object_api.c). PENDING(l-obj)
    pub fn git_commit_lookup(out: *mut *mut git_commit, repo: *mut git_repository, id: *const Oid) -> c_int;
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
                .cast::<u8>()
                .add(REPO_REFDB_TYPE_OFFSET)
                .cast::<c_int>()
                .read()
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
        ErrorCode::check(unsafe { git_repository__configmap_lookup(&raw mut v, self.as_ptr(), item) })?;
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
        let r = unsafe { git_signature_new(&raw mut out, name.as_ptr(), email.as_ptr(), time, offset) };
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
        // SAFETY: a live object, whose id lives as long.
        unsafe { *git_object_id(self.0.as_ptr()) }
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
