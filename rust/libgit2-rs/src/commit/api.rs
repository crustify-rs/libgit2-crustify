//! The safe API of commits (PORTING.md §5): [`Commit`] (an [`Object`] of
//! type commit), its signatures ([`SignatureRef`]), [`CommitBuilder`] to
//! write a commit of a tree and parents, [`CommitCreateOptions`] for the
//! commits made from the repository's state (`git_commit_create_from_*`,
//! `git_commit_amend_from_*`), [`AmendOptions`] for `git_commit_amend`, and
//! the signed-commit helpers.
//!
//! A [`Commit`] holds one reference to a libgit2 commit object (from the
//! repository's cache, or parsed from bytes), released on drop; `Clone`
//! takes another. Its accessors read the fields the parse set, which
//! nothing writes afterwards; the lazily computed summary and body are
//! computed and kept in the Rust value (`OnceLock`s), never written into
//! the shared object. So a `Commit` is `Send` and `Sync`.
//!
//! Signing: the closures of [`CommitBuilder::write_signed`] and
//! [`CommitCreateOptions::signer`] get the commit's bytes and return the
//! signature to add as a header (`None`: unsigned), as C's
//! `git_commit_signature_cb` with `git_commitbuilder_add_header`. A panic
//! in one stops the operation and is resumed after it returns.

use crate::util::cabi::CStrArg;
use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr::{self, NonNull};
use std::any::Any;
use std::ffi::CString;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::OnceLock;

use super::{
    CreateArgs, GitCommit, GitCommitCreateOptions, GitCommitHeader, GitCommitbuilder, GitSignature,
    SigOut,
};
use crate::object::{GIT_OBJECT_COMMIT, GIT_OBJECT_TREE, Object, ObjectType};
use crate::oid::{Oid, OidType};
use crate::repository::Repository;
use crate::signature::{SignatureRef, Time};
use crate::tree::api::Tree;
use crate::util::errors::{Error, ErrorClass, ErrorCode};
use crate::util::str::GitStr;

fn sig_out<'a>(s: &SignatureRef<'a>) -> SigOut<'a> {
    SigOut {
        name: Some(s.name()),
        email: Some(s.email()),
        when: s.when().raw(),
    }
}

fn invalid(msg: &str) -> Error {
    Error::with(ErrorCode::INVALID, ErrorClass::INVALID, msg)
}

fn cstring(s: &[u8], what: &str) -> Result<CString, Error> {
    CString::new(s).map_err(|_| invalid(&format!("{what} contains a NUL byte")))
}

fn opt_cstring(s: Option<&[u8]>, what: &str) -> Result<Option<CString>, Error> {
    s.map(|s| cstring(s, what)).transpose()
}

fn opt_ptr(s: &Option<CString>) -> *const c_char {
    s.as_ref().map_or(ptr::null(), |s| s.as_ptr())
}

/// A C status as the safe API's result.
fn check(e: c_int) -> Result<(), Error> {
    if e < 0 {
        Err(Error::from(super::ec(e)))
    } else {
        Ok(())
    }
}

/// The bytes of a C string the commit owns.
///
/// # Safety
///
/// `p` is NULL or a NUL-terminated string that lives (unwritten) for `'a`.
unsafe fn cbytes<'a>(p: *const c_char) -> &'a [u8] {
    // SAFETY: the caller's contract.
    unsafe { super::opt_cbytes(p) }.unwrap_or(b"")
}

/// A signature as C reads it (`git_signature`), its strings owned here.
struct CSig {
    _name: CString,
    _email: CString,
    raw: GitSignature,
}

impl CSig {
    fn new(s: &SignatureRef<'_>) -> Result<CSig, Error> {
        let name = cstring(s.name(), "signature name")?;
        let email = cstring(s.email(), "signature email")?;
        let raw = GitSignature {
            // C only reads them (`const git_signature *`).
            name: name.as_ptr().cast_mut(),
            email: email.as_ptr().cast_mut(),
            when: s.when().raw(),
        };
        Ok(CSig {
            _name: name,
            _email: email,
            raw,
        })
    }

    fn opt(s: Option<&SignatureRef<'_>>) -> Result<Option<CSig>, Error> {
        s.map(CSig::new).transpose()
    }

    /// The pointer C takes (NULL for `None`), live while `s` is.
    fn ptr(s: &Option<CSig>) -> *const GitSignature {
        s.as_ref().map_or(ptr::null(), |s| &raw const s.raw)
    }
}

/// A commit of a repository (`git_commit`), or parsed from bytes
/// ([`Commit::parse`]).
pub struct Commit<'r> {
    obj: Object<'r>,
    summary: OnceLock<Vec<u8>>,
    body: OnceLock<Option<Vec<u8>>>,
}

