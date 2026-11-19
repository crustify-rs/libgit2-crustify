//! `src/libgit2/commit.c`: commit objects: parsing, accessors, the summary
//! and body, header fields, signatures, and commit creation.
//!
//! [`GitCommit`] is a *mirrored* type (PORTING.md §4.1): `object.c`
//! allocates it with `sizeof(git_commit)` (`git_objects_table`), the clar
//! tests allocate one and read its fields, and `indexer.c` reads
//! `parent_ids` and `tree_id`. Its strings and signatures are allocated with
//! `git__allocator` and freed by [`ffi::git_commit__free`].
//!
//! A commit in the object cache is shared: every lookup of the same id, on
//! any thread, returns the same pointer, and the cache writes its refcount
//! concurrently. So the exports never form a Rust reference to a shared
//! commit: they read its fields through the raw pointer (the fields are
//! written only by the parse, before the commit is published). The two
//! lazily computed fields (`summary`, `body`) are written by
//! `git_commit_summary`/`git_commit_body` exactly as the C does, without
//! synchronisation: concurrent first calls race in the C as well. The safe
//! API ([`Commit`]) never writes them; it computes and keeps its own copy.
//!
//! The parser ([`parse`]) is pure Rust over the object's bytes, generic over
//! a [`CommitSink`] that receives the fields: the C ABI's sink allocates them
//! into a [`GitCommit`] as the C did; `commit_list.c`'s quick parse keeps only
//! the parents and the commit time, without allocating.

use crate::util::cabi::CStrArg;
use crate::util::cabi::Out;
use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::marker::PhantomData;
use core::ptr::{self, NonNull};

use crate::oid::{Oid, OidType, git_oid_t};
pub(crate) mod deps;

use crate::util::alloc;
use crate::util::array::GitArray;
use crate::util::ctype_compat::isspace;
use crate::util::errors::{self, ErrorClass, ErrorCode, Result};
use crate::util::str::GitStr;
use crate::util::util::isspace_nonlf;
use crate::{git_assert_arg, git_error_set};
use deps::{self as pend, GitCommitGraft, GitObjectHead, git_repository};

pub mod api;

/// `GIT_OBJECT_COMMIT`.
pub(crate) const OBJ_COMMIT: c_int = 1;
/// `GIT_OBJECT_TREE`.
pub(crate) const OBJ_TREE: c_int = 2;
/// `GIT_OBJECT_BLOB`.
pub(crate) const OBJ_BLOB: c_int = 3;
/// `GIT_OBJECT_ANY`.
pub(crate) const OBJ_ANY: c_int = -2;
/// `GIT_OBJECT_INVALID`.
pub(crate) const OBJ_INVALID: c_int = -1;

/// `GIT_COMMIT_PARSE_QUICK`: only the parents and the committer.
pub const GIT_COMMIT_PARSE_QUICK: c_uint = 1 << 0;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// `git_time` and `git_signature` (`include/git2/types.h`, public): the
/// signature module's mirrors. The commit allocates and frees its own
/// signatures (as `git_signature_free` does: `name`, `email`, then the
/// struct).
pub use crate::signature::{GitSignature, GitTime};

/// `struct git_commit` (`commit.h`).
#[repr(C)]
pub struct GitCommit {
    pub object: GitObjectHead,
    pub parent_ids: GitArray<Oid>,
    pub tree_id: Oid,
    pub author: *mut GitSignature,
    pub committer: *mut GitSignature,
    pub message_encoding: *mut c_char,
    pub raw_message: *mut c_char,
    pub raw_header: *mut c_char,
    pub summary: *mut c_char,
    pub body: *mut c_char,
}

/// A live commit, as C hands one to the exports (and the safe API holds
/// one): the one place where the C's guarantees about a commit pointer
/// become a type.
///
/// A commit in the object cache is shared between threads, and C's
/// `git_commit_summary` / `git_commit_body` write its `summary` / `body`
/// lazily, without synchronisation. So no reference to the whole
/// `GitCommit` is ever formed: the accessors below read one field at a
/// time through the pointer, and borrow only fields that nothing writes.
///
/// Invariant (from [`CommitRef::from_ptr`], the only constructor): for
/// `'a`, `ptr` is a live, initialized commit that is not freed; nothing
/// writes its fields other than `summary` and `body` (the parse sets them
/// before the commit is published; `object.cached.refcount` is atomic),
/// and those two are accessed only through raw pointers; `raw_message`,
/// `raw_header` and `message_encoding` are NULL or NUL-terminated strings,
/// `author` and `committer` NULL or signatures with NUL-terminated strings,
/// and `object.repo` NULL or a live repository, all live and unwritten for
/// `'a`.
///
/// It has the ABI of `const git_commit *` (`Option<CommitRef>` that of a
/// nullable one), so exports take it as their commit parameter.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommitRef<'a> {
    ptr: NonNull<GitCommit>,
    _life: PhantomData<&'a GitCommit>,
}

impl<'a> CommitRef<'a> {
    /// The commit at `ptr`.
    ///
    /// # Safety
    ///
    /// `ptr` and `'a` satisfy the type's invariant: a live commit (of the
    /// object table, or parsed by this module) that is not freed during
    /// `'a`, whose fields are written by nothing during `'a` except
    /// `summary` and `body` through raw pointers (`git_commit_summary`,
    /// `git_commit_body`).
    #[inline(always)]
    pub const unsafe fn from_ptr(ptr: NonNull<GitCommit>) -> CommitRef<'a> {
        CommitRef {
            ptr,
            _life: PhantomData,
        }
    }

    /// The commit at `p`, or `None` for NULL.
    ///
    /// # Safety
    ///
    /// As [`from_ptr`](Self::from_ptr), for a non-NULL `p`.
    #[inline(always)]
    pub unsafe fn from_raw(p: *const GitCommit) -> Option<CommitRef<'a>> {
        // SAFETY: the caller's guarantee, for a non-NULL `p`.
        NonNull::new(p.cast_mut()).map(|p| unsafe { CommitRef::from_ptr(p) })
    }

    /// The pointer.
    #[inline(always)]
    pub const fn as_ptr(self) -> *mut GitCommit {
        self.ptr.as_ptr()
    }

    /// `git_object_owner`: the owning repository (NULL for a commit parsed
    /// from raw data).
    #[inline(always)]
    pub fn owner(self) -> *mut git_repository {
        // SAFETY: a live commit (the invariant); `object.repo` is not
        // written during `'a`: read in place, no reference.
        unsafe { (*self.ptr.as_ptr()).object.repo }
    }

    /// The owning repository, as a reference.
    #[inline(always)]
    pub fn repository(self) -> Option<&'a git_repository> {
        // SAFETY: NULL or a live repository for `'a` (the invariant), only
        // read through `&` (its state is interior-mutable).
        unsafe { self.owner().as_ref() }
    }

    /// `git_object_id`.
    #[inline(always)]
    pub fn id(self) -> &'a Oid {
        // SAFETY: a field of a live commit (the invariant) that nothing
        // writes during `'a`: the reference covers that field only.
        unsafe { &(*self.ptr.as_ptr()).object.cached.oid }
    }

    /// `tree_id`.
    #[inline(always)]
    pub fn tree_id(self) -> &'a Oid {
        // SAFETY: as in `id`.
        unsafe { &(*self.ptr.as_ptr()).tree_id }
    }

    /// The parents' ids (`parent_ids`).
    #[inline(always)]
    pub fn parent_ids(self) -> &'a [Oid] {
        // SAFETY: as in `id`: the array (and its buffer, which it owns) is
        // set by the parse and not written during `'a`.
        unsafe { &(*self.ptr.as_ptr()).parent_ids }.as_slice()
    }

    /// `author` (NULL for a commit without one).
    #[inline(always)]
    pub fn author(self) -> *const GitSignature {
        // SAFETY: as in `owner`.
        unsafe { (*self.ptr.as_ptr()).author }
    }

    /// `committer` (NULL for a commit that was not parsed).
    #[inline(always)]
    pub fn committer(self) -> *const GitSignature {
        // SAFETY: as in `owner`.
        unsafe { (*self.ptr.as_ptr()).committer }
    }

    /// `author`, as a reference.
    #[inline(always)]
    pub fn author_ref(self) -> Option<&'a GitSignature> {
        // SAFETY: NULL or the commit's own signature, live and unwritten for
        // `'a` (the invariant).
        unsafe { self.author().as_ref() }
    }

    /// `committer`, as a reference.
    #[inline(always)]
    pub fn committer_ref(self) -> Option<&'a GitSignature> {
        // SAFETY: as in `author_ref`.
        unsafe { self.committer().as_ref() }
    }

    /// The author as the safe API's view (empty for NULL).
    pub fn author_view(self) -> crate::signature::SignatureRef<'a> {
        // SAFETY: the commit's own author (the type's invariant).
        unsafe { Self::view(self.author_ref()) }
    }

    /// The committer as the safe API's view (empty for NULL).
    pub fn committer_view(self) -> crate::signature::SignatureRef<'a> {
        // SAFETY: the commit's own committer (the type's invariant).
        unsafe { Self::view(self.committer_ref()) }
    }

    /// The view of `sig`.
    ///
    /// # Safety
    ///
    /// `sig` is NULL or a signature whose `name` and `email` are NULL or
    /// NUL-terminated strings, live and unwritten for `'a`.
    unsafe fn view(sig: Option<&'a GitSignature>) -> crate::signature::SignatureRef<'a> {
        match sig {
            None => crate::signature::SignatureRef::new(
                b"",
                b"",
                crate::signature::Time::from_raw(GitTime::default()),
            ),
            // SAFETY: its strings are NULL or NUL-terminated, live and
            // unwritten for `'a` (this fn's contract).
            Some(s) => unsafe { crate::signature::SignatureRef::from_raw(s) },
        }
    }

    /// `raw_message`.
    #[inline(always)]
    pub fn raw_message(self) -> *const c_char {
        // SAFETY: as in `owner`.
        unsafe { (*self.ptr.as_ptr()).raw_message }
    }

    /// `message_encoding`.
    #[inline(always)]
    pub fn message_encoding(self) -> *const c_char {
        // SAFETY: as in `owner`.
        unsafe { (*self.ptr.as_ptr()).message_encoding }
    }

    /// `raw_header`.
    #[inline(always)]
    pub fn raw_header(self) -> *const c_char {
        // SAFETY: as in `owner`.
        unsafe { (*self.ptr.as_ptr()).raw_header }
    }

    /// `raw_message`, as a string argument (no `strlen`).
    #[inline(always)]
    pub fn raw_message_arg(self) -> CStrArg<'a> {
        // SAFETY: one of the commit's strings: NULL or NUL-terminated, live
        // and unwritten for `'a` (the invariant).
        unsafe { CStrArg::from_ptr(self.raw_message()) }
    }

    /// `message_encoding`, as a string argument.
    #[inline(always)]
    pub fn message_encoding_arg(self) -> CStrArg<'a> {
        // SAFETY: as in `raw_message_arg`.
        unsafe { CStrArg::from_ptr(self.message_encoding()) }
    }

    /// `raw_header`, as a string argument.
    #[inline(always)]
    pub fn raw_header_arg(self) -> CStrArg<'a> {
        // SAFETY: as in `raw_message_arg`.
        unsafe { CStrArg::from_ptr(self.raw_header()) }
    }

    /// `git_commit_message`'s pointer: into the raw message, past its
    /// leading newlines (NULL without one).
    pub fn message_ptr(self) -> *const c_char {
        let mut m = self.raw_message();
        if m.is_null() {
            return m;
        }
        // The raw message is NUL-terminated: the loop stops at its NUL at
        // the latest, so every byte read is within it.
        // SAFETY: `m` is within the commit's NUL-terminated message, live
        // and unwritten for `'a` (the invariant), at or before its NUL.
        while unsafe { m.read() } == b'\n' as c_char {
            // `m` is a newline, not the NUL: `m + 1` is in the string.
            m = m.wrapping_add(1);
        }
        m
    }

    /// [`message_ptr`](Self::message_ptr) as a string argument.
    #[inline(always)]
    pub fn message_arg(self) -> CStrArg<'a> {
        // SAFETY: NULL or a position within the commit's raw message, at or
        // before its NUL (`message_ptr`), so a NUL-terminated string, live
        // and unwritten for `'a` (the invariant).
        unsafe { CStrArg::from_ptr(self.message_ptr()) }
    }

    /// `&commit->summary`, for `git_commit_summary` (no reference formed).
    #[inline(always)]
    fn summary_slot(self) -> *mut *mut c_char {
        // SAFETY: a field projection of a live commit (no reference).
        unsafe { &raw mut (*self.ptr.as_ptr()).summary }
    }

    /// `&commit->body`, for `git_commit_body` (no reference formed).
    #[inline(always)]
    fn body_slot(self) -> *mut *mut c_char {
        // SAFETY: as in `summary_slot`.
        unsafe { &raw mut (*self.ptr.as_ptr()).body }
    }
}

/// `git_commit__parse_options` (`commit.h`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GitCommitParseOptions {
    pub oid_type: git_oid_t,
    pub flags: c_uint,
}

/// `git_commit_header` (`include/git2/commit.h`, public).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GitCommitHeader {
    pub field: *const c_char,
    pub value: *const c_char,
}

/// `git_commit_signature_cb`.
pub type GitCommitSignatureCb = Option<
    unsafe extern "C" fn(
        builder: *mut GitCommitbuilder,
        repo: *mut git_repository,
        commit_content: *const c_char,
        payload: *mut c_void,
    ) -> c_int,
>;

/// `git_commit_create_ext_options` (public).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GitCommitCreateExtOptions {
    pub version: c_uint,
    pub update_ref: *const c_char,
    pub message_encoding: *const c_char,
    pub extra_headers: *const GitCommitHeader,
    pub extra_headers_len: usize,
    pub sign: GitCommitSignatureCb,
    pub payload: *mut c_void,
}

impl GitCommitCreateExtOptions {
    /// `GIT_COMMIT_CREATE_EXT_OPTIONS_INIT`.
    pub const INIT: GitCommitCreateExtOptions = GitCommitCreateExtOptions {
        version: 1,
        update_ref: ptr::null(),
        message_encoding: ptr::null(),
        extra_headers: ptr::null(),
        extra_headers_len: 0,
        sign: None,
        payload: ptr::null_mut(),
    };
}

/// `git_commit_create_options` (public). `flags` is the bitfield
/// `unsigned allow_empty_commit : 1` ([`ALLOW_EMPTY_COMMIT`]).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GitCommitCreateOptions {
    pub version: c_uint,
    pub flags: ffibox::CBitfield<c_uint>,
    pub author: *const GitSignature,
    pub committer: *const GitSignature,
    pub message_encoding: *const c_char,
    pub extra_headers: *const GitCommitHeader,
    pub extra_headers_len: usize,
    pub sign: GitCommitSignatureCb,
    pub payload: *mut c_void,
}

impl GitCommitCreateOptions {
    /// `GIT_COMMIT_CREATE_OPTIONS_INIT`.
    pub const INIT: GitCommitCreateOptions = GitCommitCreateOptions {
        version: 1,
        flags: ffibox::CBitfield::new(0),
        author: ptr::null(),
        committer: ptr::null(),
        message_encoding: ptr::null(),
        extra_headers: ptr::null(),
        extra_headers_len: 0,
        sign: None,
        payload: ptr::null_mut(),
    };

