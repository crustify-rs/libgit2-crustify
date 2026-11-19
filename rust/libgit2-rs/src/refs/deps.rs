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

pub use crate::object::ffi::{git_object_dup, git_object_free, git_object_lookup, git_object_peel};
pub use crate::signature::ffi::{
    git_signature__parse, git_signature__writebuf, git_signature_default, git_signature_dup,
    git_signature_free, git_signature_new, git_signature_now,
};

// Safe wrappers of the above (PORTING.md §4.6).

use core::ffi::CStr;
use core::ptr::NonNull;

use crate::util::errors::{ErrorCode, Result};

/// A live repository, as the C passes one around (a `git_repository *` of
/// the repository module).
///
/// `#[repr(transparent)]` over the pointer, so an export may take an
/// `Option<RepoPtr>` where C passes a `git_repository *` (NULL is `None`).
/// Every `RepoPtr` comes from [`new`](Self::new) / [`from_raw`](Self::from_raw)
/// (or is such a parameter, whose export's contract is the same), so the
/// repository is live while the value is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct RepoPtr(NonNull<git_repository>);

// SAFETY: the one field is a pointer to a `GitRepository`, which is
// `Send + Sync` (all its state is behind atomics, locks or `OnceLock`s;
// it is only reached through `&`). `RepoPtr` only forms a `&GitRepository`
// (`get`), so moving or sharing the handle is moving or sharing a
// `&GitRepository`; the constructors' contract makes the repository live
// for every use, on whichever thread uses it.
unsafe impl Send for RepoPtr {}
// SAFETY: as for `Send`: a `&RepoPtr` gives only `&GitRepository`.
unsafe impl Sync for RepoPtr {}
const _: () = {
    fn sync_send<T: Send + Sync>() {}
    let _ = sync_send::<git_repository>;
};

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

    /// The repository, borrowed for as long as this copy of the handle
    /// is (no unbounded lifetime: a caller cannot keep the reference past
    /// its use of the handle).
    pub fn get(&self) -> &git_repository {
        // SAFETY: the pointer is non-null and aligned (a `NonNull` to a
        // repository), and the repository is live while this value is used
        // (the constructors' contract), which covers this borrow of `self`;
        // `GitRepository` is only read through `&` (its state is
        // interior-mutable), so no `&mut` to it exists.
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

/// A valid `git_signature` borrowed for `'a`: aligned, its `name` and
/// `email` NULL or NUL-terminated strings (C's are never NULL; the Rust
/// readers take NULL as empty), and nothing writes or frees the signature
/// or its strings during `'a`.
///
/// `GitSignature`'s fields are public raw pointers, so a `&GitSignature`
/// alone does not promise valid strings; this handle does, by its
/// constructors: [`SigBox::sig`], [`CommitBox::committer`] and the
/// `unsafe` [`from_raw`](Self::from_raw) for a pointer C passed.
#[derive(Clone, Copy, Debug)]
pub struct SigRef<'a> {
    ptr: NonNull<git_signature>,
    _life: core::marker::PhantomData<&'a git_signature>,
}

impl<'a> SigRef<'a> {
    /// The signature at `p`, or `None` for NULL.
    ///
    /// # Safety
    ///
    /// `p` is NULL, or points to an aligned `git_signature` whose `name`
    /// and `email` are NULL or NUL-terminated strings, and which (with its
    /// strings) nothing writes or frees during `'a`.
    pub unsafe fn from_raw(p: *const git_signature) -> Option<SigRef<'a>> {
        NonNull::new(p.cast_mut()).map(|ptr| SigRef {
            ptr,
            _life: core::marker::PhantomData,
        })
    }

    /// The signature, for `'a`.
    pub fn get(self) -> &'a git_signature {
        // SAFETY: a valid, aligned signature that nothing writes or frees
        // during `'a` (the type's invariant).
        unsafe { self.ptr.as_ref() }
    }

    /// The pointer, for a C call that reads it during `'a`.
    pub fn as_ptr(self) -> *const git_signature {
        self.ptr.as_ptr()
    }

    /// The safe view of the signature, for `'a`.
    pub fn to_ref(self) -> crate::signature::SignatureRef<'a> {
        let s = self.get();
        // SAFETY: its strings are NULL or NUL-terminated, live and
        // unwritten for `'a` (as above).
        unsafe { crate::signature::SignatureRef::from_raw(s) }
    }

    /// A copy (`git_signature_dup`).
    pub fn to_owned(self) -> Result<crate::signature::Signature> {
        // SAFETY: a valid signature for the call (the type's invariant).
        unsafe { crate::signature::Signature::dup_raw(self.as_ptr()) }
    }

    /// The pointer of an optional signature (NULL for `None`).
    pub fn opt_ptr(s: Option<SigRef<'_>>) -> *const git_signature {
        s.map_or(core::ptr::null(), SigRef::as_ptr)
    }
}