impl<'r> TryFrom<Object<'r>> for Commit<'r> {
    type Error = Object<'r>;

    /// The object as a commit, or the object back if it is not one.
    fn try_from(obj: Object<'r>) -> Result<Commit<'r>, Object<'r>> {
        if obj.kind() == ObjectType::Commit {
            Ok(Commit::wrap(obj))
        } else {
            Err(obj)
        }
    }
}

impl<'r> From<Commit<'r>> for Object<'r> {
    fn from(c: Commit<'r>) -> Object<'r> {
        c.obj
    }
}

impl<'r> Commit<'r> {
    /// A checked commit object.
    fn wrap(obj: Object<'r>) -> Commit<'r> {
        Commit {
            obj,
            summary: OnceLock::new(),
            body: OnceLock::new(),
        }
    }

    /// Take over one reference to the commit `raw`, of `self`'s repository.
    ///
    /// # Safety
    ///
    /// `raw` is a live commit of `self`'s repository whose reference passes
    /// to the result.
    unsafe fn sibling(&self, raw: NonNull<GitCommit>) -> Commit<'r> {
        // SAFETY: the caller's contract.
        Commit::wrap(unsafe { self.obj.sibling(raw.cast()) })
    }

    /// `git_commit_lookup`.
    pub fn lookup(repo: &'r Repository, id: &Oid) -> Result<Commit<'r>, Error> {
        Ok(Commit::wrap(Object::lookup(
            repo,
            id,
            Some(ObjectType::Commit),
        )?))
    }

    /// `git_commit_lookup_prefix`: by the first `len` hex digits of `id`.
    pub fn lookup_prefix(repo: &'r Repository, id: &Oid, len: usize) -> Result<Commit<'r>, Error> {
        Ok(Commit::wrap(Object::lookup_prefix(
            repo,
            id,
            len,
            Some(ObjectType::Commit),
        )?))
    }

    /// The commit as an object.
    pub fn as_object(&self) -> &Object<'r> {
        &self.obj
    }

    /// The commit as an object.
    pub fn into_object(self) -> Object<'r> {
        self.obj
    }

    fn c(&self) -> *const GitCommit {
        self.obj.as_ptr().as_ptr().cast::<GitCommit>()
    }

    /// `git_commit_id`.
    pub fn id(&self) -> &Oid {
        self.obj.id()
    }

    /// `git_commit_tree_id`: the id of its tree.
    pub fn tree_id(&self) -> &Oid {
        // SAFETY: `self.c()` is the live commit this value holds (a checked
        // commit); only that field is borrowed, which the parse set and
        // nothing writes after.
        unsafe { &(*self.c()).tree_id }
    }

    /// The ids of its parents (`git_commit_parent_id` for each).
    pub fn parent_ids(&self) -> &[Oid] {
        // SAFETY: a live commit, its parents unchanged while `self` lives.
        unsafe { super::parent_ids_of(self.c()) }
    }

    /// `git_commit_parentcount`.
    pub fn parent_count(&self) -> usize {
        self.parent_ids().len()
    }

