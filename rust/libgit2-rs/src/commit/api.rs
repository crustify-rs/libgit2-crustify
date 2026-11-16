//! The safe API of commits (PORTING.md §5): [`Commit`], its signatures
//! ([`SignatureRef`](crate::SignatureRef)), and [`CommitBuilder`] to create commits.
//!
//! A [`Commit`] holds one reference to a libgit2 commit object (from the
//! repository's cache, or parsed from bytes), released on drop; `Clone`
//! takes another. Its accessors read the fields the parse set, which
//! nothing writes afterwards; the lazily computed summary and body are
//! computed and kept in the Rust value, never written into the shared
//! object.

use core::cell::OnceCell;
use core::ffi::CStr;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};
use std::ffi::CString;

use super::{CreateArgs, GitCommit, GitSignature, SigOut};
use crate::commit::deps as pend;
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

/// A commit of a repository (`git_commit`).
pub struct Commit<'r> {
    raw: NonNull<GitCommit>,
    summary: OnceCell<Vec<u8>>,
    body: OnceCell<Option<Vec<u8>>>,
    _repo: PhantomData<&'r Repository>,
}

fn invalid(msg: &str) -> Error {
    Error::with(ErrorCode::INVALID, ErrorClass::INVALID, msg)
}

fn cstring(s: &[u8], what: &str) -> Result<CString, Error> {
    CString::new(s).map_err(|_| invalid(&format!("{what} contains a NUL byte")))
}

/// The bytes of a C string the commit owns.
///
/// # Safety
///
/// `p` is NULL or a NUL-terminated string that lives (unwritten) for `'a`.
unsafe fn cbytes<'a>(p: *const core::ffi::c_char) -> &'a [u8] {
    // SAFETY: the caller's contract.
    unsafe { super::opt_cbytes(p) }.unwrap_or(b"")
}

impl<'r> Commit<'r> {
    /// `git_commit_lookup`.
    pub fn lookup(repo: &'r Repository, id: &Oid) -> Result<Commit<'r>, Error> {
        let mut raw = ptr::null_mut();
        // SAFETY: `raw` writable; the repository live for `'r`.
        let e = unsafe { pend::git_commit_lookup(&raw mut raw, repo.raw_ptr(), id) };
        if e < 0 {
            return Err(Error::from(super::ec(e)));
        }
        // SAFETY: a commit the lookup returned, ours to release.
        Ok(unsafe { Commit::from_raw(raw) })
    }

    /// Take over one reference to `raw`.
    ///
    /// # Safety
    ///
    /// `raw` is a non-NULL parsed commit, whose reference the caller hands
    /// over, and whose repository (if any) outlives `'r`.
    unsafe fn from_raw(raw: *mut GitCommit) -> Commit<'r> {
        Commit {
            // SAFETY: non-NULL (contract).
            raw: unsafe { NonNull::new_unchecked(raw) },
            summary: OnceCell::new(),
            body: OnceCell::new(),
            _repo: PhantomData,
        }
    }

    fn c(&self) -> *const GitCommit {
        self.raw.as_ptr()
    }

    /// The commit's id.
    pub fn id(&self) -> Oid {
        // SAFETY: a live commit.
        unsafe { super::object_id(self.c()) }.unwrap_or_default()
    }

    /// The id of its tree.
    pub fn tree_id(&self) -> Oid {
        // SAFETY: `self.c()` is the live commit this value holds.
        let p = unsafe { &raw const (*self.c()).tree_id };
        // SAFETY: a field set by the parse, never written after.
        unsafe { p.read() }
    }

    /// The ids of its parents.
    pub fn parent_ids(&self) -> &[Oid] {
        // SAFETY: a live commit, its parents unchanged while `self` lives.
        unsafe { super::parent_ids_of(self.c()) }
    }

    /// The number of parents.
    pub fn parent_count(&self) -> usize {
        self.parent_ids().len()
    }