impl<'a> From<&'a crate::signature::Signature> for SigRef<'a> {
    /// The C view of an owned signature: valid by `Signature`'s invariant
    /// (owned, non-NULL strings, written by nothing while borrowed).
    fn from(s: &'a crate::signature::Signature) -> SigRef<'a> {
        SigRef {
            ptr: NonNull::new(s.as_raw().cast_mut()).expect("a reference is not NULL"),
            _life: core::marker::PhantomData,
        }
    }
}

/// An owned `git_signature`, freed with `git_signature_free`.
#[derive(Debug)]
pub struct SigBox(NonNull<git_signature>);

impl SigBox {
    /// Adopt `out` from a C constructor's result.
    ///
    /// # Safety
    ///
    /// `out` is NULL or a signature (from `git_signature_*`) that the call
    /// returning `r` handed over to the caller, who gives it up.
    unsafe fn adopt(r: c_int, out: *mut git_signature) -> Result<SigBox> {
        if r < 0 {
            // SAFETY: NULL or a signature the failed call left us (the
            // contract), freed once.
            unsafe { git_signature_free(out) };
            ErrorCode::check(r)?;
        }
        NonNull::new(out).map(SigBox).ok_or(ErrorCode::GENERIC)
    }

    /// `git_signature_now(name, email)`.
    pub fn now(name: &CStr, email: &CStr) -> Result<SigBox> {
        let mut out = core::ptr::null_mut();
        // SAFETY: NUL-terminated strings (`&CStr`) and our writable local.
        let r = unsafe { git_signature_now(Out::from(&mut out), name.into(), email.into()) };
        // SAFETY: what `git_signature_now` left in `out`, ours.
        unsafe { Self::adopt(r, out) }
    }

    /// `git_signature_new(name, email, time, offset)`.
    pub fn new(name: &CStr, email: &CStr, time: i64, offset: c_int) -> Result<SigBox> {
        let mut out = core::ptr::null_mut();
        // SAFETY: NUL-terminated strings (`&CStr`) and our writable local.
        let r = unsafe {
            git_signature_new(Out::from(&mut out), name.into(), email.into(), time, offset)
        };
        // SAFETY: what `git_signature_new` left in `out`, ours.
        unsafe { Self::adopt(r, out) }
    }

    /// `git_signature_default(repo)`.
    pub fn default_for(repo: RepoPtr) -> Result<SigBox> {
        let mut out = core::ptr::null_mut();
        // SAFETY: a live repository (`RepoPtr`'s invariant) and our
        // writable local.
        let r = unsafe { git_signature_default(Out::from(&mut out), Some(repo.get())) };
        // SAFETY: what `git_signature_default` left in `out`, ours.
        unsafe { Self::adopt(r, out) }
    }