    fn sig(&self, p: *const GitSignature) -> SignatureRef<'_> {
        if p.is_null() {
            return SignatureRef::new(b"", b"", Time::from_raw(Default::default()));
        }
        // SAFETY: a signature the commit owns, unchanged while it lives.
        let s = unsafe { &*p };
        // SAFETY: its strings are NUL-terminated, owned by the commit and
        // unwritten while it lives (the result borrows `self`).
        unsafe { SignatureRef::from_raw(s) }
    }

    /// `git_commit_author`.
    pub fn author(&self) -> SignatureRef<'_> {
        // SAFETY: `self.c()` is the live commit this value holds.
        let p = unsafe { &raw const (*self.c()).author };
        // SAFETY: a field set by the parse, never written after.
        self.sig(unsafe { p.read() })
    }

    /// `git_commit_committer`.
    pub fn committer(&self) -> SignatureRef<'_> {
        // SAFETY: `self.c()` is the live commit this value holds.
        let p = unsafe { &raw const (*self.c()).committer };
        // SAFETY: a field set by the parse, never written after.
        self.sig(unsafe { p.read() })
    }

    /// `git_commit_time`: the committer's time, in seconds.
    pub fn time(&self) -> i64 {
        self.committer().when().seconds()
    }

    /// `git_commit_time_offset`: the committer's offset, in minutes.
    pub fn time_offset(&self) -> i32 {
        self.committer().when().offset_minutes()
    }

    /// `git_commit_message_raw`: the message as stored.
    pub fn message_raw(&self) -> &[u8] {
        // SAFETY: `self.c()` is the live commit this value holds.
        let p = unsafe { &raw const (*self.c()).raw_message };
        // SAFETY: a field set by the parse, never written after.
        let s = unsafe { p.read() };
        // SAFETY: NULL or a NUL-terminated string the commit owns, unwritten
        // while it lives (the result borrows `self`).
        unsafe { cbytes(s) }
    }

    /// `git_commit_message`: the message without its leading newlines.
    pub fn message(&self) -> &[u8] {
        super::message_of(self.message_raw())
    }

    /// `git_commit_message_encoding`.
    pub fn message_encoding(&self) -> Option<&[u8]> {
        // SAFETY: `self.c()` is the live commit this value holds.
        let p = unsafe { &raw const (*self.c()).message_encoding };
        // SAFETY: a field set by the parse, never written after.
        let s = unsafe { p.read() };
        // SAFETY: NULL or a NUL-terminated string the commit owns, unwritten
        // while it lives (the result borrows `self`).
        unsafe { super::opt_cbytes(s) }
    }

    /// `git_commit_raw_header`.
    pub fn raw_header(&self) -> &[u8] {
        // SAFETY: `self.c()` is the live commit this value holds.
        let p = unsafe { &raw const (*self.c()).raw_header };
        // SAFETY: a field set by the parse, never written after.
        let s = unsafe { p.read() };
        // SAFETY: NULL or a NUL-terminated string the commit owns, unwritten
        // while it lives (the result borrows `self`).
        unsafe { cbytes(s) }
    }

    /// `git_commit_summary`: the first paragraph, on one line.
    pub fn summary(&self) -> &[u8] {
        self.summary
            .get_or_init(|| super::summary_of(self.message()))
    }

    /// `git_commit_body`: the rest of the message, trimmed; `None` if empty.
    pub fn body(&self) -> Option<&[u8]> {
        self.body
            .get_or_init(|| super::body_of(self.message()).map(<[u8]>::to_vec))
            .as_deref()
    }

    /// `git_commit_header_field`: the value of a header (continuation lines
    /// joined with `\n`).
    pub fn header_field(&self, field: impl AsRef<[u8]>) -> Result<Vec<u8>, Error> {
        let mut out = GitStr::new();
        super::header_field(self.raw_header(), field.as_ref(), &mut out).map_err(Error::from)?;
        Ok(out.as_bytes().to_vec())
    }

    /// `git_commit_tree`.
    pub fn tree(&self) -> Result<Tree<'r>, Error> {
        self.obj.need_repo()?;
        // SAFETY: the commit's live repository (checked above).
        let raw =
            unsafe { crate::object::lookup(self.obj.repo_raw(), self.tree_id(), GIT_OBJECT_TREE) }?;
        // SAFETY: a tree of the same repository, with its reference.
        let obj = unsafe { self.obj.sibling(raw) };
        Ok(Tree::wrap(obj))
    }

    /// `git_commit_parent`: the `n`th parent.
    pub fn parent(&self, n: usize) -> Result<Commit<'r>, Error> {
        self.obj.need_repo()?;
        let mut raw = ptr::null_mut();
        let n = u32::try_from(n).unwrap_or(u32::MAX);
        // SAFETY: `raw` writable; a live commit of a repository.
        check(unsafe { super::ffi::git_commit_parent(&raw mut raw, self.c(), n) })?;
        let raw = NonNull::new(raw).ok_or_else(|| Error::from(ErrorCode::GENERIC))?;
        // SAFETY: a commit of the same repository, ours to release.
        Ok(unsafe { self.sibling(raw) })
    }

    /// `git_commit_nth_gen_ancestor`: the `n`th first-parent ancestor (the
    /// commit itself for 0).
    pub fn nth_gen_ancestor(&self, n: u32) -> Result<Commit<'r>, Error> {
        if n > 0 {
            self.obj.need_repo()?;
        }
        let mut raw = ptr::null_mut();
        // SAFETY: `raw` writable; a live commit.
        check(unsafe { super::ffi::git_commit_nth_gen_ancestor(&raw mut raw, self.c(), n) })?;
        let raw = NonNull::new(raw).ok_or_else(|| Error::from(ErrorCode::GENERIC))?;
        // SAFETY: as in `parent`.
        Ok(unsafe { self.sibling(raw) })
    }

    /// `git_commit_amend`: write a commit like this one with the parts
    /// `opts` sets replaced (its parents are kept), and update the
    /// reference `opts` names (which must point to this commit); the new
    /// commit's id.
    pub fn amend(&self, opts: &AmendOptions<'_>) -> Result<Oid, Error> {
        self.obj.need_repo()?;
        let update_ref = opt_cstring(opts.update_ref, "reference name")?;
        let author = CSig::opt(opts.author.as_ref())?;
        let committer = CSig::opt(opts.committer.as_ref())?;
        let encoding = opt_cstring(opts.encoding, "message encoding")?;
        let message = opt_cstring(opts.message, "message")?;
        let tree = opts
            .tree
            .map_or(ptr::null(), |t| t.as_raw_tree().as_ptr().cast_const());
        let mut id = Oid::default();
        // SAFETY: `id` writable; a live commit of a repository; the strings
        // and signatures live (above) during the call; `tree` NULL or a live
        // tree (the C checks that it is the commit's repository's).
        check(unsafe {
            super::ffi::git_commit_amend(
                &raw mut id,
                self.c(),
                opt_ptr(&update_ref),
                CSig::ptr(&author),
                CSig::ptr(&committer),
                opt_ptr(&encoding),
                opt_ptr(&message),
                tree,
            )
        })?;
        Ok(id)
    }
}