    fn sig(&self, p: *const GitSignature) -> SignatureRef<'_> {
        if p.is_null() {
            return SignatureRef::new(b"", b"", Time::from_raw(Default::default()));
        }
        // SAFETY: a signature the commit owns, unchanged while it lives;
        // its strings are NUL-terminated, owned by the commit and unwritten
        // while it lives (the result borrows `self`).
        unsafe { SignatureRef::from_raw(&*p) }
    }

    /// The author.
    pub fn author(&self) -> SignatureRef<'_> {
        // SAFETY: `self.c()` is the live commit this value holds.
        let p = unsafe { &raw const (*self.c()).author };
        // SAFETY: a field set by the parse, never written after.
        self.sig(unsafe { p.read() })
    }

    /// The committer.
    pub fn committer(&self) -> SignatureRef<'_> {
        // SAFETY: `self.c()` is the live commit this value holds.
        let p = unsafe { &raw const (*self.c()).committer };
        // SAFETY: a field set by the parse, never written after.
        self.sig(unsafe { p.read() })
    }

    /// `git_commit_time`.
    pub fn time(&self) -> i64 {
        self.committer().when().seconds()
    }

    /// `git_commit_time_offset`.
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
    pub fn header_field(&self, field: &[u8]) -> Result<Vec<u8>, Error> {
        let mut out = GitStr::new();
        super::header_field(self.raw_header(), field, &mut out).map_err(Error::from)?;
        Ok(out.as_bytes().to_vec())
    }

    fn repo(&self) -> *mut pend::git_repository {
        // SAFETY: a live commit.
        unsafe { super::object_owner(self.c()) }.unwrap_or(ptr::null_mut())
    }

    /// A parsed commit has no repository to look objects up in (the C would
    /// crash).
    fn need_repo(&self) -> Result<(), Error> {
        if self.repo().is_null() {
            return Err(Error::from(crate::util::errors::assert_arg_failed("repo")));
        }
        Ok(())
    }

    /// `git_commit_tree`.
    pub fn tree(&self) -> Result<Tree<'r>, Error> {
        // SAFETY: the commit's repository outlives `'r`.
        unsafe { Tree::lookup_raw(self.repo(), &self.tree_id()) }
    }

    /// `git_commit_parent`: the `n`th parent.
    pub fn parent(&self, n: usize) -> Result<Commit<'r>, Error> {
        self.need_repo()?;
        let mut raw = ptr::null_mut();
        let n = u32::try_from(n).unwrap_or(u32::MAX);
        // SAFETY: `raw` writable; a live commit.
        let e = unsafe { super::ffi::git_commit_parent(&raw mut raw, self.c(), n) };
        if e < 0 {
            return Err(Error::from(super::ec(e)));
        }
        // SAFETY: a commit of the same repository, ours to release.
        Ok(unsafe { Commit::from_raw(raw) })
    }

    /// `git_commit_nth_gen_ancestor`: the `n`th first-parent ancestor (the
    /// commit itself for 0).
    pub fn nth_gen_ancestor(&self, n: u32) -> Result<Commit<'r>, Error> {
        if n > 0 {
            self.need_repo()?;
        }
        let mut raw = ptr::null_mut();
        // SAFETY: `raw` writable; a live commit.
        let e = unsafe { super::ffi::git_commit_nth_gen_ancestor(&raw mut raw, self.c(), n) };
        if e < 0 {
            return Err(Error::from(super::ec(e)));
        }
        // SAFETY: as in `parent`.
        Ok(unsafe { Commit::from_raw(raw) })
    }
}

impl Commit<'static> {
    /// Parse a commit from its bytes (`git_object__from_raw`), outside any
    /// repository (so without grafts, and [`tree`](Commit::tree) and
    /// [`parent`](Commit::parent) fail).
    pub fn parse(data: &[u8], oid_type: OidType) -> Result<Commit<'static>, Error> {
        let mut raw = ptr::null_mut();
        // SAFETY: `raw` writable; `data` readable. The commit copies what it
        // keeps, so it does not borrow `data`.
        let e = unsafe {
            pend::git_object__from_raw(
                &raw mut raw,
                data.as_ptr().cast(),
                data.len(),
                super::OBJ_COMMIT,
                oid_type.raw(),
            )
        };
        if e < 0 {
            return Err(Error::from(super::ec(e)));
        }
        // SAFETY: a commit just made, ours to release.
        Ok(unsafe { Commit::from_raw(raw.cast()) })
    }
}

impl Clone for Commit<'_> {
    fn clone(&self) -> Self {
        let mut raw = ptr::null_mut();
        // SAFETY: a live object; `git_object_dup` only counts a reference
        // (it cannot fail).
        unsafe { pend::git_object_dup(&raw mut raw, self.raw.as_ptr().cast()) };
        // SAFETY: the reference just taken.
        unsafe { Commit::from_raw(raw.cast()) }
    }
}

impl Drop for Commit<'_> {
    fn drop(&mut self) {
        // SAFETY: the reference this value holds, released once.
        unsafe { pend::git_object_free(self.raw.as_ptr().cast()) }
    }
}

impl core::fmt::Debug for Commit<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Commit").field("id", &self.id()).finish()
    }
}

/// Builds and writes a commit (`git_commit_create*`).
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
        let update_ref = self
            .update_ref
            .map(|r| cstring(r, "reference name"))
            .transpose()?;
        let args = self.args(update_ref.as_deref());
        let parents = &self.parents;
        let mut next = |i: usize| parents.get(i).copied();
        let mut id = Oid::default();
        // SAFETY: a live repository; no C callback is involved.
        unsafe { super::create_internal(&mut id, repo.raw_ptr(), &args, &mut next, None) }
            .map_err(Error::from)?;
        Ok(id)
    }
}