    fn allow_empty_commit(&self) -> bool {
        self.flags.is_set(ALLOW_EMPTY_COMMIT)
    }
}

/// `unsigned allow_empty_commit : 1` of [`GitCommitCreateOptions::flags`].
pub const ALLOW_EMPTY_COMMIT: ffibox::CBits<c_uint> = ffibox::CBits::first(1);

/// `struct git_commitbuilder` (opaque to C: only `commit.c` reads it).
#[repr(C)]
pub struct GitCommitbuilder {
    contents: *mut GitStr,
}

// ---------------------------------------------------------------------------
// Signatures (the signature module's parser and writer)
// ---------------------------------------------------------------------------

pub use crate::signature::SigFields;

/// `git_signature__parse(sig, &buffer, buffer_end, header, '\n')` over
/// `buf[*pos..]`: on success `*pos` is past the line's newline.
#[inline]
pub fn parse_signature<'a>(buf: &'a [u8], pos: &mut usize, header: &[u8]) -> Result<SigFields<'a>> {
    let (f, used) = crate::signature::parse_fields(&buf[*pos..], Some(header), b'\n')?;
    *pos += used;
    Ok(f)
}

/// A `git_signature` allocated as `git_signature__parse` fills one.
pub(crate) fn alloc_signature(sig: &SigFields<'_>) -> Result<*mut GitSignature> {
    crate::signature::alloc_raw(sig)
}

/// `git_signature_free` of a signature allocated by [`alloc_signature`].
///
/// # Safety
///
/// `sig` is NULL or a signature whose strings and struct come from
/// `git__allocator` and that nothing else frees or uses after.
pub(crate) unsafe fn free_signature(sig: *mut GitSignature) {
    // SAFETY: NULL or a `git__allocator` signature with `git__allocator`
    // strings that nothing uses after (the contract): `free_raw`'s.
    unsafe { crate::signature::free_raw(sig) }
}