impl Commit<'static> {
    /// Parse a commit from its bytes (`git_object__from_raw`), outside any
    /// repository (so without grafts, and [`tree`](Commit::tree) and
    /// [`parent`](Commit::parent) fail).
    pub fn parse(data: impl AsRef<[u8]>, oid_type: OidType) -> Result<Commit<'static>, Error> {
        crate::libgit2::ensure_init()?;
        let data = data.as_ref();
        // SAFETY: `data` readable for its length. The commit copies what it
        // keeps, so it does not borrow `data`.
        let raw = unsafe {
            crate::object::from_raw(
                data.as_ptr().cast(),
                data.len(),
                GIT_OBJECT_COMMIT,
                oid_type.raw(),
            )
        }?;
        // SAFETY: a commit just made, with its reference, of no repository
        // and owning its data.
        Ok(Commit::wrap(unsafe { Object::adopt(raw) }))
    }
}

impl Commit<'_> {
    /// `git_commit_extract_signature`: the value of the signature header
    /// `field` (`None`: `gpgsig`) of the commit `id`, and the rest of the
    /// commit (the signed data). `NOT_FOUND` if it is not signed.
    pub fn extract_signature(
        repo: &Repository,
        id: &Oid,
        field: Option<&[u8]>,
    ) -> Result<(Vec<u8>, Vec<u8>), Error> {
        let field = opt_cstring(field, "signature field")?;
        let mut sig = GitStr::new();
        let mut signed = GitStr::new();
        let mut id = *id;
        // SAFETY: two distinct, valid strings; a live repository; `id`
        // readable; `field` NULL or NUL-terminated.
        check(unsafe {
            super::ffi::git_commit__extract_signature(
                &raw mut sig,
                &raw mut signed,
                repo.raw_ptr(),
                &raw mut id,
                CStrArg::from(field.as_deref()),
            )
        })?;
        Ok((sig.as_bytes().to_vec(), signed.as_bytes().to_vec()))
    }

    /// `git_commit_create_with_signature`: write the commit `content` (as
    /// [`CommitBuilder::to_buffer`] makes it) with the header `field`
    /// (`None`: `gpgsig`) set to `signature` (`None`: unsigned), after
    /// checking that its tree and parents exist; its id.
    pub fn create_with_signature(
        repo: &Repository,
        content: impl AsRef<[u8]>,
        signature: Option<&[u8]>,
        field: Option<&[u8]>,
    ) -> Result<Oid, Error> {
        let content = cstring(content.as_ref(), "commit content")?;
        let signature = opt_cstring(signature, "signature")?;
        let field = opt_cstring(field, "signature field")?;
        let mut id = Oid::default();
        // SAFETY: `id` writable; a live repository; the strings
        // NUL-terminated (or NULL), live during the call.
        check(unsafe {
            super::ffi::git_commit_create_with_signature(
                &raw mut id,
                repo.raw_ptr(),
                content.as_c_str().into(),
                CStrArg::from(signature.as_deref()),
                CStrArg::from(field.as_deref()),
            )
        })?;
        Ok(id)
    }

    /// `git_commit_create_from_tree`: commit `tree` on top of `HEAD` (its
    /// parents: `HEAD` and, during a merge, `MERGE_HEAD`s), updating
    /// `HEAD`; `UNCHANGED` if `tree` is `HEAD`'s and `opts` does not allow
    /// empty commits. Signatures default to the repository's
    /// (`user.name`/`user.email`).
    pub fn create_from_tree(
        repo: &Repository,
        tree: &Tree<'_>,
        message: impl AsRef<[u8]>,
        opts: CommitCreateOptions<'_>,
    ) -> Result<Oid, Error> {
        let message = cstring(message.as_ref(), "message")?;
        // The emptiness check diffs the tree, through its repository: it
        // must be `repo` (C asserts it later, in `git_commit_create_ext`).
        if tree.as_object().repo_raw() != crate::object::repo_ptr(repo) {
            return Err(Error::from(crate::util::errors::assert_arg_failed(
                "git_tree_owner(tree) == repo",
            )));
        }
        let t = tree.as_raw_tree().as_ptr().cast_const();
        // The emptiness check diffs trees, whose drivers read attributes.
        crate::index::api::with_repository_index_locked(repo.raw(), || {
            opts.run(|o| {
                let mut id = Oid::default();
                // SAFETY: `id` writable; a live repository and tree; the
                // message NUL-terminated; valid options (`run`).
                let e = unsafe {
                    super::ffi::git_commit_create_from_tree(
                        &raw mut id,
                        repo.raw_ptr(),
                        t,
                        message.as_ptr(),
                        o,
                    )
                };
                (e, id)
            })
        })
    }

    /// `git_commit_create_from_stage`: as
    /// [`create_from_tree`](Self::create_from_tree) with the tree of the
    /// repository's index.
    pub fn create_from_stage(
        repo: &Repository,
        message: impl AsRef<[u8]>,
        opts: CommitCreateOptions<'_>,
    ) -> Result<Oid, Error> {
        let message = cstring(message.as_ref(), "message")?;
        crate::index::api::with_repository_index_locked(repo.raw(), || {
            opts.run(|o| {
                let mut id = Oid::default();
                // SAFETY: `id` writable; a live repository; the message
                // NUL-terminated; valid options (`run`). The index is the
                // repository's, locked (above).
                let e = unsafe {
                    super::ffi::git_commit_create_from_stage(
                        &raw mut id,
                        repo.raw_ptr(),
                        message.as_ptr(),
                        o,
                    )
                };
                (e, id)
            })
        })
    }

    /// `git_commit_amend_from_tree`: replace `HEAD`'s commit with one of
    /// `tree` (same parents; `message`: `None` keeps the old message and
    /// encoding; the author defaults to the old one, the committer to the
    /// repository's), updating `HEAD`. `allow_empty` does not apply.
    pub fn amend_from_tree(
        repo: &Repository,
        tree: &Tree<'_>,
        message: Option<&[u8]>,
        opts: CommitCreateOptions<'_>,
    ) -> Result<Oid, Error> {
        let message = opt_cstring(message, "message")?;
        let t = tree.as_raw_tree().as_ptr().cast_const();
        opts.run(|o| {
            let mut id = Oid::default();
            // SAFETY: `id` writable; a live repository and tree; the
            // message NULL or NUL-terminated; valid options (`run`).
            let e = unsafe {
                super::ffi::git_commit_amend_from_tree(
                    &raw mut id,
                    repo.raw_ptr(),
                    t,
                    opt_ptr(&message),
                    o,
                )
            };
            (e, id)
        })
    }

    /// `git_commit_amend_from_stage`: as
    /// [`amend_from_tree`](Self::amend_from_tree) with the tree of the
    /// repository's index.
    pub fn amend_from_stage(
        repo: &Repository,
        message: Option<&[u8]>,
        opts: CommitCreateOptions<'_>,
    ) -> Result<Oid, Error> {
        let message = opt_cstring(message, "message")?;
        crate::index::api::with_repository_index_locked(repo.raw(), || {
            opts.run(|o| {
                let mut id = Oid::default();
                // SAFETY: `id` writable; a live repository; the message NULL
                // or NUL-terminated; valid options (`run`). The index is the
                // repository's, locked (above).
                let e = unsafe {
                    super::ffi::git_commit_amend_from_stage(
                        &raw mut id,
                        repo.raw_ptr(),
                        opt_ptr(&message),
                        o,
                    )
                };
                (e, id)
            })
        })
    }
}