    /// `git_signature_dup(sig)`.
    pub fn dup(sig: SigRef<'_>) -> Result<SigBox> {
        let mut out = core::ptr::null_mut();
        // SAFETY: a valid signature for the call (`SigRef`) and our local.
        let r = unsafe { git_signature_dup(Out::from(&mut out), Some(sig.get())) };
        // SAFETY: what `git_signature_dup` left in `out`, ours.
        unsafe { Self::adopt(r, out) }
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

    /// The signature, borrowed.
    pub fn sig(&self) -> SigRef<'_> {
        SigRef {
            ptr: self.0,
            _life: core::marker::PhantomData,
        }
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
///
/// Invariant (its constructors): the pointer is a live object this value
/// holds a reference to.
pub struct ObjBox(NonNull<git_object>);

impl ObjBox {
    /// The object.
    fn obj(&self) -> &crate::object::GitObject {
        // SAFETY: a live object this value holds a reference to (the
        // invariant), so live while `&self` is; its header is not written
        // once the object is shared but for its atomic count.
        unsafe { self.0.cast::<crate::object::GitObject>().as_ref() }
    }

    /// `git_object_lookup(repo, id, kind)`.
    pub fn lookup(repo: RepoPtr, id: &Oid, kind: c_int) -> Result<ObjBox> {
        let mut out = core::ptr::null_mut();
        // SAFETY: a live repository (`RepoPtr`'s invariant); `id` valid for
        // the call; `out` a writable local.
        ErrorCode::check(unsafe {
            git_object_lookup(Out::from(&mut out), Some(repo.get()), Some(id), kind)
        })?;
        NonNull::new(out).map(ObjBox).ok_or(ErrorCode::GENERIC)
    }

    /// `git_object_type`.
    pub fn kind(&self) -> c_int {
        self.obj().raw_kind()
    }

    /// `git_object_id`.
    pub fn id(&self) -> Oid {
        self.obj().cached.oid
    }

    /// `git_tag_target_id` of a tag object.
    pub fn tag_target_id(&self) -> Option<Oid> {
        if self.kind() != crate::object::GIT_OBJECT_TAG {
            return None;
        }
        // SAFETY: our live object, of kind tag (checked): a `git_tag`, only
        // read once published.
        let tag = unsafe { self.0.cast::<crate::tag::GitTag>().as_ref() };
        // SAFETY: a live tag, as the export requires.
        let p = unsafe { crate::tag::ffi::git_tag_target_id(Some(tag)) };
        // SAFETY: NULL or the tag's id, alive with it.
        unsafe { p.as_ref() }.copied()
    }

    /// `git_object_peel(self, kind)`.
    pub fn peel(&self, kind: c_int) -> Result<ObjBox> {
        let mut out = core::ptr::null_mut();
        // SAFETY: a live object; `out` writable.
        ErrorCode::check(unsafe { git_object_peel(Out::from(&mut out), self.0.as_ptr(), kind) })?;
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
    crate::object::is_valid(repo.get(), id, kind)
}

/// An owned `git_commit` (looked up and freed through object_api.c,
///).
pub struct CommitBox(NonNull<git_commit>);

impl CommitBox {
    /// `git_commit_lookup`.
    pub fn lookup(repo: RepoPtr, id: &Oid) -> Result<CommitBox> {
        let mut out: *mut crate::object::GitObject = core::ptr::null_mut();
        // SAFETY: a live repository (`RepoPtr`'s invariant), a valid id and
        // our writable local.
        ErrorCode::check(unsafe {
            crate::object_api::ffi::git_commit_lookup(
                Out::from(&mut out),
                Some(repo.get()),
                Some(id),
            )
        })?;
        NonNull::new(out.cast::<git_commit>())
            .map(CommitBox)
            .ok_or(ErrorCode::GENERIC)
    }

    /// The commit as the commit module's handle.
    fn commit(&self) -> crate::commit::CommitRef<'_> {
        // SAFETY: a live commit of the object table that this value holds a
        // reference to, not freed while it is borrowed.
        unsafe { crate::commit::CommitRef::from_ptr(self.0) }
    }

    /// `git_commit_parentcount`.
    pub fn parentcount(&self) -> c_uint {
        // SAFETY: a live commit.
        unsafe { crate::commit::ffi::git_commit_parentcount(Some(self.commit())) }
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

    /// `git_commit_committer` (NULL as `None`), alive as long as the
    /// commit.
    pub fn committer(&self) -> Option<SigRef<'_>> {
        // SAFETY: a live commit (we own it).
        let p = unsafe { crate::commit::ffi::git_commit_committer(Some(self.commit())) };
        // SAFETY: NULL, or a parsed commit's committer: a valid signature
        // the commit owns, never written after the parse and freed only
        // with the commit, which `&self` keeps alive for the result's
        // lifetime.
        unsafe { SigRef::from_raw(p) }
    }
}

impl Drop for CommitBox {
    fn drop(&mut self) {
        // SAFETY: a commit whose reference we own, released once.
        unsafe { crate::object_api::ffi::git_commit_free(self.0.as_ptr().cast()) };
    }
}