/// `git_signature__writebuf(buf, header, sig)`; a NULL name or e-mail is
/// printf's `(null)`.
pub(crate) fn write_signature(
    buf: &mut GitStr,
    header: &[u8],
    name: Option<&[u8]>,
    email: Option<&[u8]>,
    when: GitTime,
) {
    // printf's "%s" of NULL is glibc's "(null)".
    let name = name.unwrap_or(b"(null)");
    let email = email.unwrap_or(b"(null)");
    crate::signature::writebuf(buf, Some(header), name, email, when);
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Receives the fields of a commit as [`parse`] finds them.
pub trait CommitSink<'a> {
    /// The tree (not called by a quick parse).
    fn tree(&mut self, id: Oid);
    /// The next parent.
    fn parent(&mut self, id: Oid) -> Result<()>;
    /// The (first) author (not called by a quick parse).
    fn author(&mut self, sig: &SigFields<'a>) -> Result<()>;
    /// The committer.
    fn committer(&mut self, sig: &SigFields<'a>) -> Result<()>;
    /// An `encoding` header's value, to the end of its line.
    fn encoding(&mut self, value: &'a [u8]) -> Result<()>;
    /// The header (up to the blank line) and the message (after it; `None`
    /// when there is none).
    fn header_and_message(&mut self, header: &'a [u8], message: Option<&'a [u8]>) -> Result<()>;
}

fn bad_buffer() -> ErrorCode {
    let _ = git_error_set!(ErrorClass::OBJECT, "failed to parse bad commit object");
    ErrorCode::INVALID
}

/// `git_object__parse_oid_header` over `buf[*pos..]` (the object
/// module's): `<header><hex>\n`; `None` on a mismatch.
#[inline]
pub(crate) fn parse_oid_header(
    buf: &[u8],
    pos: &mut usize,
    header: &[u8],
    kind: Option<OidType>,
) -> Option<Oid> {
    let rest = &buf[*pos..];
    let Some(kind) = kind else {
        // An unknown type has hex size 0: C matches `<header>\n`, then
        // git_oid_from_prefix fails.
        if rest.len() > header.len() && rest.starts_with(header) && rest[header.len()] == b'\n' {
            let _ = git_error_set!(ErrorClass::INVALID, "unable to parse OID - too short");
        }
        return None;
    };
    let (id, used) = crate::object::parse_oid_header(rest, header, kind).ok()?;
    *pos += used;
    Some(id)
}

/// `commit_parse`: parse the commit in `data`, giving its fields to `sink`.
/// A quick parse (`GIT_COMMIT_PARSE_QUICK`) skips the tree, the author, the
/// extra headers and the message.
pub fn parse<'a, S: CommitSink<'a>>(
    data: &'a [u8],
    oid_type: git_oid_t,
    quick: bool,
    sink: &mut S,
) -> Result<()> {
    let kind = OidType::from_raw(oid_type);
    let end = data.len();
    let mut pos = 0usize;

    // The tree is always the first field.
    if !quick {
        match parse_oid_header(data, &mut pos, b"tree ", kind) {
            Some(id) => sink.tree(id),
            None => return Err(bad_buffer()),
        }
    } else {
        let tree_len = 5 + kind.map_or(0, OidType::hexsize) + 1;
        if tree_len > end {
            return Err(bad_buffer());
        }
        pos = tree_len;
    }

    while let Some(id) = parse_oid_header(data, &mut pos, b"parent ", kind) {
        sink.parent(id)?;
    }

    if !quick {
        let sig = parse_signature(data, &mut pos, b"author ")?;
        sink.author(&sig)?;
    }

    // Some tools create multiple author fields, ignore the extra ones.
    while data[pos..].starts_with(b"author ") {
        parse_signature(data, &mut pos, b"author ")?;
    }

    // Always parse the committer; we need the commit time.
    let sig = parse_signature(data, &mut pos, b"committer ")?;
    sink.committer(&sig)?;

    if quick {
        return Ok(());
    }

    // Parse add'l header entries. (`pos > 0` here: the committer's line
    // ends with a newline at `pos - 1`.)
    while pos < end {
        if data[pos - 1] == b'\n' && data[pos] == b'\n' {
            break;
        }
        let eoln = memchr::memchr(b'\n', &data[pos..]).map_or(end, |i| pos + i);
        if data[pos..].starts_with(b"encoding ") {
            sink.encoding(&data[pos + 9..eoln])?;
        }
        pos = if eoln < end { eoln + 1 } else { eoln };
    }

    let header = &data[..pos];
    // +1 for the final LF
    let message = if pos < end {
        Some(&data[pos + 1..])
    } else {
        None
    };
    sink.header_and_message(header, message)
}

/// The sink of the C ABI: fills a [`GitCommit`] as `commit_parse` did.
struct CommitFill<'c> {
    commit: &'c mut GitCommit,
}

impl<'a> CommitSink<'a> for CommitFill<'_> {
    fn tree(&mut self, id: Oid) {
        self.commit.tree_id = id;
    }
    #[inline]
    fn parent(&mut self, id: Oid) -> Result<()> {
        self.commit.parent_ids.push(id).map(|_| ())
    }
    fn author(&mut self, sig: &SigFields<'a>) -> Result<()> {
        let p = alloc_signature(sig)?;
        // SAFETY: the old value is NULL (a fresh commit) or one this sink set.
        unsafe { free_signature(self.commit.author) };
        self.commit.author = p;
        Ok(())
    }
    fn committer(&mut self, sig: &SigFields<'a>) -> Result<()> {
        let p = alloc_signature(sig)?;
        // SAFETY: as in `author`.
        unsafe { free_signature(self.commit.committer) };
        self.commit.committer = p;
        Ok(())
    }
    fn encoding(&mut self, value: &'a [u8]) -> Result<()> {
        let p = alloc::strndup(value, value.len());
        if p.is_null() {
            return Err(errors::oom());
        }
        // (The C leaks the previous value of a repeated header.)
        // SAFETY: NULL or this sink's own allocation.
        unsafe { alloc::free(self.commit.message_encoding.cast()) };
        self.commit.message_encoding = p;
        Ok(())
    }
    fn header_and_message(&mut self, header: &'a [u8], message: Option<&'a [u8]>) -> Result<()> {
        let h = alloc::strndup(header, header.len());
        if h.is_null() {
            return Err(errors::oom());
        }
        self.commit.raw_header = h;
        let m = alloc::strndup(message.unwrap_or(b""), message.map_or(0, <[u8]>::len));
        if m.is_null() {
            return Err(errors::oom());
        }
        self.commit.raw_message = m;
        Ok(())
    }
}

/// `commit_parse` into `commit`.
pub(crate) fn parse_into(
    commit: &mut GitCommit,
    data: &[u8],
    opts: &GitCommitParseOptions,
) -> Result<()> {
    // Allocate for one, which will allow not to realloc 90% of the time.
    commit.parent_ids = GitArray::with_capacity(1)?;
    parse(
        data,
        opts.oid_type,
        opts.flags & GIT_COMMIT_PARSE_QUICK != 0,
        &mut CommitFill { commit },
    )
}

/// `assign_commit_parents_from_graft`.
fn parents_from_graft(parents: &mut GitArray<Oid>, graft: &GitArray<Oid>) -> Result<()> {
    *parents = GitArray::with_capacity(graft.len())?;
    for id in graft.iter() {
        parents.push(*id)?;
    }
    Ok(())
}

/// The graft of `id` in the repository's grafts, then its shallow grafts
/// (`None` without a repository).
///
/// # Safety
///
/// Nothing changes the repository's grafts during the call, and the
/// result is used only while they are unchanged. (They are refreshed only
/// by `repository::open::load_grafts`, on a repository not yet returned,
/// and by `git_repository__shallow_roots*` and the grafts exports on
/// `git_repository_*grafts__weakptr`, whose contracts exclude concurrent
/// use.)
pub(crate) unsafe fn find_graft(
    repo: Option<&git_repository>,
    id: &Oid,
) -> Option<*mut GitCommitGraft> {
    let r = repo?;
    for grafts in [r.grafts_ptr(), r.shallow_grafts_ptr()] {
        let grafts = grafts.cast::<pend::git_grafts>();
        let mut g = ptr::null_mut();
        // SAFETY: `g` is a writable local; `grafts` is NULL (which the
        // export rejects: C asserts it is not) or the live repository's own
        // grafts, unchanged during the call (the contract); `id` readable.
        if unsafe { pend::git_grafts_get(crate::util::cabi::Out::from(&mut g), grafts, Some(id)) }
            == 0
        {
            return Some(g);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Summary, body, header fields
// ---------------------------------------------------------------------------

/// `git_commit_message` of a raw message: without its leading newlines.
pub fn message_of(raw: &[u8]) -> &[u8] {
    let n = raw.iter().take_while(|&&c| c == b'\n').count();
    &raw[n..]
}

/// `git_commit_summary`'s computation over the message (without its
/// leading newlines): the first paragraph, with each run of whitespace that
/// contains a newline collapsed to one space, and the trailing whitespace
/// dropped.
pub fn summary_into(msg: &[u8], out: &mut GitStr) -> Result<()> {
    let at = |i: usize| msg.get(i).copied().unwrap_or(0);
    let mut space: Option<usize> = None;
    let mut space_contains_newline = false;
    // The pending run of message bytes the summary copies unchanged.
    let mut run_start: Option<usize> = None;
    let mut run_end = 0usize;
    let mut i = 0;
    while i < msg.len() && msg[i] != 0 {
        let c = msg[i];
        // stop processing at the end of the first paragraph
        if c == b'\n' {
            if at(i + 1) == 0 || at(i + 1) == b'\n' {
                break;
            }
            // stop processing if next line contains only whitespace
            let mut next = i + 1;
            while at(next) != 0 && isspace_nonlf(at(next)) {
                next += 1;
            }
            if at(next) == 0 || at(next) == b'\n' {
                break;
            }
        }
        if isspace(c) {
            // record the beginning of contiguous whitespace runs
            if space.is_none() {
                space = Some(i);
                space_contains_newline = false;
            }
            space_contains_newline |= c == b'\n';
        } else {
            // process any recorded whitespace: a run with a newline becomes
            // one space; one without is copied, so it joins the pending run
            // of bytes copied as they are (put in one go, not per byte)
            if let Some(s) = space.take() {
                if space_contains_newline {
                    if let Some(rs) = run_start.take() {
                        out.put(&msg[rs..run_end])?;
                    }
                    out.putc(b' ')?;
                } else if run_start.is_none() {
                    run_start = Some(s);
                }
            }
            // copy the next character
            if run_start.is_none() {
                run_start = Some(i);
            }
            run_end = i + 1;
        }
        i += 1;
    }
    if let Some(rs) = run_start {
        out.put(&msg[rs..run_end])?;
    }
    Ok(())
}

/// [`summary_into`] as a `Vec`.
pub fn summary_of(msg: &[u8]) -> Vec<u8> {
    let mut s = GitStr::new();
    let _ = summary_into(msg, &mut s);
    s.as_bytes().to_vec()
}

/// `git_commit_body`'s computation over the message (without its leading
/// newlines): what follows the first paragraph, trimmed of whitespace at
/// both ends; `None` if that is empty.
pub fn body_of(msg: &[u8]) -> Option<&[u8]> {
    let msg = &msg[..memchr::memchr(0, msg).unwrap_or(msg.len())];
    // search for end of summary
    let mut i = 0;
    while i < msg.len() {
        if msg[i] == b'\n' && (i + 1 == msg.len() || msg[i + 1] == b'\n') {
            break;
        }
        i += 1;
    }
    // trim leading and trailing whitespace
    while i < msg.len() && isspace(msg[i]) {
        i += 1;
    }
    let rest = &msg[i..];
    let n = rest.iter().rposition(|&c| !isspace(c))?;
    Some(&rest[..=n])
}

/// `git_commit__header_field` over a raw header (as a C string: up to its
/// first NUL): the value of `field`, continuation lines joined with `\n`.
pub fn header_field(raw_header: &[u8], field: &[u8], out: &mut GitStr) -> Result<()> {
    let n = memchr::memchr(0, raw_header).unwrap_or(raw_header.len());
    header_field_in(&raw_header[..n], field, out)
}

/// `header_field` over a header that has no NUL (the bytes of a C string),
/// without the scan for one.
pub(crate) fn header_field_in(buf: &[u8], field: &[u8], out: &mut GitStr) -> Result<()> {
    out.clear();
    let at = |i: usize| buf.get(i).copied().unwrap_or(0);
    // An empty field matches no line: C's prefix matches, but then wants
    // the SP that only a (skipped) continuation line starts with.
    let Some(&first) = field.first() else {
        return Err(no_such_field(field));
    };
    let mut pos = 0usize;
    while let Some(eol) = memchr::memchr(b'\n', &buf[pos..]).map(|i| pos + i) {
        // We can skip continuations here (`first` cannot be SP on a line
        // that is not one), and skip until we find the field we're after
        // (git__prefixcmp: a NUL in the field ends it). Most lines already
        // differ in their first byte, which is compared on its own first.
        let line = &buf[pos..];
        if line[0] == b' ' || line[0] != first || !line.starts_with(field) {
            pos = eol + 1;
            continue;
        }
        let mut b = pos + field.len();
        // Check that we're not matching a prefix but the field itself
        if at(b) != b' ' {
            pos = eol + 1;
            continue;
        }
        b += 1; // skip the SP
        if b > eol {
            // A field containing a newline matched across lines: the C
            // copies `eol - buf` (negative) bytes, which fails as out of
            // memory (DEFECTS.md).
            return Err(errors::oom());
        }
        let mut eol = eol;
        out.put(&buf[b..eol])?;
        // If the next line starts with SP, it's multi-line, we must continue
        while at(eol + 1) == b' ' {
            out.putc(b'\n')?;
            let start = eol + 2;
            let Some(i) = memchr::memchr(b'\n', &buf[start.min(buf.len())..]) else {
                return Err(malformed());
            };
            eol = start + i;
            out.put(&buf[start..eol])?;
        }
        return Ok(());
    }
    Err(no_such_field(field))
}

/// A position in a NUL-terminated C string that is read a line at a time,
/// as C's `strchr` walk reads it, without measuring the string first:
/// `git_commit_header_field` never reads the lines after the one it
/// returns, which matters for commits whose `mergetag`/`gpgsig` headers
/// are many times longer than the `tree` and `parent` lines before them
/// (perf WP: measuring the header first cost 25% on git.git's commits).
pub(crate) struct CLines<'a> {
    /// At or before the string's NUL.
    p: *const c_char,
    _s: core::marker::PhantomData<&'a CStr>,
}

impl<'a> CLines<'a> {
    /// A cursor at the start of `s`.
    pub(crate) fn new(s: &'a CStr) -> CLines<'a> {
        // SAFETY: a `CStr` is NUL-terminated, live and unwritten for `'a`.
        unsafe { CLines::from_ptr(s.as_ptr()) }
    }

    /// A cursor at the start of the C string at `p`, which is not measured
    /// (a `CStr` would be: `strlen`).
    ///
    /// # Safety
    ///
    /// `p` points to a NUL-terminated string that stays live and unwritten
    /// for `'a`.
    pub(crate) unsafe fn from_ptr(p: *const c_char) -> CLines<'a> {
        CLines {
            p,
            _s: core::marker::PhantomData,
        }
    }

    /// A cursor at the start of `s`, which is not measured; `None` for
    /// NULL.
    pub(crate) fn from_arg(s: CStrArg<'a>) -> Option<CLines<'a>> {
        if s.is_null() {
            return None;
        }
        // SAFETY: a non-NULL `CStrArg` is a NUL-terminated string, live and
        // unwritten for `'a` (its invariant).
        Some(unsafe { CLines::from_ptr(s.as_ptr()) })
    }

    /// The byte at the position (0 at the end).
    fn peek(&self) -> u8 {
        // SAFETY: `p` is at or before the NUL of the string borrowed for
        // `'a`, so it is readable.
        unsafe { self.p.read() as u8 }
    }

    /// Move past one byte, unless at the end.
    fn skip_byte(&mut self) {
        if self.peek() != 0 {
            // Not the NUL: the next byte is at or before it.
            self.p = self.p.wrapping_add(1);
        }
    }

    /// The offset of the next `\n` from the position, if there is one
    /// before the NUL.
    #[cfg(not(miri))]
    fn find_nl(&self) -> Option<usize> {
        // SAFETY: `p` points into a NUL-terminated string (the invariant),
        // which `strchr` reads up to that NUL at most.
        let eol = unsafe { libc::strchr(self.p, c_int::from(b'\n')) };
        // (`addr` differences: no provenance is needed for a length.)
        (!eol.is_null()).then(|| eol.addr() - self.p.addr())
    }

    /// As above, for Miri (which does not run `strchr`).
    #[cfg(miri)]
    fn find_nl(&self) -> Option<usize> {
        let mut n = 0;
        loop {
            // SAFETY: bytes `0..n` were neither `\n` nor the NUL, so byte `n`
            // is at or before the NUL.
            match unsafe { self.p.wrapping_add(n).read() } as u8 {
                b'\n' => return Some(n),
                0 => return None,
                _ => n += 1,
            }
        }
    }

    /// The line at the position, without its `\n`, moving past it; `None`
    /// (not moving) if no `\n` comes before the NUL.
    fn next_line(&mut self) -> Option<&'a [u8]> {
        let len = self.find_nl()?;
        // SAFETY: the `len` bytes from `p` come before the `\n` that
        // `find_nl` found, so before the NUL: part of the string, which is
        // live and unwritten for `'a`.
        let line = unsafe { core::slice::from_raw_parts(self.p.cast::<u8>(), len) };
        // Past the `\n`, which is not the NUL: at or before it.
        self.p = self.p.wrapping_add(len + 1);
        Some(line)
    }
}

/// [`header_field`] of the C string `raw_header`, read as C reads it: line
/// by line, up to the field's last line only.
pub(crate) fn header_field_cstr(raw_header: &CStr, field: &[u8], out: &mut GitStr) -> Result<()> {
    match header_field_lines(CLines::new(raw_header), field, out) {
        // A line has no `\n`, so a field with one never matches here; it
        // can match across lines (and then fail as C does), which the
        // whole-buffer version handles.
        Err(ErrorCode::NOT_FOUND) if field.contains(&b'\n') => {
            errors::clear();
            header_field_in(raw_header.to_bytes(), field, out)
        }
        r => r,
    }
}

/// [`header_field_cstr`] of the C string `raw_header` (`""` for NULL),
/// which is not measured (except to match a field with a `\n`).
pub(crate) fn header_field_arg(
    raw_header: CStrArg<'_>,
    field: &[u8],
    out: &mut GitStr,
) -> Result<()> {
    let Some(lines) = CLines::from_arg(raw_header) else {
        return header_field_cstr(c"", field, out);
    };
    match header_field_lines(lines, field, out) {
        // As in `header_field_cstr`.
        Err(ErrorCode::NOT_FOUND) if field.contains(&b'\n') => {
            errors::clear();
            header_field_in(raw_header.to_bytes(), field, out)
        }
        r => r,
    }
}

/// The search of [`header_field_cstr`], for a field without `\n`.
fn header_field_lines(mut lines: CLines<'_>, field: &[u8], out: &mut GitStr) -> Result<()> {
    out.clear();
    if field.is_empty() {
        return Err(no_such_field(field));
    }
    while let Some(line) = lines.next_line() {
        // Continuation lines (SP) are skipped; the field must be followed
        // by SP on its line (a field without `\n` cannot match across
        // lines). The first byte is compared on its own first.
        if line.first() != field.first()
            || line[0] == b' '
            || line.len() <= field.len()
            || line[field.len()] != b' '
            || !line.starts_with(field)
        {
            continue;
        }
        out.put(&line[field.len() + 1..])?;
        // If the next line starts with SP, it's multi-line, we must continue
        while lines.peek() == b' ' {
            out.putc(b'\n')?;
            lines.skip_byte();
            let Some(cont) = lines.next_line() else {
                return Err(malformed());
            };
            out.put(cont)?;
        }
        return Ok(());
    }
    Err(no_such_field(field))
}

#[cold]
fn no_such_field(field: &[u8]) -> ErrorCode {
    let _ = git_error_set!(
        ErrorClass::OBJECT,
        "no such field '{}'",
        errors::Bytes(field)
    );
    ErrorCode::NOT_FOUND
}

fn malformed() -> ErrorCode {
    git_error_set!(ErrorClass::OBJECT, "malformed header")
}

/// `git_commit__extract_signature`'s split of a commit's bytes (as a C
/// string: up to the first NUL) into the value of the `field` header and
/// the rest (the signed data).
pub fn extract_signature(
    data: &[u8],
    field: &[u8],
    signature: &mut GitStr,
    signed_data: &mut GitStr,
) -> Result<()> {
    let buf = &data[..memchr::memchr(0, data).unwrap_or(data.len())];
    let at = |i: usize| buf.get(i).copied().unwrap_or(0);
    let mut pos = 0usize;
    // An empty field matches every line without advancing: the C loops
    // forever on the first line that does not start with a space. It is
    // never found here (DEFECTS.md).
    let lines = if field.is_empty() { None } else { Some(()) };
    while let Some(h) = lines.and_then(|()| memchr::memchr(b'\n', &buf[pos..]).map(|i| pos + i)) {
        if at(h + 1) == 0 {
            break;
        }
        let h = h + 1;
        if !buf[pos..].starts_with(field) {
            signed_data.put(&buf[pos..h])?;
            pos = h;
            continue;
        }
        let mut hh = pos + field.len();
        let eol = memchr::memchr(b'\n', &buf[hh..]).map(|i| hh + i);
        if at(hh) != b' ' {
            // (C: the line is neither copied nor skipped past; the search
            // goes on from after the field name.)
            pos = hh;
            continue;
        }
        let Some(mut eol) = eol else {
            return Err(malformed());
        };
        hh += 1; // skip the SP
        signature.put(&buf[hh..eol])?;
        // If the next line starts with SP, it's multi-line, we must continue
        while at(eol + 1) == b' ' {
            signature.putc(b'\n')?;
            let start = eol + 2;
            let Some(i) = memchr::memchr(b'\n', &buf[start.min(buf.len())..]) else {
                return Err(malformed());
            };
            eol = start + i;
            signature.put(&buf[start..eol])?;
        }
        return signed_data.puts(&buf[eol + 1..]);
    }
    let _ = git_error_set!(ErrorClass::OBJECT, "this commit is not signed");
    Err(ErrorCode::NOT_FOUND)
}

// ---------------------------------------------------------------------------
// Creation
// ---------------------------------------------------------------------------

/// `format_header_field`: `field SP content LF`, each newline of `content`
/// followed by a space (a continuation).
pub fn format_header_field(out: &mut GitStr, field: &[u8], content: &[u8]) -> Result<()> {
    let _ = out.puts(field);
    let _ = out.putc(b' ');
    let mut c = content;
    while let Some(lf) = memchr::memchr(b'\n', c) {
        let _ = out.put(&c[..lf]);
        let _ = out.puts(b"\n ");
        c = &c[lf + 1..];
    }
    let _ = out.puts(c);
    let _ = out.putc(b'\n');
    if out.is_oom() {
        Err(ErrorCode::GENERIC)
    } else {
        Ok(())
    }
}

/// `git_object__write_oid_header` (the object module's).
pub(crate) fn write_oid_header(buf: &mut GitStr, header: &[u8], id: &Oid) -> Result<()> {
    crate::object::write_oid_header(buf, header, id)
}

/// A signature to write: its fields, as C strings may be NULL.
#[derive(Clone, Copy, Debug)]
pub struct SigOut<'a> {
    pub name: Option<&'a [u8]>,
    pub email: Option<&'a [u8]>,
    pub when: GitTime,
}

/// `git_commit__create_buffer_internal`: the commit's bytes. On failure the
/// buffer is disposed, as C does.
#[allow(clippy::too_many_arguments)] // the C function's fields, one each
pub fn create_buffer_internal(
    out: &mut GitStr,
    author: &SigOut<'_>,
    committer: &SigOut<'_>,
    message: &[u8],
    tree: &Oid,
    parents: &[Oid],
    message_encoding: Option<&[u8]>,
    extra_headers: &[(&[u8], &[u8])],
) -> Result<()> {
    let r = (|| {
        write_oid_header(out, b"tree ", tree)?;
        for p in parents {
            write_oid_header(out, b"parent ", p)?;
        }
        write_signature(out, b"author ", author.name, author.email, author.when);
        write_signature(
            out,
            b"committer ",
            committer.name,
            committer.email,
            committer.when,
        );
        if let Some(enc) = message_encoding {
            format_header_field(out, b"encoding", enc)?;
        }
        for (f, v) in extra_headers {
            format_header_field(out, f, v)?;
        }
        let _ = out.putc(b'\n');
        out.puts(message)
    })();
    if r.is_err() {
        out.dispose();
    }
    r
}

/// `append_header`: `raw_content` with `name value` added at the end of its
/// header.
pub fn append_header(
    out: &mut GitStr,
    raw_content: &[u8],
    name: &[u8],
    value: &[u8],
) -> Result<()> {
    // Identifying the end of the commit header area
    let Some(header_end) = crate::util::util::memmem(raw_content, b"\n\n") else {
        return Err(git_error_set!(
            ErrorClass::INVALID,
            "malformed commit contents"
        ));
    };
    // The header ends after the first LF
    let header_end = header_end + 1;
    let _ = out.put(&raw_content[..header_end]);
    format_header_field(out, name, value)?;
    let _ = out.puts(&raw_content[header_end..]);
    if out.is_oom() {
        Err(ErrorCode::GENERIC)
    } else {
        Ok(())
    }
}

/// The parents of a commit being created, as `git_commit_parent_callback`
/// gives them: the `n`th, or `None` after the last.
pub(crate) type ParentSource<'a> = &'a mut dyn FnMut(usize) -> Option<Oid>;

/// `git_object__is_valid(repo, id, t)`, with C's result for a NULL
/// repository (`git_repository_odb__weakptr`'s failed assert, under strict
/// validation).
fn object_is_valid(repo: Option<&git_repository>, id: &Oid, t: c_int) -> bool {
    match repo {
        Some(r) => crate::object::is_valid(r, id, t),
        None if !crate::object::strict_input_validation() => true,
        None => {
            let _ = errors::assert_arg_failed("repo");
            false
        }
    }
}

/// `repo` as the `git_repository *` the other modules' exports take (they
/// read it through `&` only, as its state is interior-mutable).
pub(crate) fn repo_raw(repo: Option<&git_repository>) -> *mut git_repository {
    repo.map_or(ptr::null_mut(), |r| ptr::from_ref(r).cast_mut())
}

/// `validate_tree_and_parents`: collect the parents, checking (when
/// `validate`) that the tree and the parents exist with the right types,
/// and that `current_id` (when given) is the first parent.
fn validate_tree_and_parents(
    parents: &mut GitArray<Oid>,
    repo: Option<&git_repository>,
    tree: &Oid,
    next: ParentSource<'_>,
    current_id: Option<Oid>,
    validate: bool,
) -> Result<()> {
    if validate && !object_is_valid(repo, tree, OBJ_TREE) {
        return Err(ErrorCode::GENERIC);
    }
    let mut i = 0;
    let r = (|| {
        while let Some(parent) = next(i) {
            if validate && !object_is_valid(repo, &parent, OBJ_COMMIT) {
                return Err(ErrorCode::GENERIC);
            }
            parents.push(parent)?;
            i += 1;
        }
        if let Some(cur) = current_id
            && parents.as_slice().first() != Some(&cur)
        {
            let _ = git_error_set!(
                ErrorClass::OBJECT,
                "failed to create commit: current tip is not the first parent"
            );
            return Err(ErrorCode::MODIFIED);
        }
        Ok(())
    })();
    if r.is_err() {
        parents.clear();
    }
    r
}

/// A signature as [`SigOut`] (`None` for NULL, which the C dereferences:
/// see DEFECTS.md).
///
/// # Safety
///
/// `sig`'s strings are NULL or NUL-terminated, live and unwritten for
/// `'a`.
unsafe fn sig_out<'a>(sig: Option<&'a GitSignature>) -> Option<SigOut<'a>> {
    let s = sig?;
    Some(SigOut {
        // SAFETY: NULL or NUL-terminated, live and unwritten for `'a` (the
        // contract).
        name: unsafe { opt_cbytes(s.name) },
        // SAFETY: as for the name.
        email: unsafe { opt_cbytes(s.email) },
        when: s.when,
    })
}