impl Clone for Commit<'_> {
    /// `git_commit_dup` (the summary and body caches are copied).
    fn clone(&self) -> Self {
        Commit {
            obj: self.obj.clone(),
            summary: self.summary.clone(),
            body: self.body.clone(),
        }
    }
}

impl core::fmt::Debug for Commit<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Commit").field("id", self.id()).finish()
    }
}

impl Repository {
    /// `git_repository_commit_parents`: the parents a commit made now would
    /// have (`HEAD`'s commit, and the `MERGE_HEAD`s during a merge; none on
    /// an unborn branch).
    pub fn commit_parents(&self) -> Result<Vec<Commit<'_>>, Error> {
        let mut arr = crate::repository::ffi::GitCommitArray {
            commits: ptr::null_mut(),
            count: 0,
        };
        // SAFETY: a live repository; `arr` writable.
        check(unsafe {
            crate::repository::ffi::git_repository_commit_parents(&raw mut arr, self.raw_ptr())
        })?;
        let mut out = Vec::with_capacity(arr.count);
        for i in 0..arr.count {
            // SAFETY: `i < count` readable commit pointers.
            let c = unsafe { super::nth(arr.commits.cast_const(), i) };
            let Some(c) = NonNull::new(c.cast::<crate::object::GitObject>()) else {
                continue;
            };
            // SAFETY: a commit of `self` whose reference the array owned and
            // passes to the result (the array itself is freed below).
            out.push(Commit::wrap(unsafe { Object::from_raw(self, c) }));
        }
        // SAFETY: NULL or the array's `git__allocator` buffer, freed once;
        // its references were taken over above.
        unsafe { crate::util::alloc::free(arr.commits.cast()) };
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Signing
// ---------------------------------------------------------------------------

/// A signing closure: the commit's bytes in, the signature out.
type SignFn<'a> = dyn FnMut(&[u8]) -> Result<Option<Vec<u8>>, Error> + 'a;

/// The state of a signing closure during an operation.
struct Signing<'s, 'a> {
    field: &'s CStr,
    f: &'s mut SignFn<'a>,
    panic: Option<Box<dyn Any + Send>>,
}

impl Signing<'_, '_> {
    /// Run the closure on `contents` and add its signature to them as the
    /// header `field` (`git_commitbuilder_add_header`); C's status.
    fn sign(&mut self, contents: &mut GitStr) -> c_int {
        if self.panic.is_some() {
            return ErrorCode::GENERIC.raw();
        }
        let raw = contents.as_bytes();
        let raw = &raw[..memchr::memchr(0, raw).unwrap_or(raw.len())];
        let sig = match catch_unwind(AssertUnwindSafe(|| (self.f)(raw))) {
            Ok(Ok(None)) => return 0,
            Ok(Ok(Some(sig))) => sig,
            Ok(Err(e)) => return e.set_last().raw(),
            Err(p) => {
                self.panic = Some(p);
                return Error::panicked().set_last().raw();
            }
        };
        // `git_commitbuilder_add_header` takes C strings.
        if memchr::memchr(0, &sig).is_some() {
            return invalid("signature contains a NUL byte").set_last().raw();
        }
        let mut signed = GitStr::new();
        match super::append_header(&mut signed, raw, self.field.to_bytes(), &sig) {
            Ok(()) => {
                contents.swap(&mut signed);
                0
            }
            Err(e) => e.raw(),
        }
    }

    /// Resume a panic of the closure.
    fn finish(self) {
        if let Some(p) = self.panic {
            resume_unwind(p);
        }
    }
}

/// `git_commit_signature_cb` over a [`Signing`].
///
/// # Safety
///
/// `builder` is the builder libgit2 hands the callback; `payload` the
/// `Signing` of the running operation, used by nothing else meanwhile.
unsafe extern "C" fn sign_cb(
    builder: *mut GitCommitbuilder,
    _repo: *mut crate::commit::deps::git_repository,
    _content: *const c_char,
    payload: *mut c_void,
) -> c_int {
    // SAFETY: `payload` is the running operation's `Signing`, used by
    // nothing else during the callback (contract).
    let s = unsafe { &mut *payload.cast::<Signing<'_, '_>>() };
    if builder.is_null() {
        return crate::util::errors::assert_arg_failed("builder").raw();
    }
    // SAFETY: the builder's string is live during the callback, and only
    // this callback uses it meanwhile (contract).
    let contents = unsafe { (*builder).contents };
    // SAFETY: as above: the live string, exclusively ours meanwhile.
    let contents = unsafe { &mut *contents };
    s.sign(contents)
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Options of the commits made from the repository's state
/// ([`Commit::create_from_tree`], [`Commit::create_from_stage`],
/// [`Commit::amend_from_tree`], [`Commit::amend_from_stage`];
/// `git_commit_create_options`). By-value builder; the default makes no
/// empty commits, takes the signatures from the repository's
/// configuration, and does not sign.
#[derive(Default)]
pub struct CommitCreateOptions<'a> {
    allow_empty: bool,
    author: Option<SignatureRef<'a>>,
    committer: Option<SignatureRef<'a>>,
    encoding: Option<&'a [u8]>,
    headers: Vec<(&'a [u8], &'a [u8])>,
    signer: Option<(&'a [u8], Box<SignFn<'a>>)>,
}

impl core::fmt::Debug for CommitCreateOptions<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CommitCreateOptions")
            .field("allow_empty", &self.allow_empty)
            .field("author", &self.author)
            .field("committer", &self.committer)
            .field("encoding", &self.encoding.map(String::from_utf8_lossy))
            .field("headers", &self.headers.len())
            .field("signed", &self.signer.is_some())
            .finish()
    }
}