/// The bytes of a C string, or `None` for NULL.
///
/// # Safety
///
/// `p` is NULL or NUL-terminated, live and unwritten for `'a`.
pub(crate) unsafe fn opt_cbytes<'a>(p: *const c_char) -> Option<&'a [u8]> {
    // SAFETY: NULL or a NUL-terminated string, live and unwritten for `'a`
    // (the contract), which is `cstr`'s.
    unsafe { crate::util::export::cstr(p) }.map(CStr::to_bytes)
}

/// Extra headers, or the C expression of the `GIT_ASSERT_ARG` that a NULL
/// string fails in `format_header_field`.
pub(crate) type Headers<'a> = core::result::Result<Vec<(&'a [u8], &'a [u8])>, &'static str>;

/// The extra headers of an options struct.
///
/// # Safety
///
/// `headers` points to `len` readable `git_commit_header`s (or `len` is
/// 0), whose strings are NULL or NUL-terminated, all live and unwritten
/// for `'a`.
unsafe fn extra_headers<'a>(headers: *const GitCommitHeader, len: usize) -> Result<Headers<'a>> {
    if len == 0 {
        return Ok(Ok(Vec::new()));
    }
    let mut v = Vec::new();
    v.try_reserve(len).map_err(|_| errors::oom())?;
    for i in 0..len {
        // SAFETY: `i < len`: one of the `len` readable headers (the
        // contract; `len > 0`, so `headers` is not NULL), copied.
        let h = unsafe { nth(headers, i) };
        // SAFETY: NULL or NUL-terminated, live and unwritten for `'a` (the
        // contract).
        let f = unsafe { opt_cbytes(h.field) };
        // SAFETY: as for `field`.
        let val = unsafe { opt_cbytes(h.value) };
        let Some(f) = f else { return Ok(Err("field")) };
        let Some(val) = val else {
            return Ok(Err("content"));
        };
        v.push((f, val));
    }
    Ok(Ok(v))
}

/// What `git_commit__create_internal` writes, in Rust form. A `None`
/// signature or message is the C's NULL, reported when the buffer is made.
pub(crate) struct CreateArgs<'a> {
    pub author: Option<SigOut<'a>>,
    pub committer: Option<SigOut<'a>>,
    pub message: Option<&'a [u8]>,
    pub tree: Oid,
    pub encoding: Option<&'a [u8]>,
    pub headers: Headers<'a>,
    pub update_ref: Option<&'a CStr>,
    pub validate: bool,
}

/// The signing callback: given the commit's buffer, `< 0` to fail.
pub(crate) type Signer<'a> = &'a mut dyn FnMut(&mut GitStr) -> c_int;

/// The buffer of `args` and `parents` (`git_commit__create_buffer_internal`
/// with the C's NULL checks).
pub(crate) fn args_buffer(out: &mut GitStr, args: &CreateArgs<'_>, parents: &[Oid]) -> Result<()> {
    // The C fails these after writing the start of the buffer, which it
    // then disposes.
    let fail = |out: &mut GitStr, expr: &str| {
        out.dispose();
        Err(errors::assert_arg_failed(expr))
    };
    let (Some(a), Some(c)) = (args.author.as_ref(), args.committer.as_ref()) else {
        return fail(out, "sig");
    };
    let headers = match &args.headers {
        Ok(h) => h.as_slice(),
        Err(expr) => return fail(out, expr),
    };
    let Some(msg) = args.message else {
        // git_str_puts's GIT_ASSERT_ARG(string), after the header.
        return fail(out, "string");
    };
    create_buffer_internal(out, a, c, msg, &args.tree, parents, args.encoding, headers)
}