impl<'a> CommitCreateOptions<'a> {
    /// The defaults.
    pub fn new() -> CommitCreateOptions<'a> {
        CommitCreateOptions::default()
    }

    /// Make the commit even if its tree is `HEAD`'s (`allow_empty_commit`;
    /// creation only).
    pub fn allow_empty(mut self, on: bool) -> Self {
        self.allow_empty = on;
        self
    }

    /// The author (default: the repository's signature; for an amend, the
    /// amended commit's author).
    pub fn author(mut self, sig: impl Into<SignatureRef<'a>>) -> Self {
        self.author = Some(sig.into());
        self
    }

    /// The committer (default: the repository's signature).
    pub fn committer(mut self, sig: impl Into<SignatureRef<'a>>) -> Self {
        self.committer = Some(sig.into());
        self
    }

    /// The message's `encoding` header.
    pub fn encoding(mut self, enc: &'a [u8]) -> Self {
        self.encoding = Some(enc);
        self
    }

    /// An extra header (a value with newlines becomes continuation lines).
    pub fn header(mut self, field: &'a [u8], value: &'a [u8]) -> Self {
        self.headers.push((field, value));
        self
    }

    /// Sign the commit: `f` gets its bytes and returns the value of the
    /// header `field` (e.g. `gpgsig`), or `None` to leave it unsigned; an
    /// error fails the operation with it.
    pub fn signer(
        mut self,
        field: &'a [u8],
        f: impl FnMut(&[u8]) -> Result<Option<Vec<u8>>, Error> + 'a,
    ) -> Self {
        self.signer = Some((field, Box::new(f)));
        self
    }

    /// Run `op` with the C options (valid during the call); its status and
    /// id as the result. A panic of the signer is resumed after `op`.
    fn run(
        mut self,
        op: impl FnOnce(*const GitCommitCreateOptions) -> (c_int, Oid),
    ) -> Result<Oid, Error> {
        let author = CSig::opt(self.author.as_ref())?;
        let committer = CSig::opt(self.committer.as_ref())?;
        let encoding = opt_cstring(self.encoding, "message encoding")?;
        let mut strings = Vec::with_capacity(self.headers.len());
        for (f, v) in &self.headers {
            strings.push((cstring(f, "header field")?, cstring(v, "header value")?));
        }
        let headers: Vec<GitCommitHeader> = strings
            .iter()
            .map(|(f, v)| GitCommitHeader {
                field: f.as_ptr(),
                value: v.as_ptr(),
            })
            .collect();
        let field = match &self.signer {
            Some((f, _)) => Some(cstring(f, "signature field")?),
            None => None,
        };
        let mut o = GitCommitCreateOptions::INIT;
        o.flags
            .set_flag(super::ALLOW_EMPTY_COMMIT, self.allow_empty);
        o.author = CSig::ptr(&author);
        o.committer = CSig::ptr(&committer);
        o.message_encoding = opt_ptr(&encoding);
        if !headers.is_empty() {
            o.extra_headers = headers.as_ptr();
            o.extra_headers_len = headers.len();
        }
        let mut signing = match (&mut self.signer, &field) {
            (Some((_, f)), Some(field)) => Some(Signing {
                field,
                f: &mut **f,
                panic: None,
            }),
            _ => None,
        };
        if let Some(s) = signing.as_mut() {
            o.sign = Some(sign_cb);
            o.payload = ptr::from_mut(s).cast();
        }
        let (e, id) = op(&raw const o);
        if let Some(s) = signing {
            s.finish();
        }
        check(e)?;
        Ok(id)
    }
}

/// What [`Commit::amend`] replaces (`git_commit_amend`'s arguments); by
/// default nothing (the old author, committer, message, encoding and tree
/// are kept) and no reference is updated.
#[derive(Clone, Copy, Debug, Default)]
pub struct AmendOptions<'a> {
    update_ref: Option<&'a [u8]>,
    author: Option<SignatureRef<'a>>,
    committer: Option<SignatureRef<'a>>,
    encoding: Option<&'a [u8]>,
    message: Option<&'a [u8]>,
    tree: Option<&'a Tree<'a>>,
}

impl<'a> AmendOptions<'a> {
    /// Nothing replaced.
    pub fn new() -> AmendOptions<'a> {
        AmendOptions::default()
    }

    /// The reference to move to the new commit (it must point to the
    /// amended one).
    pub fn update_ref(mut self, name: &'a [u8]) -> Self {
        self.update_ref = Some(name);
        self
    }

    /// The new author.
    pub fn author(mut self, sig: impl Into<SignatureRef<'a>>) -> Self {
        self.author = Some(sig.into());
        self
    }

    /// The new committer.
    pub fn committer(mut self, sig: impl Into<SignatureRef<'a>>) -> Self {
        self.committer = Some(sig.into());
        self
    }

    /// The new message encoding.
    pub fn encoding(mut self, enc: &'a [u8]) -> Self {
        self.encoding = Some(enc);
        self
    }

    /// The new message.
    pub fn message(mut self, msg: &'a [u8]) -> Self {
        self.message = Some(msg);
        self
    }

    /// The new tree (of the commit's repository).
    pub fn tree(mut self, tree: &'a Tree<'a>) -> Self {
        self.tree = Some(tree);
        self
    }
}

/// Builds and writes a commit of a tree and parents given by id
/// (`git_commit_create`, `git_commit_create_buffer`,
/// `git_commit_create_ext`'s headers and signing).
#[derive(Clone, Debug)]
pub struct CommitBuilder<'a> {
    tree: Oid,
    author: SignatureRef<'a>,
    committer: SignatureRef<'a>,
    message: &'a [u8],
    parents: Vec<Oid>,
    encoding: Option<&'a [u8]>,
    headers: Vec<(&'a [u8], &'a [u8])>,
    update_ref: Option<&'a [u8]>,
    validate: bool,
}

impl<'a> CommitBuilder<'a> {
    /// A commit of `tree` with no parents.
    pub fn new(
        tree: Oid,
        author: impl Into<SignatureRef<'a>>,
        committer: impl Into<SignatureRef<'a>>,
        message: &'a [u8],
    ) -> CommitBuilder<'a> {
        CommitBuilder {
            tree,
            author: author.into(),
            committer: committer.into(),
            message,
            parents: Vec::new(),
            encoding: None,
            headers: Vec::new(),
            update_ref: None,
            validate: true,
        }
    }

    /// Add a parent (in order).
    pub fn parent(mut self, id: Oid) -> Self {
        self.parents.push(id);
        self
    }

    /// The message's `encoding` header.
    pub fn encoding(mut self, enc: &'a [u8]) -> Self {
        self.encoding = Some(enc);
        self
    }

    /// An extra header (a value with newlines becomes continuation lines).
    pub fn header(mut self, field: &'a [u8], value: &'a [u8]) -> Self {
        self.headers.push((field, value));
        self
    }

    /// The reference to update to the new commit (whose current target must
    /// be the first parent).
    pub fn update_ref(mut self, name: &'a [u8]) -> Self {
        self.update_ref = Some(name);
        self
    }

    /// Whether [`write`](Self::write) checks that the tree and parents exist
    /// (when strict object validation is on); `true` by default.
    pub fn validate(mut self, v: bool) -> Self {
        self.validate = v;
        self
    }

    fn args<'s>(&'s self, update_ref: Option<&'s CStr>) -> CreateArgs<'s> {
        CreateArgs {
            author: Some(sig_out(&self.author)),
            committer: Some(sig_out(&self.committer)),
            message: Some(self.message),
            tree: self.tree,
            encoding: self.encoding,
            headers: Ok(self.headers.clone()),
            update_ref,
            validate: self.validate,
        }
    }

    /// The commit's bytes, as `git_commit_create_buffer` makes them (without
    /// checking the tree and parents).
    pub fn to_buffer(&self) -> Result<Vec<u8>, Error> {
        let mut out = GitStr::new();
        super::args_buffer(&mut out, &self.args(None), &self.parents).map_err(Error::from)?;
        Ok(out.as_bytes().to_vec())
    }

    /// Write the commit to `repo`'s object database (and update the
    /// reference, if one was given); its id.
    pub fn write(&self, repo: &Repository) -> Result<Oid, Error> {
        self.write_with(repo, None)
    }

    /// [`write`](Self::write), signed (`git_commit_create_ext` with a
    /// signing callback): `sign` gets the commit's bytes and returns the
    /// value of the header `field` (e.g. `gpgsig`), or `None` to leave it
    /// unsigned; an error fails the write with it.
    pub fn write_signed(
        &self,
        repo: &Repository,
        field: impl AsRef<[u8]>,
        mut sign: impl FnMut(&[u8]) -> Result<Option<Vec<u8>>, Error>,
    ) -> Result<Oid, Error> {
        let field = cstring(field.as_ref(), "signature field")?;
        let mut s = Signing {
            field: &field,
            f: &mut sign,
            panic: None,
        };
        let r = self.write_with(repo, Some(&mut s));
        s.finish();
        r
    }

    fn write_with(
        &self,
        repo: &Repository,
        signing: Option<&mut Signing<'_, '_>>,
    ) -> Result<Oid, Error> {
        let update_ref = opt_cstring(self.update_ref, "reference name")?;
        let args = self.args(update_ref.as_deref());
        let parents = &self.parents;
        let mut next = |i: usize| parents.get(i).copied();
        let mut sign_c;
        let sign: Option<super::Signer<'_>> = match signing {
            Some(s) => {
                sign_c = move |buf: &mut GitStr| s.sign(buf);
                Some(&mut sign_c)
            }
            None => None,
        };
        let mut id = Oid::default();
        // SAFETY: a live repository; the signer is Rust's.
        unsafe { super::create_internal(&mut id, repo.raw_ptr(), &args, &mut next, sign) }
            .map_err(Error::from)?;
        Ok(id)
    }
}