/// `git_commit__create_internal`. `next` and `sign` may run C callbacks:
/// the closures' own `unsafe` blocks state their contracts.
pub(crate) fn create_internal(
    id: &mut Oid,
    repo: Option<&git_repository>,
    args: &CreateArgs<'_>,
    next: ParentSource<'_>,
    sign: Option<Signer<'_>>,
) -> Result<()> {
    let mut r#ref: *mut pend::git_reference = ptr::null_mut();
    let update_ref = CStrArg::from(args.update_ref);
    if !update_ref.is_null() {
        // SAFETY: `ref` is a writable local; `repo` NULL (which the export
        // rejects) or live (a reference, read through `&` only);
        // `update_ref` a NUL-terminated string (a `CStr`).
        let e = unsafe {
            crate::refs::ffi::git_reference_lookup_resolved(
                Out::from(&mut r#ref),
                repo_raw(repo),
                update_ref,
                10,
            )
        };
        if e < 0 && e != ErrorCode::NOT_FOUND.raw() {
            return Err(ec(e));
        }
    }
    errors::clear();

    let current_id = if r#ref.is_null() {
        None
    } else {
        // SAFETY: the reference just looked up, live until freed below.
        let t = unsafe { crate::refs::ffi::git_reference_target(r#ref) };
        // SAFETY: NULL or the live reference's id, aligned and readable.
        (!t.is_null()).then(|| unsafe { t.read() })
    };

    let mut parents = GitArray::new();
    let mut buf = GitStr::new();
    let r = (|| -> Result<()> {
        validate_tree_and_parents(
            &mut parents,
            repo,
            &args.tree,
            next,
            current_id,
            args.validate,
        )?;
        args_buffer(&mut buf, args, parents.as_slice())?;

        if let Some(sign) = sign {
            let e = sign(&mut buf);
            if e < 0 {
                return Err(ec(e));
            }
        }

        let Some(repo) = repo else {
            // `git_repository_odb__weakptr`'s check.
            return Err(errors::assert_arg_failed("repo"));
        };
        let odb = crate::object::repo_odb_weak(repo)?;
        // SAFETY: the repository's odb, live while it is (for the call);
        // the tree id readable.
        ErrorCode::check(unsafe {
            crate::odb::ffi::git_odb__freshen(Some(odb), Some(&args.tree))
        })?;
        let bytes = buf.as_bytes();
        // SAFETY: `id` a writable id (a `&mut`); the live odb; `bytes`
        // readable for its length.
        ErrorCode::check(unsafe {
            crate::odb::ffi::git_odb_write(
                Out::from(&mut *id),
                Some(odb),
                bytes.as_ptr().cast(),
                bytes.len(),
                OBJ_COMMIT,
            )
        })?;

        if !update_ref.is_null() {
            // SAFETY: a live repository; `ref` NULL or the reference looked
            // up above; `update_ref` and the message NUL-terminated; `id`
            // readable.
            ErrorCode::check_zero(unsafe {
                crate::refs::ffi::git_reference__update_for_commit(
                    repo_raw(Some(repo)),
                    r#ref,
                    update_ref,
                    id,
                    c"commit".as_ptr(),
                )
            })?;
        }
        Ok(())
    })();
    // SAFETY: NULL or the reference looked up above, freed once, not used
    // after.
    unsafe { crate::refs::ffi::git_reference_free(r#ref) };
    r
}

/// `git_repository_odb__weakptr`.
///
/// # Safety
///
/// `repo` is a live repository, not freed during the call.
pub(crate) unsafe fn repo_odb(repo: *mut git_repository) -> Result<*mut pend::git_odb> {
    // SAFETY: a live repository (the contract), only read through `&` (its
    // state is interior-mutable).
    let repo = unsafe { &*repo };
    repo.odb_weak().map(NonNull::as_ptr)
}

/// The owner of a tree (`git_tree_owner`).
fn tree_owner(tree: &crate::tree::GitTree) -> *mut git_repository {
    tree.object.repo
}

/// The id of a tree (`git_tree_id`).
fn tree_id_of(tree: &crate::tree::GitTree) -> Oid {
    tree.object.cached.oid
}

/// `p[i]`.
///
/// # Safety
///
/// `p` addresses at least `i + 1` readable `T`s.
#[inline]
pub(crate) unsafe fn nth<T: Copy>(p: *const T, i: usize) -> T {
    // SAFETY: `i` is within the readable run (contract).
    let q = unsafe { p.add(i) };
    // SAFETY: a readable, initialized `T` (contract).
    unsafe { q.read() }
}

/// A C status (non-zero) as an `ErrorCode`.
#[inline]
pub(crate) fn ec(e: c_int) -> ErrorCode {
    ErrorCode::from_raw(e).unwrap_or(ErrorCode::GENERIC)
}

/// The C callers' error code.
pub(crate) fn code(r: Result<()>) -> c_int {
    match r {
        Ok(()) => 0,
        Err(e) => e.raw(),
    }
}

pub mod ffi {
    //! The exports of `commit.c`.
    //!
    //! Commits arrive as `Option<CommitRef<'_>>` (NULL or a live commit:
    //! see [`CommitRef`]); trees, repositories, signatures and options as
    //! `Option<&T>` (only read: published objects are immutable but for
    //! their atomic refcount, repositories are interior-mutable). Ids that
    //! another argument may also point to (the tree id, the parents) stay
    //! raw and are copied on entry, as the out-parameter `id` may be the
    //! same variable.
    use crate::git_arg;
    use crate::util::cabi::CStrArg;
    use crate::util::cabi::Out;

    use super::*;
    use crate::buf::GitBuf;
    use crate::buf::ffi::{git_buf_fromstr, git_buf_tostr};
    use crate::commit::deps::GitCommitArray;
    use crate::tree::GitTree;
    use crate::util::export::barrier;

    /// `GIT_BUF_WRAP_PRIVATE(buf, fn, …)`.
    ///
    /// # Safety
    ///
    /// `buf` is NULL or a valid `git_buf` that nothing else (`f` included)
    /// accesses during the call.
    pub(crate) unsafe fn wrap_buf(
        buf: *mut GitBuf,
        f: impl FnOnce(&mut GitStr) -> Result<()>,
    ) -> c_int {
        // SAFETY: `buf` is NULL or a valid `git_buf` that nothing else
        // reaches during the call (the contract), which is `wrap_private`'s.
        unsafe { crate::buf::wrap_private(buf, f) }
    }

    /// `git_commit__free`.
    ///
    /// # Safety
    ///
    /// `commit` is NULL or a commit allocated with `git__allocator` (zeroed,
    /// then parsed or not), which nothing uses during or after the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit__free(commit: *mut c_void) {
        barrier(|| {
            let c = commit.cast::<GitCommit>();
            if c.is_null() {
                return;
            }
            // SAFETY: an initialized commit that the caller hands over:
            // ours alone (the contract).
            let cm = unsafe { &mut *c };
            cm.parent_ids.dispose();
            // The commit's own allocations (NULL or `git__allocator`
            // blocks the parse made), each freed once; `cm` is not used
            // after the last.
            // SAFETY: as above.
            unsafe { free_signature(cm.author) };
            // SAFETY: as above.
            unsafe { free_signature(cm.committer) };
            // SAFETY: as above.
            unsafe { alloc::free(cm.raw_header.cast()) };
            // SAFETY: as above.
            unsafe { alloc::free(cm.raw_message.cast()) };
            // SAFETY: as above.
            unsafe { alloc::free(cm.message_encoding.cast()) };
            // SAFETY: as above (written by `git_commit_summary`).
            unsafe { alloc::free(cm.summary.cast()) };
            // SAFETY: as above (written by `git_commit_body`).
            unsafe { alloc::free(cm.body.cast()) };
            // SAFETY: the commit's `git__allocator` block (the contract),
            // freed once; nothing reads it after.
            unsafe { alloc::free(commit) };
        })
    }

    /// `git_commit_create_ext_options_init`.
    ///
    /// # Safety
    ///
    /// `opts` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create_ext_options_init(
        mut opts: Out<'_, GitCommitCreateExtOptions>,
        version: c_uint,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!opts.is_null(), "opts", -1);
            let mut v = GitCommitCreateExtOptions::INIT;
            v.version = version;
            opts.write(v);
            0
        })
    }

    /// `git_commit_create_options_init`.
    ///
    /// # Safety
    ///
    /// `opts` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create_options_init(
        mut opts: Out<'_, GitCommitCreateOptions>,
        version: c_uint,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!opts.is_null(), "opts", -1);
            let mut v = GitCommitCreateOptions::INIT;
            v.version = version;
            opts.write(v);
            0
        })
    }

    /// `git_commit_create_from_callback`.
    ///
    /// # Safety
    ///
    /// The C's: `id` NULL or writable; `repo` NULL or live; the signatures
    /// NULL or valid (their strings NULL or NUL-terminated); the strings
    /// NULL or NUL-terminated; `tree` NULL or readable; `parent_cb` NULL or
    /// sound to call with `parent_payload`, returning NULL or a readable
    /// id. All unwritten by others during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create_from_callback(
        id: Out<'_, Oid>,
        repo: Option<&git_repository>,
        update_ref: *const c_char,
        author: Option<&GitSignature>,
        committer: Option<&GitSignature>,
        message_encoding: *const c_char,
        message: CStrArg<'_>,
        tree: *const Oid,
        parent_cb: pend::GitCommitParentCallback,
        parent_payload: *mut c_void,
    ) -> c_int {
        barrier(|| {
            let mut opts = GitCommitCreateExtOptions::INIT;
            opts.update_ref = update_ref;
            opts.message_encoding = message_encoding;
            let mut next = |i: usize| -> Option<Oid> {
                let cb = parent_cb?;
                // SAFETY: the user's callback, sound with its payload (this
                // export's contract).
                let p = unsafe { cb(i, parent_payload) };
                // SAFETY: NULL or a readable id (the callback's contract),
                // copied.
                (!p.is_null()).then(|| unsafe { p.read() })
            };
            // SAFETY: NULL or a readable id (this export's contract), copied
            // now (`id` may be the same variable).
            let tree = unsafe { tree.as_ref() }.copied();
            // SAFETY: the signatures' strings and the options' strings are
            // NULL or NUL-terminated, unwritten during the call (this
            // export's contract); no sign callback.
            unsafe {
                create_from_c(
                    id,
                    repo,
                    author,
                    committer,
                    message,
                    tree,
                    &mut next,
                    Some(&opts),
                    true,
                )
            }
            .0
        })
    }

    /// [`create_internal`] with C's out-parameter and NULL checks; the
    /// status, and the id if it was written (by `git_odb_write`, before a
    /// failing reference update).
    ///
    /// # Safety
    ///
    /// `author`'s and `committer`'s strings are NULL or NUL-terminated;
    /// `opts`'s `update_ref`, `message_encoding` and headers are NULL or
    /// valid (`extra_headers_len` readable headers with NULL or
    /// NUL-terminated strings), and its `sign` NULL or sound to call with
    /// a builder, the repository, the commit's content and `payload`. All
    /// live and unwritten by others during the call.
    #[allow(clippy::too_many_arguments)]
    unsafe fn create_from_c(
        mut id: Out<'_, Oid>,
        repo: Option<&git_repository>,
        author: Option<&GitSignature>,
        committer: Option<&GitSignature>,
        message: CStrArg<'_>,
        tree: Option<Oid>,
        next: ParentSource<'_>,
        opts: Option<&GitCommitCreateExtOptions>,
        validate: bool,
    ) -> (c_int, Option<Oid>) {
        if id.is_null() {
            return (errors::assert_arg_failed("id").raw(), None);
        }
        let Some(tree) = tree else {
            return (errors::assert_arg_failed("tree").raw(), None);
        };
        let headers = match opts {
            // SAFETY: the options' headers are `extra_headers_len` readable
            // headers with NULL or NUL-terminated strings, unwritten during
            // the call (the contract).
            Some(o) => match unsafe { extra_headers(o.extra_headers, o.extra_headers_len) } {
                Ok(h) => h,
                Err(e) => return (e.raw(), None),
            },
            None => Ok(Vec::new()),
        };
        let args = CreateArgs {
            // SAFETY: the signature's strings are NULL or NUL-terminated,
            // unwritten during the call (the contract).
            author: unsafe { sig_out(author) },
            // SAFETY: as for the author.
            committer: unsafe { sig_out(committer) },
            message: message.get().map(CStr::to_bytes),
            tree,
            // SAFETY: NULL or NUL-terminated, unwritten during the call (the
            // contract).
            encoding: opts.and_then(|o| unsafe { opt_cbytes(o.message_encoding) }),
            headers,
            // SAFETY: as for the encoding.
            update_ref: opts.and_then(|o| unsafe { crate::util::export::cstr(o.update_ref) }),
            validate,
        };
        let mut sign_c;
        let sign: Option<Signer<'_>> = match opts.and_then(|o| o.sign.map(|f| (f, o.payload))) {
            Some((f, payload)) => {
                sign_c = move |buf: &mut GitStr| {
                    let content = buf.as_ptr();
                    let mut builder = GitCommitbuilder { contents: buf };
                    // SAFETY: the user's callback, sound with its payload
                    // (the contract), given a builder over `buf` (live for
                    // the call, reached only through the builder meanwhile),
                    // the repository and `buf`'s NUL-terminated content.
                    unsafe { f(&raw mut builder, repo_raw(repo), content, payload) }
                };
                Some(&mut sign_c)
            }
            None => None,
        };
        let mut out = Oid::default();
        let r = create_internal(&mut out, repo, &args, next, sign);
        // The C writes `id` with git_odb_write before updating the ref.
        let written = (r.is_ok() || out != Oid::default()).then_some(out);
        if let Some(v) = written {
            id.write(v);
        }
        (code(r), written)
    }

    /// The parents of `git_commit__create_v`'s array.
    ///
    /// # Safety
    ///
    /// `parents` holds `n` readable pointers, each NULL or a live commit,
    /// for as long as the result is used.
    unsafe fn from_v_array<'a>(
        parents: *const *const GitCommit,
        n: usize,
    ) -> impl FnMut(usize) -> Option<Oid> + 'a {
        move |i| {
            if i >= n {
                return None;
            }
            // SAFETY: `i < n`: one of the `n` readable pointers (the
            // contract), copied.
            let c = unsafe { nth(parents, i) };
            // SAFETY: NULL or a live commit (the contract).
            unsafe { CommitRef::from_raw(c) }.map(|c| *c.id())
        }
    }

    /// `git_commit__create_v`: `git_commit_create_v`'s body, called by its
    /// C shim (`src/util/rust_shims.c`) with the variadic parents collected.
    ///
    /// # Safety
    ///
    /// As `git_commit_create`; `parents` holds `parent_count` readable
    /// pointers (NULL ends the list early, as the C's `va_arg` reading does).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit__create_v(
        id: Out<'_, Oid>,
        repo: Option<&git_repository>,
        update_ref: *const c_char,
        author: Option<&GitSignature>,
        committer: Option<&GitSignature>,
        message_encoding: *const c_char,
        message: CStrArg<'_>,
        tree: Option<&GitTree>,
        parent_count: usize,
        parents: *const *const GitCommit,
    ) -> c_int {
        barrier(|| {
            let tree = git_arg!(tree, "tree", -1);
            git_assert_arg!(
                tree_owner(tree) == repo_raw(repo),
                "git_tree_owner(tree) == repo",
                -1
            );
            let mut opts = GitCommitCreateExtOptions::INIT;
            opts.update_ref = update_ref;
            opts.message_encoding = message_encoding;
            // SAFETY: `parent_count` readable pointers, NULL or live commits
            // (this export's contract), for the call.
            let mut next = unsafe { from_v_array(parents, parent_count) };
            // SAFETY: the strings and signatures of this export's contract
            // (`git_commit_create`'s); no sign callback.
            unsafe {
                create_from_c(
                    id,
                    repo,
                    author,
                    committer,
                    message,
                    Some(tree_id_of(tree)),
                    &mut next,
                    Some(&opts),
                    false,
                )
            }
            .0
        })
    }

    /// `git_commit_create_from_ids`.
    ///
    /// # Safety
    ///
    /// As `git_commit_create_from_callback`; `parents` holds `parent_count`
    /// readable pointers, each NULL or a readable id.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create_from_ids(
        id: Out<'_, Oid>,
        repo: Option<&git_repository>,
        update_ref: *const c_char,
        author: Option<&GitSignature>,
        committer: Option<&GitSignature>,
        message_encoding: *const c_char,
        message: CStrArg<'_>,
        tree: *const Oid,
        parent_count: usize,
        parents: *const *const Oid,
    ) -> c_int {
        barrier(|| {
            let mut opts = GitCommitCreateExtOptions::INIT;
            opts.update_ref = update_ref;
            opts.message_encoding = message_encoding;
            let mut next = |i: usize| -> Option<Oid> {
                if i >= parent_count {
                    return None;
                }
                // SAFETY: `i < parent_count`: one of the readable pointers
                // (this export's contract), copied.
                let p = unsafe { nth(parents, i) };
                // SAFETY: NULL or a readable id, copied.
                (!p.is_null()).then(|| unsafe { p.read() })
            };
            // SAFETY: NULL or a readable id (this export's contract), copied
            // now (`id` may be the same variable).
            let tree = unsafe { tree.as_ref() }.copied();
            // SAFETY: the strings and signatures of this export's contract;
            // no sign callback.
            unsafe {
                create_from_c(
                    id,
                    repo,
                    author,
                    committer,
                    message,
                    tree,
                    &mut next,
                    Some(&opts),
                    true,
                )
            }
            .0
        })
    }

    /// `commit_parent_from_array`: the parents in `parents` that belong to
    /// `repo` (the first that does not ends the list).
    ///
    /// # Safety
    ///
    /// `parents` holds `n` readable pointers to live commits (or NULL), for
    /// as long as the result is used.
    unsafe fn from_array<'a>(
        parents: *const *mut GitCommit,
        n: usize,
        repo: *mut git_repository,
    ) -> impl FnMut(usize) -> Option<Oid> + 'a {
        move |i| {
            if i >= n {
                return None;
            }
            // SAFETY: `i < n`: one of the `n` readable pointers (the
            // contract), copied.
            let c = unsafe { nth(parents, i) };
            // SAFETY: NULL or a live commit (the contract). (C dereferences
            // a NULL one: here it ends the list, as `git_object_owner`'s
            // assert fails.)
            let c = match unsafe { CommitRef::from_raw(c) } {
                Some(c) => c,
                None => {
                    let _ = errors::assert_arg_failed("obj");
                    return None;
                }
            };
            if c.owner() != repo {
                return None;
            }
            Some(*c.id())
        }
    }

    /// `git_commit_create_ext` (without the barrier).
    ///
    /// # Safety
    ///
    /// As [`git_commit_create_ext`].
    #[allow(clippy::too_many_arguments)]
    unsafe fn create_ext(
        id: Out<'_, Oid>,
        repo: Option<&git_repository>,
        author: Option<&GitSignature>,
        committer: Option<&GitSignature>,
        message: CStrArg<'_>,
        tree: Option<&GitTree>,
        parent_count: usize,
        parents: *const *mut GitCommit,
        opts: Option<&GitCommitCreateExtOptions>,
    ) -> (c_int, Option<Oid>) {
        let tree = git_arg!(tree, "tree", (-1, None));
        git_assert_arg!(
            tree_owner(tree) == repo_raw(repo),
            "git_tree_owner(tree) == repo",
            (-1, None)
        );
        // SAFETY: `parent_count` readable pointers to live commits (or
        // NULL), for the call (the contract).
        let mut next = unsafe { from_array(parents, parent_count, repo_raw(repo)) };
        // SAFETY: the strings, signatures and options of the contract.
        unsafe {
            create_from_c(
                id,
                repo,
                author,
                committer,
                message,
                Some(tree_id_of(tree)),
                &mut next,
                opts,
                false,
            )
        }
    }

    /// `git_commit_create_ext`.
    ///
    /// # Safety
    ///
    /// As `git_commit_create`; `opts` NULL or a valid options struct (its
    /// strings and headers NULL or valid, `sign` NULL or sound to call with
    /// `payload`).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create_ext(
        id: Out<'_, Oid>,
        repo: Option<&git_repository>,
        author: Option<&GitSignature>,
        committer: Option<&GitSignature>,
        message: CStrArg<'_>,
        tree: Option<&GitTree>,
        parent_count: usize,
        parents: *const *mut GitCommit,
        opts: Option<&GitCommitCreateExtOptions>,
    ) -> c_int {
        barrier(|| {
            // SAFETY: this export's arguments, under its contract.
            unsafe {
                create_ext(
                    id,
                    repo,
                    author,
                    committer,
                    message,
                    tree,
                    parent_count,
                    parents,
                    opts,
                )
            }
            .0
        })
    }

    /// `git_commitbuilder_add_header`.
    ///
    /// # Safety
    ///
    /// `builder` is NULL or the builder a signing callback was given,
    /// during that callback; `field` and `value` are NULL or
    /// NUL-terminated, and not in the builder's buffer.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commitbuilder_add_header(
        builder: Option<&mut GitCommitbuilder>,
        field: CStrArg<'_>,
        value: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let builder = git_arg!(builder, "builder", -1);
            // SAFETY: a builder's `contents` is the commit's buffer, live
            // during the signing callback (this export's contract) and
            // reached only through the builder meanwhile.
            let contents = unsafe { &mut *builder.contents };
            let f = git_arg!(field.get(), "field", -1).to_bytes();
            let v = git_arg!(value.get(), "content", -1).to_bytes();
            let raw = contents.as_bytes();
            let raw = &raw[..memchr::memchr(0, raw).unwrap_or(raw.len())];
            let mut signed = GitStr::new();
            match append_header(&mut signed, raw, f, v) {
                Ok(()) => {
                    contents.swap(&mut signed);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_commit_create`.
    ///
    /// # Safety
    ///
    /// The C's: `id` NULL or writable; `repo` NULL or live; `tree` NULL or
    /// a live tree; `parents` holds `parent_count` live commits (or NULL);
    /// the signatures NULL or valid (their strings NULL or NUL-terminated);
    /// the strings NULL or NUL-terminated. All unwritten by others during
    /// the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create(
        id: Out<'_, Oid>,
        repo: Option<&git_repository>,
        update_ref: *const c_char,
        author: Option<&GitSignature>,
        committer: Option<&GitSignature>,
        message_encoding: *const c_char,
        message: CStrArg<'_>,
        tree: Option<&GitTree>,
        parent_count: usize,
        parents: *const *mut GitCommit,
    ) -> c_int {
        barrier(|| {
            let mut opts = GitCommitCreateExtOptions::INIT;
            opts.update_ref = update_ref;
            opts.message_encoding = message_encoding;
            // SAFETY: this export's arguments, under its contract (the
            // options' strings are its own).
            unsafe {
                create_ext(
                    id,
                    repo,
                    author,
                    committer,
                    message,
                    tree,
                    parent_count,
                    parents,
                    Some(&opts),
                )
            }
            .0
        })
    }

    /// `git_tree_id` of a tree a lookup returned (a place read).
    ///
    /// # Safety
    ///
    /// `tree` is a live tree.
    unsafe fn looked_up_tree_id(tree: *mut GitTree) -> Oid {
        // SAFETY: a live tree (the contract), whose id is not written after
        // it is published: read in place.
        unsafe { (*tree).object.cached.oid }
    }

    /// `git_commit_amend`.
    ///
    /// # Safety
    ///
    /// `id` NULL or writable; `commit_to_amend` NULL or a live commit; the
    /// rest NULL or as `git_commit_create`'s.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_amend(
        mut id: Out<'_, Oid>,
        commit_to_amend: Option<CommitRef<'_>>,
        update_ref: CStrArg<'_>,
        author: Option<&GitSignature>,
        committer: Option<&GitSignature>,
        message_encoding: *const c_char,
        message: CStrArg<'_>,
        tree: Option<&GitTree>,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!id.is_null(), "id", -1);
            let c = git_arg!(commit_to_amend, "commit_to_amend", -1);
            let repo = c.repository();
            let author = author.or(c.author_ref());
            let committer = committer.or(c.committer_ref());
            let message_encoding = if message_encoding.is_null() {
                c.message_encoding()
            } else {
                message_encoding
            };
            let message = if message.is_null() {
                c.message_arg()
            } else {
                message
            };
            let tree_id = match tree {
                None => {
                    let mut old: *mut GitTree = ptr::null_mut();
                    let e = commit_tree(Out::from(&mut old), c);
                    if e < 0 {
                        return e;
                    }
                    // SAFETY: the tree just looked up (non-NULL on success),
                    // live until freed below.
                    let t = unsafe { looked_up_tree_id(old) };
                    // SAFETY: our reference to it, released once.
                    unsafe { pend::git_tree_free(old) };
                    t
                }
                Some(t) => {
                    git_assert_arg!(
                        tree_owner(t) == repo_raw(repo),
                        "git_tree_owner(tree) == repo",
                        -1
                    );
                    tree_id_of(t)
                }
            };

            let mut r#ref = ptr::null_mut();
            if !update_ref.is_null() {
                // SAFETY: `ref` a writable local; `repo` NULL (which the
                // export rejects) or live; `update_ref` NUL-terminated.
                let e = unsafe {
                    crate::refs::ffi::git_reference_lookup_resolved(
                        Out::from(&mut r#ref),
                        repo_raw(repo),
                        update_ref,
                        5,
                    )
                };
                if e < 0 {
                    return e;
                }
                // SAFETY: the reference just looked up, live until freed.
                let target = unsafe { crate::refs::ffi::git_reference_target(r#ref) };
                // SAFETY: NULL or the reference's id, copied.
                if target.is_null() || unsafe { target.read() } != *c.id() {
                    // SAFETY: freed once, not used after.
                    unsafe { crate::refs::ffi::git_reference_free(r#ref) };
                    let _ = git_error_set!(
                        ErrorClass::REFERENCE,
                        "commit to amend is not the tip of the given branch"
                    );
                    return -1;
                }
            }

            let mut opts = GitCommitCreateExtOptions::INIT;
            opts.message_encoding = message_encoding;
            let pids = c.parent_ids();
            let mut next = |i: usize| pids.get(i).copied();
            // SAFETY: the signatures are this export's (valid) or the
            // commit's (`CommitRef`'s invariant); the encoding NULL or
            // NUL-terminated (likewise); no sign callback.
            let (mut error, written) = unsafe {
                create_from_c(
                    id.reborrow(),
                    repo,
                    author,
                    committer,
                    message,
                    Some(tree_id),
                    &mut next,
                    Some(&opts),
                    false,
                )
            };
            if error == 0
                && !update_ref.is_null()
                && let Some(new_id) = written
            {
                // SAFETY: `repo` NULL or live; the reference looked up above;
                // the new id readable; the message NUL-terminated.
                error = unsafe {
                    crate::refs::ffi::git_reference__update_for_commit(
                        repo_raw(repo),
                        r#ref,
                        CStrArg::NULL,
                        &raw const new_id,
                        c"commit".as_ptr(),
                    )
                };
            }
            if !update_ref.is_null() {
                // SAFETY: the reference looked up above, freed once.
                unsafe { crate::refs::ffi::git_reference_free(r#ref) };
            }
            error
        })
    }

    /// `git_commit__parse`.
    ///
    /// # Safety
    ///
    /// `commit` is NULL or a fresh (zeroed) commit that only the caller
    /// holds, whose `object.repo` is NULL or a live repository; `obj` a
    /// live odb object.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit__parse(
        commit: *mut c_void,
        obj: *mut pend::git_odb_object,
        oid_type: git_oid_t,
    ) -> c_int {
        let opts = GitCommitParseOptions { oid_type, flags: 0 };
        // SAFETY: NULL or a fresh commit only the caller holds (this
        // export's contract): exclusive for the call.
        let commit = unsafe { commit.cast::<GitCommit>().as_mut() };
        // SAFETY: this export's arguments, under `git_commit__parse_ext`'s
        // contract.
        unsafe { git_commit__parse_ext(commit, obj, Some(&opts)) }
    }

    /// `git_commit__parse_raw`.
    ///
    /// # Safety
    ///
    /// `commit` is NULL or a fresh (zeroed) commit that only the caller
    /// holds; `data` is NULL or readable for `size` bytes, unwritten during
    /// the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit__parse_raw(
        commit: *mut c_void,
        data: *const c_char,
        size: usize,
        oid_type: git_oid_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!commit.is_null(), "commit", -1);
            git_assert_arg!(!data.is_null(), "data", -1);
            // SAFETY: a non-NULL, initialized commit that only the caller
            // holds (this export's contract): exclusive for the call.
            let c = unsafe { &mut *commit.cast::<GitCommit>() };
            // SAFETY: `size` readable bytes at the non-NULL `data`,
            // unwritten during the call (this export's contract).
            let d = unsafe { crate::util::export::bytes(data, size) };
            code(parse_into(
                c,
                d,
                &GitCommitParseOptions { oid_type, flags: 0 },
            ))
        })
    }

    /// `git_commit__parse_ext`: parse `odb_obj` into `commit`, then apply
    /// the repository's grafts.
    ///
    /// # Safety
    ///
    /// `commit` is NULL or a zeroed commit that only the caller holds,
    /// whose `object.repo` is NULL or a live repository whose grafts do not
    /// change during the call; `odb_obj` a live odb object; `parse_opts`
    /// NULL or readable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit__parse_ext(
        commit: Option<&mut GitCommit>,
        odb_obj: *mut pend::git_odb_object,
        parse_opts: Option<&GitCommitParseOptions>,
    ) -> c_int {
        barrier(|| {
            let c = git_arg!(commit, "commit", -1);
            let opts = *git_arg!(parse_opts, "opts", -1);
            // SAFETY: `odb_obj` is NULL or a live odb object, not written once shared but
            // for its atomic count.
            let odb_obj_ref = unsafe { odb_obj.as_ref() };
            // SAFETY: a live odb object (this export's contract).
            let p = unsafe { crate::odb::ffi::git_odb_object_data(odb_obj_ref) }.cast::<c_char>();
            // SAFETY: `odb_obj` is NULL or a live odb object, not written once shared but
            // for its atomic count.
            let odb_obj_ref = unsafe { odb_obj.as_ref() };
            // SAFETY: as above.
            let n = unsafe { crate::odb::ffi::git_odb_object_size(odb_obj_ref) };
            if p.is_null() {
                return errors::assert_arg_failed("data").raw();
            }
            // SAFETY: the object's data, readable for its size and
            // unwritten while the object lives (the whole call).
            let data = unsafe { crate::util::export::bytes(p, n) };
            // SAFETY: `odb_obj` is NULL or a live odb object, not written once shared but
            // for its atomic count.
            let odb_obj_ref = unsafe { odb_obj.as_ref() };
            // SAFETY: the live object's id, aligned.
            let id = unsafe { crate::odb::ffi::git_odb_object_id(odb_obj_ref) };
            // SAFETY: as above, copied.
            let id = unsafe { id.read() };
            if let Err(e) = parse_into(c, data, &opts) {
                return e.raw();
            }
            // Perform necessary grafts
            // SAFETY: `object.repo` is NULL or a live repository (this
            // export's contract), only read through `&`.
            let repo = unsafe { c.object.repo.as_ref() };
            // SAFETY: the grafts do not change during the call (this
            // export's contract), and the graft is used only within it.
            if let Some(g) = unsafe { find_graft(repo, &id) } {
                // SAFETY: a live graft of the unchanged grafts (above).
                let graft = unsafe { &(*g).parents };
                return code(parents_from_graft(&mut c.parent_ids, graft));
            }
            0
        })
    }

    macro_rules! getter {
        ($(#[$m:meta])* $name:ident -> $ret:ty, $invalid:expr, |$c:ident| $body:expr) => {
            $(#[$m])*
            ///
            /// # Safety
            ///
            /// `commit` is NULL or a live commit.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $name(commit: Option<CommitRef<'_>>) -> $ret {
                barrier(|| {
                    let $c = git_arg!(commit, "commit", $invalid);
                    $body
                })
            }
        };
    }

    getter!(
        /// `git_commit_author`.
        git_commit_author -> *const GitSignature, ptr::null(), |c| c.author()
    );
    getter!(
        /// `git_commit_committer`.
        git_commit_committer -> *const GitSignature, ptr::null(), |c| c.committer()
    );
    getter!(
        /// `git_commit_message_raw`.
        git_commit_message_raw -> *const c_char, ptr::null(), |c| c.raw_message()
    );
    getter!(
        /// `git_commit_message_encoding`.
        git_commit_message_encoding -> *const c_char, ptr::null(), |c| c.message_encoding()
    );
    getter!(
        /// `git_commit_raw_header`.
        git_commit_raw_header -> *const c_char, ptr::null(), |c| c.raw_header()
    );
    getter!(
        /// `git_commit_time` (`INT64_MIN` without a committer, which the C
        /// dereferences).
        git_commit_time -> i64, i64::MIN, |c| c.committer_ref().map_or(i64::MIN, |s| s.when.time)
    );
    getter!(
        /// `git_commit_time_offset`.
        git_commit_time_offset -> c_int, -1, |c| c.committer_ref().map_or(-1, |s| s.when.offset)
    );
    getter!(
        /// `git_commit_parentcount`.
        git_commit_parentcount -> c_uint, 0, |c| c.parent_ids().len() as c_uint
    );
    getter!(
        /// `git_commit_tree_id`.
        git_commit_tree_id -> *const Oid, ptr::null(), |c| ptr::from_ref(c.tree_id())
    );
    getter!(
        /// `git_commit_message`: the raw message without its leading
        /// newlines.
        git_commit_message -> *const c_char, ptr::null(), |c| c.message_ptr()
    );

    /// `git_commit_summary`: computed on first use and kept in the commit.
    ///
    /// # Safety
    ///
    /// `commit` is NULL or a live commit, and no other thread calls this
    /// (or `git_commit__free`) on it meanwhile (as the C's).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_summary(commit: Option<CommitRef<'_>>) -> *const c_char {
        barrier(|| {
            let c = git_arg!(commit, "commit", ptr::null());
            let slot = c.summary_slot();
            // SAFETY: an aligned field of the live commit, written only here
            // (and by the free), not concurrently (this export's contract),
            // and never covered by a reference (`CommitRef`).
            if unsafe { slot.read() }.is_null() {
                let mut s = GitStr::new();
                if let Some(m) = c.message_arg().get() {
                    let _ = summary_into(m.to_bytes(), &mut s);
                }
                let mut p = s.detach().map_or(ptr::null_mut(), |d| d.into_raw());
                if p.is_null() {
                    p = alloc::strdup(c"");
                }
                // SAFETY: as for the read.
                unsafe { slot.write(p) };
            }
            // SAFETY: as for the read.
            unsafe { slot.read() }.cast_const()
        })
    }

    /// `git_commit_body`: computed on first use and kept in the commit.
    ///
    /// # Safety
    ///
    /// As [`git_commit_summary`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_body(commit: Option<CommitRef<'_>>) -> *const c_char {
        barrier(|| {
            let c = git_arg!(commit, "commit", ptr::null());
            let slot = c.body_slot();
            // SAFETY: as in `git_commit_summary`, for `body`.
            if unsafe { slot.read() }.is_null()
                && let Some(m) = c.message_arg().get()
                && let Some(b) = body_of(m.to_bytes())
            {
                // SAFETY: as for the read.
                unsafe { slot.write(alloc::strndup(b, b.len())) };
            }
            // SAFETY: as for the read.
            unsafe { slot.read() }.cast_const()
        })
    }

    /// `git_commit_tree` of a commit.
    pub(crate) fn commit_tree(tree_out: Out<'_, *mut GitTree>, c: CommitRef<'_>) -> c_int {
        // SAFETY: `tree_out` NULL or writable (`Out`'s invariant), which
        // the lookup checks; the commit's repository NULL (which the lookup
        // rejects) or live; its tree id readable.
        unsafe { pend::git_tree_lookup(tree_out.as_ptr(), c.owner(), Some(c.tree_id())) }
    }

    /// `git_commit_tree`.
    ///
    /// # Safety
    ///
    /// `tree_out` NULL or writable; `commit` NULL or a live commit.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_tree(
        tree_out: Out<'_, *mut GitTree>,
        commit: Option<CommitRef<'_>>,
    ) -> c_int {
        barrier(|| commit_tree(tree_out, git_arg!(commit, "commit", -1)))
    }

    /// `git_commit_parent_id`.
    ///
    /// # Safety
    ///
    /// `commit` is NULL or a live commit.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_parent_id(
        commit: Option<CommitRef<'_>>,
        n: c_uint,
    ) -> *const Oid {
        barrier(|| {
            let c = git_arg!(commit, "commit", ptr::null());
            c.parent_ids()
                .get(n as usize)
                .map_or(ptr::null(), ptr::from_ref)
        })
    }

    /// `git_commit_parent` of a commit.
    pub(crate) fn commit_parent(
        parent: Out<'_, *mut GitCommit>,
        c: CommitRef<'_>,
        n: c_uint,
    ) -> c_int {
        let Some(id) = c.parent_ids().get(n as usize) else {
            let _ = git_error_set!(ErrorClass::INVALID, "parent {} does not exist", n);
            return ErrorCode::NOT_FOUND.raw();
        };
        // SAFETY: `parent` NULL or writable (`Out`'s invariant), which the
        // lookup checks; the repository NULL (which the lookup rejects) or
        // live; `id` readable.
        unsafe { pend::git_commit_lookup(parent.as_ptr(), c.owner(), Some(id)) }
    }

    /// `git_commit_parent`.
    ///
    /// # Safety
    ///
    /// `parent` NULL or writable; `commit` NULL or a live commit.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_parent(
        parent: Out<'_, *mut GitCommit>,
        commit: Option<CommitRef<'_>>,
        n: c_uint,
    ) -> c_int {
        barrier(|| commit_parent(parent, git_arg!(commit, "commit", -1), n))
    }

    /// `git_commit_nth_gen_ancestor`.
    ///
    /// # Safety
    ///
    /// `ancestor` NULL or writable; `commit` NULL or a live commit.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_nth_gen_ancestor(
        mut ancestor: Out<'_, *mut GitCommit>,
        commit: Option<CommitRef<'_>>,
        n: c_uint,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!ancestor.is_null(), "ancestor", -1);
            let commit = git_arg!(commit, "commit", -1);
            let mut current = ptr::null_mut();
            // SAFETY: `current` a writable local; a live commit.
            if unsafe { pend::git_commit_dup(&raw mut current, commit.as_ptr()) } < 0 {
                return -1;
            }
            if n == 0 {
                ancestor.write(current);
                return 0;
            }
            let mut parent = ptr::null_mut();
            for _ in 0..n {
                // SAFETY: `current` is a commit we hold a reference to (from
                // the dup or the last lookup), live until released below.
                let error = match unsafe { CommitRef::from_raw(current) } {
                    Some(c) => commit_parent(Out::from(&mut parent), c, 0),
                    None => errors::assert_arg_failed("commit").raw(),
                };
                // SAFETY: our reference, released once (`c` not used after).
                unsafe { pend::git_commit_free(current) };
                if error < 0 {
                    return error;
                }
                current = parent;
            }
            ancestor.write(parent);
            0
        })
    }

    /// `git_commit_header_field`.
    ///
    /// # Safety
    ///
    /// `out` NULL or a valid `git_buf`; `commit` NULL or a live commit;
    /// `field` NULL or NUL-terminated, possibly in `out`'s buffer (read, as
    /// in C, after `out` is sanitized), and otherwise unwritten during the
    /// call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_header_field(
        out: *mut GitBuf,
        commit: Option<CommitRef<'_>>,
        field: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `field` as `header_field_of` requires (this export's
            // contract).
            let f = |s: &mut GitStr| unsafe { header_field_of(s, commit, field) };
            // SAFETY: `out` NULL or a valid `git_buf` (this export's
            // contract), which `f` does not reach.
            unsafe { wrap_buf(out, f) }
        })
    }

    /// The header field of a commit for the exports.
    ///
    /// # Safety
    ///
    /// `field` is NULL or a string readable now that, unless it lies in
    /// `out`'s buffer, nothing writes during the call.
    unsafe fn header_field_of(
        out: &mut GitStr,
        commit: Option<CommitRef<'_>>,
        field: *const c_char,
    ) -> Result<()> {
        let commit = commit.ok_or_else(|| errors::assert_arg_failed("commit"))?;
        // `field` may lie in `out`'s buffer, which the search writes: read
        // now (after the sanitize, as C) and copied if so.
        // SAFETY: this fn's contract is `arg_bytes`'s.
        let field = unsafe { out.arg_bytes(field) }.unwrap_or_default();
        // (A quick-parsed commit has no header: `""`.)
        header_field_arg(commit.raw_header_arg(), &field, out)
    }

    /// `git_commit__header_field`.
    ///
    /// # Safety
    ///
    /// `out` NULL or a valid `git_str`, not in the commit's header;
    /// `commit` NULL or a live commit; `field` NULL or NUL-terminated,
    /// possibly in `out`'s buffer, and otherwise unwritten during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit__header_field(
        out: Option<&mut GitStr>,
        commit: Option<CommitRef<'_>>,
        field: *const c_char,
    ) -> c_int {
        barrier(|| {
            let out = git_arg!(out, "out", -1);
            // SAFETY: `field` as `header_field_of` requires (this export's
            // contract).
            code(unsafe { header_field_of(out, commit, field) })
        })
    }

    /// `git_commit_extract_signature`.
    ///
    /// # Safety
    ///
    /// `signature_out`, `signed_data_out` NULL or valid, distinct
    /// `git_buf`s; `repo` NULL or live; `commit_id` NULL or readable;
    /// `field` NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_extract_signature(
        signature_out: *mut GitBuf,
        signed_data_out: *mut GitBuf,
        repo: Option<&git_repository>,
        commit_id: Option<&Oid>,
        field: *const c_char,
    ) -> c_int {
        barrier(|| {
            let mut signature = GitStr::new();
            let mut signed_data = GitStr::new();
            // SAFETY: `signature_out` NULL or a valid `git_buf` (this
            // export's contract); `signature` a local string.
            let mut error = unsafe { git_buf_tostr(&raw mut signature, signature_out) };
            if error == 0 {
                // SAFETY: as above, for `signed_data_out`.
                error = unsafe { git_buf_tostr(&raw mut signed_data, signed_data_out) };
            }
            if error == 0 {
                // SAFETY: two distinct local strings; the rest this export's
                // arguments, under the same contract.
                error = unsafe {
                    git_commit__extract_signature(
                        &raw mut signature,
                        &raw mut signed_data,
                        repo,
                        commit_id,
                        field,
                    )
                };
            }
            if error == 0 {
                // SAFETY: as for `git_buf_tostr` above.
                error = unsafe { git_buf_fromstr(signature_out, Some(&mut signature)) };
            }
            if error == 0 {
                // SAFETY: as above.
                error = unsafe { git_buf_fromstr(signed_data_out, Some(&mut signed_data)) };
            }
            error
        })
    }

    /// `git_commit__extract_signature`.
    ///
    /// # Safety
    ///
    /// `signature` and `signed_data` NULL or valid, distinct `git_str`s;
    /// the rest as `git_commit_extract_signature`'s.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit__extract_signature(
        signature: *mut GitStr,
        signed_data: *mut GitStr,
        repo: Option<&git_repository>,
        commit_id: Option<&Oid>,
        field: *const c_char,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!signature.is_null(), "signature", -1);
            git_assert_arg!(!signed_data.is_null(), "signed_data", -1);
            // SAFETY: a valid `git_str`, distinct from `signed_data` (this
            // export's contract), so exclusive for the call.
            let sig = unsafe { &mut *signature };
            // SAFETY: as above.
            let sd = unsafe { &mut *signed_data };
            sig.clear();
            sd.clear();
            // Read after the clears, as C: copied if it lies in either
            // output's buffer (which the extraction writes).
            let field = if sd.points_into(field) {
                // SAFETY: `field` is a string readable now (this export's
                // contract), in `sd`'s buffer: `arg_bytes` copies it.
                unsafe { sd.arg_bytes(field) }
            } else {
                // SAFETY: `field` is NULL or a string readable now that,
                // outside the outputs' buffers, nothing writes during the
                // call (this export's contract); in `sig`'s, it is copied.
                unsafe { sig.arg_bytes(field) }
            };
            let field = field.as_deref().unwrap_or(b"gpgsig");
            // `git_repository_odb__weakptr`'s check.
            let repo = git_arg!(repo, "repo", -1);
            let odb = match crate::object::repo_odb_weak(repo) {
                Ok(o) => o,
                Err(e) => return e.raw(),
            };
            let mut obj = ptr::null_mut();
            // SAFETY: `obj` a writable local; the repository's odb, live
            // while it is; `commit_id` NULL (which `git_odb_read` rejects) or
            // readable.
            let error =
                unsafe { crate::odb::ffi::git_odb_read(Out::from(&mut obj), Some(odb), commit_id) };
            if error < 0 {
                return error;
            }
            // SAFETY: `obj` is NULL or a live odb object, not written once shared but
            // for its atomic count.
            let obj_ref = unsafe { obj.as_ref() };
            // SAFETY: the object just read (non-NULL on success), live until
            // freed below.
            let r = if unsafe { crate::odb::ffi::git_odb_object_type(obj_ref) } != OBJ_COMMIT {
                let _ = git_error_set!(
                    ErrorClass::INVALID,
                    "the requested type does not match the type in the ODB"
                );
                Err(ErrorCode::NOT_FOUND)
            } else {
                // SAFETY: as above.
                let p = unsafe { crate::odb::ffi::git_odb_object_data(obj_ref) }.cast::<c_char>();
                // SAFETY: `obj` is NULL or a live odb object, not written once shared but
                // for its atomic count.
                let obj_ref = unsafe { obj.as_ref() };
                // SAFETY: as above.
                let n = unsafe { crate::odb::ffi::git_odb_object_size(obj_ref) };
                // SAFETY: the object's data (empty if NULL), readable for its
                // size and unwritten until it is freed below, after its last
                // use.
                let data = unsafe { crate::util::export::bytes(p, n) };
                extract_signature(data, field, sig, sd)
            };
            // SAFETY: the object read above, freed once, not used after.
            unsafe { crate::odb::ffi::git_odb_object_free(obj) };
            if let Err(e) = r {
                sig.clear();
                sd.clear();
                return e.raw();
            }
            0
        })
    }

    /// `git_commit__create_buffer` into `out`.
    ///
    /// # Safety
    ///
    /// As [`git_commit__create_buffer`] (`out` aside): the strings may lie
    /// in `out`'s buffer.
    #[allow(clippy::too_many_arguments)]
    unsafe fn create_buffer(
        out: Option<&mut GitStr>,
        repo: Option<&git_repository>,
        author: Option<&GitSignature>,
        committer: Option<&GitSignature>,
        message_encoding: *const c_char,
        message: *const c_char,
        tree: Option<&GitTree>,
        parent_count: usize,
        parents: *const *mut GitCommit,
    ) -> c_int {
        let tree = git_arg!(tree, "tree", -1);
        git_assert_arg!(
            tree_owner(tree) == repo_raw(repo),
            "git_tree_owner(tree) == repo",
            -1
        );
        let out = git_arg!(out, "out", -1);
        // The strings may lie in `out`'s buffer, which the buffer is built
        // in: read now, copied if so (C reads them later, while writing
        // `out`, which clobbers such a string: rust/DEFECTS.md).
        // SAFETY: NULL or strings readable now that, outside `out`'s
        // buffer, nothing writes during the call (this fn's contract).
        let message = unsafe { out.arg_bytes(message) };
        // SAFETY: as above.
        let message_encoding = unsafe { out.arg_bytes(message_encoding) };
        let tree_id = tree_id_of(tree);
        // SAFETY: `parent_count` readable pointers to live commits (or
        // NULL), for the call (the contract).
        let mut next = unsafe { from_array(parents, parent_count, repo_raw(repo)) };
        let mut parents_arr = GitArray::new();
        if let Err(e) =
            validate_tree_and_parents(&mut parents_arr, repo, &tree_id, &mut next, None, true)
        {
            return e.raw();
        }
        let args = CreateArgs {
            // SAFETY: the signature's strings are NULL or NUL-terminated,
            // unwritten during the call (the contract).
            author: unsafe { sig_out(author) },
            // SAFETY: as for the author.
            committer: unsafe { sig_out(committer) },
            message: message.as_deref(),
            tree: tree_id,
            encoding: message_encoding.as_deref(),
            headers: Ok(Vec::new()),
            update_ref: None,
            validate: true,
        };
        code(args_buffer(out, &args, parents_arr.as_slice()))
    }

    /// `git_commit_create_buffer`.
    ///
    /// # Safety
    ///
    /// `out` NULL or a valid `git_buf`; the rest as `git_commit_create`'s.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create_buffer(
        out: *mut GitBuf,
        repo: Option<&git_repository>,
        author: Option<&GitSignature>,
        committer: Option<&GitSignature>,
        message_encoding: *const c_char,
        message: *const c_char,
        tree: Option<&GitTree>,
        parent_count: usize,
        parents: *const *mut GitCommit,
    ) -> c_int {
        barrier(|| {
            let f = |s: &mut GitStr| {
                // SAFETY: this export's arguments, under its contract; `s`
                // is the string `wrap_buf` lends.
                ErrorCode::check_zero(unsafe {
                    create_buffer(
                        Some(s),
                        repo,
                        author,
                        committer,
                        message_encoding,
                        message,
                        tree,
                        parent_count,
                        parents,
                    )
                })
            };
            // SAFETY: `out` NULL or a valid `git_buf` (this export's
            // contract), which `f` does not reach.
            unsafe { wrap_buf(out, f) }
        })
    }

    /// `git_commit__create_buffer`.
    ///
    /// # Safety
    ///
    /// `out` NULL or a valid `git_str` that no other argument points into;
    /// the rest as `git_commit_create`'s.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit__create_buffer(
        out: Option<&mut GitStr>,
        repo: Option<&git_repository>,
        author: Option<&GitSignature>,
        committer: Option<&GitSignature>,
        message_encoding: *const c_char,
        message: *const c_char,
        tree: Option<&GitTree>,
        parent_count: usize,
        parents: *const *mut GitCommit,
    ) -> c_int {
        barrier(|| {
            // SAFETY: this export's arguments, under its contract.
            unsafe {
                create_buffer(
                    out,
                    repo,
                    author,
                    committer,
                    message_encoding,
                    message,
                    tree,
                    parent_count,
                    parents,
                )
            }
        })
    }

    /// `git_commit_create_with_signature`.
    ///
    /// # Safety
    ///
    /// `out` NULL or writable; `repo` NULL or live; `commit_content`,
    /// `signature`, `signature_field` NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create_with_signature(
        mut out: Out<'_, Oid>,
        repo: Option<&git_repository>,
        commit_content: CStrArg<'_>,
        signature: CStrArg<'_>,
        signature_field: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null(), "out", -1);
            let repo = git_arg!(repo, "repo", -1);
            let commit_content = git_arg!(commit_content.get(), "commit_content", -1);
            let content = commit_content.to_bytes();
            let sig = signature.get().map(CStr::to_bytes);
            let field = signature_field
                .get()
                .map(CStr::to_bytes)
                .unwrap_or(b"gpgsig");
            let oid_type: git_oid_t = repo.oid_type_raw();

            // The first step is to verify that all the tree and parents exist
            let parsed = alloc::calloc(1, size_of::<GitCommit>()).cast::<GitCommit>();
            if parsed.is_null() {
                return -1;
            }
            let r = (|| -> Result<()> {
                // SAFETY: a fresh zeroed `git__allocator` block of a
                // `GitCommit`'s size (8-aligned), all-zero being a valid
                // (empty) `GitCommit`; ours alone until freed below.
                let pc = unsafe { &mut *parsed };
                if parse_into(pc, content, &GitCommitParseOptions { oid_type, flags: 0 }).is_err() {
                    return Err(ErrorCode::GENERIC);
                }
                let pids = pc.parent_ids.as_slice();
                let mut next = |i: usize| pids.get(i).copied();
                let mut parents = GitArray::new();
                validate_tree_and_parents(
                    &mut parents,
                    Some(repo),
                    &pc.tree_id,
                    &mut next,
                    None,
                    true,
                )?;
                drop(parents);

                let mut signed = GitStr::new();
                let bytes = match sig {
                    Some(s) => {
                        append_header(&mut signed, content, field, s)?;
                        signed.as_bytes()
                    }
                    None => content,
                };
                let odb = crate::object::repo_odb_weak(repo)?;
                // SAFETY: `out` is non-NULL (checked above), so writable
                // (`Out`'s invariant), and written here only, by the odb;
                // the repository's live odb; `bytes` readable for its
                // length.
                ErrorCode::check(unsafe {
                    crate::odb::ffi::git_odb_write(
                        out.reborrow(),
                        Some(odb),
                        bytes.as_ptr().cast(),
                        bytes.len(),
                        OBJ_COMMIT,
                    )
                })
                .map(drop)
            })();
            // SAFETY: the commit allocated above (no reference to it left),
            // freed once.
            unsafe { git_commit__free(parsed.cast()) };
            code(r)
        })
    }

    /// `check_for_empty_commit`.
    ///
    /// # Safety
    ///
    /// `index` is a live index of `repo` when `tree` is `None`, not freed
    /// during the call.
    unsafe fn check_for_empty_commit(
        repo: &git_repository,
        tree: Option<&GitTree>,
        index: *mut pend::git_index,
    ) -> c_int {
        let mut head_tree: *mut c_void = ptr::null_mut();
        // SAFETY: a live repository; `head_tree` a writable local.
        let error = unsafe {
            crate::repository::ffi::git_repository_head_tree(Out::from(&mut head_tree), Some(repo))
        };
        let head_tree = head_tree.cast::<GitTree>();
        let r = if error != 0 && error != ErrorCode::UNBORN_BRANCH.raw() {
            error
        } else {
            let new_tree = tree.map_or(ptr::null_mut(), |t| ptr::from_ref(t).cast_mut());
            // SAFETY: a live repository; the head tree NULL or the one just
            // looked up; `new_tree` NULL or live (only read by the diff);
            // `index` live when `new_tree` is NULL (the contract).
            match unsafe {
                crate::sys::diff::count_deltas(repo_raw(Some(repo)), head_tree, new_tree, index)
            } {
                Err(e) => e,
                Ok(0) => {
                    let _ =
                        git_error_set!(ErrorClass::REPOSITORY, "no changes are staged for commit");
                    ErrorCode::UNCHANGED.raw()
                }
                Ok(_) => 0,
            }
        };
        // SAFETY: NULL or the tree looked up above, released once.
        unsafe { pend::git_tree_free(head_tree) };
        r
    }

    /// `init_ext_options`.
    fn ext_options(create: Option<&GitCommitCreateOptions>) -> GitCommitCreateExtOptions {
        let mut e = GitCommitCreateExtOptions::INIT;
        if let Some(c) = create {
            e.message_encoding = c.message_encoding;
            e.extra_headers = c.extra_headers;
            e.extra_headers_len = c.extra_headers_len;
            e.sign = c.sign;
            e.payload = c.payload;
        }
        e
    }

    /// `create_from_tree`.
    ///
    /// # Safety
    ///
    /// `opts` is valid: its signatures NULL or valid, its strings and
    /// headers NULL or valid, `sign` NULL or sound to call with `payload`;
    /// `message` and all of these unwritten during the call.
    unsafe fn create_from_tree(
        out: Out<'_, Oid>,
        repo: &git_repository,
        tree: &GitTree,
        message: CStrArg<'_>,
        opts: &GitCommitCreateOptions,
    ) -> c_int {
        let mut ext = ext_options(Some(opts));
        ext.update_ref = c"HEAD".as_ptr();
        let mut default_signature = ptr::null_mut();
        let mut parents = GitCommitArray {
            commits: ptr::null(),
            count: 0,
        };
        let error = (|| {
            // SAFETY: NULL or a valid signature (the contract), only read.
            let mut author = unsafe { opts.author.as_ref() };
            // SAFETY: as for the author.
            let mut committer = unsafe { opts.committer.as_ref() };
            if author.is_none() || committer.is_none() {
                // SAFETY: a live repository; a writable local out-parameter.
                if unsafe {
                    pend::git_signature_default(Out::from(&mut default_signature), Some(repo))
                } < 0
                {
                    return -1;
                }
                // SAFETY: the signature just made (non-NULL on success),
                // ours until freed below.
                let default = unsafe { default_signature.as_ref() };
                author = author.or(default);
                committer = committer.or(default);
            }
            // SAFETY: a live repository; `parents` a writable local.
            // (`l_commit::GitCommitArray` and the repository's are both
            // `git_commitarray`.)
            if unsafe {
                crate::repository::ffi::git_repository_commit_parents(
                    (&raw mut parents).cast(),
                    Some(repo),
                )
            } < 0
            {
                return -1;
            }
            // SAFETY: the parents just looked up (`count` live commits); the
            // options of the contract.
            unsafe {
                create_ext(
                    out,
                    Some(repo),
                    author,
                    committer,
                    message,
                    Some(tree),
                    parents.count,
                    parents.commits,
                    Some(&ext),
                )
            }
            .0
        })();
        // SAFETY: the array filled above (or empty), released once.
        unsafe { git_commitarray_dispose(Some(&mut parents)) };
        // SAFETY: NULL or the signature made above (no reference to it
        // left), freed once.
        unsafe { pend::git_signature_free(default_signature) };
        error
    }

    /// `git_commit_create_from_stage`.
    ///
    /// # Safety
    ///
    /// `out` NULL or writable; `repo` NULL or live; `message` NULL or
    /// NUL-terminated; `given_opts` NULL or valid (as `create_from_tree`'s).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create_from_stage(
        out: Out<'_, Oid>,
        repo: Option<&git_repository>,
        message: CStrArg<'_>,
        given_opts: Option<&GitCommitCreateOptions>,
    ) -> c_int {
        barrier(|| {
            let (Some(repo), false, false) = (repo, out.is_null(), message.is_null()) else {
                let _ = errors::assert_arg_failed("out && repo && message");
                return -1;
            };
            // A copy, as the C's memcpy.
            let opts = given_opts.copied().unwrap_or(GitCommitCreateOptions::INIT);
            let mut index = ptr::null_mut();
            let mut tree = ptr::null_mut();
            let error = (|| {
                // SAFETY: a live repository; a writable local.
                if unsafe {
                    crate::repository::ffi::git_repository_index(&raw mut index, Some(repo))
                } < 0
                {
                    return -1;
                }
                if !opts.allow_empty_commit() {
                    // SAFETY: the repository's index, just looked up.
                    let e = unsafe { check_for_empty_commit(repo, None, index) };
                    if e < 0 {
                        return e;
                    }
                }
                let mut tree_id = Oid::default();
                // SAFETY: the index looked up above; `tree_id` writable.
                let written =
                    unsafe { crate::index::ffi::git_index_write_tree(&raw mut tree_id, index) };
                if written < 0
                    // SAFETY: a writable local; a live repository; `tree_id` set.
                    || unsafe {
                        pend::git_tree_lookup(&raw mut tree, repo_raw(Some(repo)), Some(&tree_id))
                    } < 0
                {
                    return -1;
                }
                // SAFETY: the tree just looked up (non-NULL on success), ours
                // until freed below; only read.
                let Some(t) = (unsafe { tree.as_ref() }) else {
                    return -1;
                };
                // SAFETY: the options of this export's contract.
                unsafe { create_from_tree(out, repo, t, message, &opts) }
            })();
            // SAFETY: NULL or the tree looked up above, released once.
            unsafe { pend::git_tree_free(tree) };
            // SAFETY: NULL or the index looked up above, released once.
            unsafe { crate::index::ffi::git_index_free(index) };
            error
        })
    }

    /// `git_commit_create_from_tree`.
    ///
    /// # Safety
    ///
    /// `out` NULL or writable; `repo` NULL or live; `tree` NULL or a live
    /// tree; `message` NULL or NUL-terminated; `given_opts` NULL or valid
    /// (as `create_from_tree`'s).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create_from_tree(
        out: Out<'_, Oid>,
        repo: Option<&git_repository>,
        tree: Option<&GitTree>,
        message: CStrArg<'_>,
        given_opts: Option<&GitCommitCreateOptions>,
    ) -> c_int {
        barrier(|| {
            let (Some(repo), Some(tree), false, false) =
                (repo, tree, out.is_null(), message.is_null())
            else {
                let _ = errors::assert_arg_failed("out && repo && tree && message");
                return -1;
            };
            let opts = given_opts.copied().unwrap_or(GitCommitCreateOptions::INIT);
            if !opts.allow_empty_commit()
                // SAFETY: a tree is given: no index is needed.
                && unsafe { check_for_empty_commit(repo, Some(tree), ptr::null_mut()) } < 0
            {
                return -1;
            }
            // SAFETY: the options of this export's contract.
            unsafe { create_from_tree(out, repo, tree, message, &opts) }
        })
    }

    /// `git_commit_amend_from_stage`.
    ///
    /// # Safety
    ///
    /// As `git_commit_amend_from_tree`, without a tree.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_amend_from_stage(
        out: Out<'_, Oid>,
        repo: Option<&git_repository>,
        message: CStrArg<'_>,
        opts: Option<&GitCommitCreateOptions>,
    ) -> c_int {
        barrier(|| {
            let (Some(repo), false) = (repo, out.is_null()) else {
                let _ = errors::assert_arg_failed("out && repo");
                return -1;
            };
            let mut index = ptr::null_mut();
            let mut tree = ptr::null_mut();
            let mut tree_id = Oid::default();
            // A live repository and writable locals.
            // SAFETY: as above.
            let error = if unsafe { crate::repository::ffi::git_repository_index(&raw mut index, Some(repo)) } < 0
                // SAFETY: the index just looked up.
                || unsafe { crate::index::ffi::git_index_write_tree(&raw mut tree_id, index) } < 0
                // SAFETY: as above; `tree_id` set.
                || unsafe { pend::git_tree_lookup(&raw mut tree, repo_raw(Some(repo)), Some(&tree_id)) } < 0
            {
                -1
            } else {
                // SAFETY: the tree just looked up (non-NULL on success), ours
                // until freed below; only read.
                match unsafe { tree.as_ref() } {
                    // SAFETY: the options of this export's contract.
                    Some(t) => unsafe { amend_from_tree(out, repo, t, message, opts) },
                    None => -1,
                }
            };
            // SAFETY: NULL or the tree looked up above, released once.
            unsafe { pend::git_tree_free(tree) };
            // SAFETY: NULL or the index looked up above, released once.
            unsafe { crate::index::ffi::git_index_free(index) };
            error
        })
    }

    /// `git_commit_amend_from_tree` (without the barrier and the NULL
    /// checks).
    ///
    /// # Safety
    ///
    /// `given_message` and `opts` as [`git_commit_amend_from_tree`]'s.
    unsafe fn amend_from_tree(
        mut out: Out<'_, Oid>,
        repo: &git_repository,
        tree: &GitTree,
        given_message: CStrArg<'_>,
        opts: Option<&GitCommitCreateOptions>,
    ) -> c_int {
        let mut ext = ext_options(opts);
        let mut head_ref: *mut pend::git_reference = ptr::null_mut();
        let mut head_commit: *mut GitCommit = ptr::null_mut();
        let mut new_committer = ptr::null_mut();
        let error = (|| {
            // SAFETY: a live repository; a writable local.
            let mut e = unsafe {
                crate::repository::ffi::git_repository_head((&raw mut head_ref).cast(), Some(repo))
            };
            if e >= 0 {
                // SAFETY: the reference just looked up; a writable local.
                e = unsafe {
                    crate::refs::ffi::git_reference_peel(
                        ptr::addr_of_mut!(head_commit).cast(),
                        head_ref,
                        OBJ_COMMIT,
                    )
                };
            }
            if e < 0 {
                return e;
            }
            // SAFETY: the commit just peeled to (non-NULL on success), whose
            // reference we hold until it is released below.
            let Some(head) = (unsafe { CommitRef::from_raw(head_commit) }) else {
                return -1;
            };
            // SAFETY: NULL or a valid signature of the options (the
            // contract), only read.
            let author = match opts.and_then(|o| unsafe { o.author.as_ref() }) {
                Some(a) => Some(a),
                None => head.author_ref(),
            };
            // SAFETY: as for the author.
            let committer = match opts.and_then(|o| unsafe { o.committer.as_ref() }) {
                Some(c) => Some(c),
                None => {
                    // SAFETY: a live repository; a writable local.
                    let e = unsafe {
                        pend::git_signature_default(Out::from(&mut new_committer), Some(repo))
                    };
                    if e < 0 {
                        return e;
                    }
                    // SAFETY: the signature just made (non-NULL on success),
                    // ours until freed below.
                    unsafe { new_committer.as_ref() }
                }
            };
            let message = if given_message.is_null() {
                ext.message_encoding = head.message_encoding();
                head.message_arg()
            } else {
                given_message
            };
            let pids = head.parent_ids();
            let mut next = |i: usize| pids.get(i).copied();
            // SAFETY: the signatures are the options' (valid), the head
            // commit's (`CommitRef`) or the default one; the options'
            // strings, headers and callback as the contract states.
            let (e, written) = unsafe {
                create_from_c(
                    out.reborrow(),
                    Some(repo),
                    author,
                    committer,
                    message,
                    Some(tree_id_of(tree)),
                    &mut next,
                    Some(&ext),
                    false,
                )
            };
            if e != 0 {
                return e;
            }
            let Some(new_id) = written else { return -1 };
            // SAFETY: a live repository; the reference looked up; the new
            // id readable; the message NUL-terminated.
            unsafe {
                crate::refs::ffi::git_reference__update_for_commit(
                    repo_raw(Some(repo)),
                    head_ref,
                    CStrArg::NULL,
                    &raw const new_id,
                    c"commit".as_ptr(),
                )
            }
        })();
        // SAFETY: NULL or the signature made above (no reference to it
        // left), freed once.
        unsafe { pend::git_signature_free(new_committer) };
        // SAFETY: NULL or the commit looked up above (no `CommitRef` to it
        // left), released once.
        unsafe { pend::git_commit_free(head_commit) };
        // SAFETY: NULL or the reference looked up above, freed once.
        unsafe { crate::refs::ffi::git_reference_free(head_ref) };
        error
    }

    /// `git_commit_amend_from_tree`.
    ///
    /// # Safety
    ///
    /// `out` NULL or writable; `repo` NULL or live; `tree` NULL or a live
    /// tree; `given_message` NULL or NUL-terminated; `opts` NULL or valid
    /// (as `create_from_tree`'s).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_amend_from_tree(
        out: Out<'_, Oid>,
        repo: Option<&git_repository>,
        tree: Option<&GitTree>,
        given_message: CStrArg<'_>,
        opts: Option<&GitCommitCreateOptions>,
    ) -> c_int {
        barrier(|| {
            let (Some(repo), Some(tree), false) = (repo, tree, out.is_null()) else {
                let _ = errors::assert_arg_failed("out && repo && tree");
                return -1;
            };
            // SAFETY: this export's arguments, under its contract.
            unsafe { amend_from_tree(out, repo, tree, given_message, opts) }
        })
    }

    /// `git_commit_committer_with_mailmap`.
    ///
    /// # Safety
    ///
    /// `out` NULL or writable; `commit` NULL (C dereferences it: here an
    /// invalid argument) or a live commit; `mailmap` NULL or live.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_committer_with_mailmap(
        out: *mut *mut GitSignature,
        commit: Option<CommitRef<'_>>,
        mailmap: *const c_void,
    ) -> c_int {
        barrier(|| {
            let c = git_arg!(commit, "commit", -1);
            // SAFETY: `out` NULL or writable, `mailmap` NULL or live (this
            // export's contract); the commit's signature (NULL or live).
            unsafe { crate::sys::mailmap::resolve_signature(out, mailmap, c.committer()) }
        })
    }

    /// `git_commit_author_with_mailmap`.
    ///
    /// # Safety
    ///
    /// As [`git_commit_committer_with_mailmap`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_author_with_mailmap(
        out: *mut *mut GitSignature,
        commit: Option<CommitRef<'_>>,
        mailmap: *const c_void,
    ) -> c_int {
        barrier(|| {
            let c = git_arg!(commit, "commit", -1);
            // SAFETY: as in `git_commit_committer_with_mailmap`.
            unsafe { crate::sys::mailmap::resolve_signature(out, mailmap, c.author()) }
        })
    }

    /// `git_commitarray_dispose`.
    ///
    /// # Safety
    ///
    /// `array` NULL or a `git_commitarray` that libgit2 filled (`count`
    /// commit references in a `git__allocator` buffer), whose references
    /// nothing uses after.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commitarray_dispose(array: Option<&mut GitCommitArray>) {
        barrier(|| {
            let Some(a) = array else { return };
            for i in 0..a.count {
                // SAFETY: `i < count` readable commit pointers (the
                // contract), copied.
                let c = unsafe { nth(a.commits, i) };
                // SAFETY: a commit reference the array owns, released once.
                unsafe { pend::git_commit_free(c) };
            }
            // SAFETY: the array's `git__allocator` buffer (or NULL), freed
            // once.
            unsafe { alloc::free(a.commits.cast_mut().cast()) };
            *a = GitCommitArray {
                commits: ptr::null(),
                count: 0,
            };
        })
    }
}

/// `git_commit_message`'s pointer: into the raw message, past its leading
/// newlines (NULL without one).
///
/// # Safety
///
/// `c` is a live commit (as [`CommitRef::from_ptr`] requires).
pub(crate) unsafe fn message_ptr(c: *const GitCommit) -> *const c_char {
    // SAFETY: a live commit (the contract).
    unsafe { CommitRef::from_raw(c) }.map_or(ptr::null(), CommitRef::message_ptr)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]
// each block's SAFETY covers its sequence
mod tests;

crate::c_layout_test! {
    headers: ["commit.h", "cache.h", "grafts.h", "git2/index.h"],
    types: [
        GitCommit => "git_commit" {
            object, parent_ids, tree_id, author, committer, message_encoding,
            raw_message, raw_header, summary, body
        },
        GitTime => "git_time" { time, offset, sign },
        GitSignature => "git_signature" { name, email, when },
        GitCommitParseOptions => "git_commit__parse_options" { oid_type, flags },
        GitCommitHeader => "git_commit_header" { field, value },
        GitCommitCreateExtOptions => "git_commit_create_ext_options" {
            version, update_ref, message_encoding, extra_headers,
            extra_headers_len, sign, payload
        },
        GitCommitCreateOptions => "git_commit_create_options" {
            version, author, committer, message_encoding, extra_headers,
            extra_headers_len, sign, payload
        },
        crate::commit::deps::GitCommitArray => "git_commitarray" { commits, count },
    ],
    asserts: ["GIT_COMMIT_PARSE_QUICK == 1"],
}
