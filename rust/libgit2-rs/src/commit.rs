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
use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr;

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
    // SAFETY: per the contract.
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

/// The graft of `id` in the repository's grafts, then its shallow grafts.
///
/// # Safety
///
/// `repo` is a live repository whose grafts nobody changes during the call.
pub(crate) unsafe fn find_graft(
    repo: *mut git_repository,
    id: &Oid,
) -> Option<*mut GitCommitGraft> {
    if repo.is_null() {
        return None;
    }
    // SAFETY: `repo` is live (contract); `GitRepository` is `Sync`.
    let r = unsafe { &*repo };
    for grafts in [r.grafts_ptr(), r.shallow_grafts_ptr()] {
        let grafts = grafts.cast::<pend::git_grafts>();
        let mut g = ptr::null_mut();
        // SAFETY: `grafts` is the repository's (the C asserts it is not
        // NULL); `g` is writable.
        if unsafe { pend::git_grafts_get(&raw mut g, grafts, id) } == 0 {
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

/// [`header_field_cstr`] of the C string at `raw_header`, which is never
/// measured.
///
/// # Safety
///
/// `raw_header` points to a NUL-terminated string, live and unwritten
/// during the call.
pub(crate) unsafe fn header_field_raw(
    raw_header: *const c_char,
    field: &[u8],
    out: &mut GitStr,
) -> Result<()> {
    // SAFETY: the contract.
    match header_field_lines(unsafe { CLines::from_ptr(raw_header) }, field, out) {
        // As in `header_field_cstr`.
        Err(ErrorCode::NOT_FOUND) if field.contains(&b'\n') => {
            errors::clear();
            // SAFETY: the contract.
            let h = unsafe { CStr::from_ptr(raw_header) };
            header_field_in(h.to_bytes(), field, out)
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

/// `validate_tree_and_parents`: collect the parents, checking (when
/// `validate`) that the tree and the parents exist with the right types,
/// and that `current_id` (when given) is the first parent.
///
/// # Safety
///
/// `repo` is a live repository (used only when `validate`).
unsafe fn validate_tree_and_parents(
    parents: &mut GitArray<Oid>,
    repo: *mut git_repository,
    tree: &Oid,
    next: ParentSource<'_>,
    current_id: Option<Oid>,
    validate: bool,
) -> Result<()> {
    // SAFETY: `repo` is live (contract).
    if validate && !unsafe { pend::git_object__is_valid(repo, tree, OBJ_TREE) } {
        return Err(ErrorCode::GENERIC);
    }
    let mut i = 0;
    let r = (|| {
        while let Some(parent) = next(i) {
            if validate
                // SAFETY: `repo` is live (contract).
                && !unsafe { pend::git_object__is_valid(repo, &raw const parent, OBJ_COMMIT) }
            {
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

/// A signature pointer as [`SigOut`] (`None` for NULL, which the C
/// dereferences: see DEFECTS.md).
///
/// # Safety
///
/// `sig` is NULL or a live `git_signature` whose strings are NULL or
/// NUL-terminated, unchanged while the result lives.
unsafe fn sig_out<'a>(sig: *const GitSignature) -> Option<SigOut<'a>> {
    if sig.is_null() {
        return None;
    }
    // SAFETY: a live signature, unchanged while the result lives
    // (contract).
    let s = unsafe { &*sig };
    Some(SigOut {
        // SAFETY: NULL or NUL-terminated, live for `'a` (contract).
        name: unsafe { opt_cbytes(s.name) },
        // SAFETY: as above.
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
    // SAFETY: the caller's contract.
    unsafe { crate::util::export::cstr(p) }.map(CStr::to_bytes)
}

/// Extra headers, or the C expression of the `GIT_ASSERT_ARG` that a NULL
/// string fails in `format_header_field`.
pub(crate) type Headers<'a> = core::result::Result<Vec<(&'a [u8], &'a [u8])>, &'static str>;

/// The extra headers of an options struct.
///
/// # Safety
///
/// `headers` is NULL or `len` readable `git_commit_header`s whose strings
/// are NULL or NUL-terminated, live for `'a`.
unsafe fn extra_headers<'a>(headers: *const GitCommitHeader, len: usize) -> Result<Headers<'a>> {
    if len == 0 {
        return Ok(Ok(Vec::new()));
    }
    let mut v = Vec::new();
    v.try_reserve(len).map_err(|_| errors::oom())?;
    for i in 0..len {
        // SAFETY: `i < len` headers are readable (contract).
        let h = unsafe { nth(headers, i) };
        // SAFETY: NUL-terminated or NULL, live for `'a` (contract).
        let f = unsafe { opt_cbytes(h.field) };
        // SAFETY: as above.
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

/// `git_commit__create_internal`.
///
/// # Safety
///
/// `repo` is a live repository; `next` and `sign` may call C callbacks
/// whose contracts the caller guarantees.
pub(crate) unsafe fn create_internal(
    id: &mut Oid,
    repo: *mut git_repository,
    args: &CreateArgs<'_>,
    next: ParentSource<'_>,
    sign: Option<Signer<'_>>,
) -> Result<()> {
    let mut r#ref: *mut pend::git_reference = ptr::null_mut();
    let update_ref = CStrArg::from(args.update_ref);
    if !update_ref.is_null() {
        // SAFETY: `repo` is live; `update_ref` NUL-terminated.
        let e = unsafe {
            crate::refs::ffi::git_reference_lookup_resolved(
                &raw mut r#ref,
                repo,
                update_ref.as_ptr(),
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
        // SAFETY: the reference just looked up.
        let t = unsafe { crate::refs::ffi::git_reference_target(r#ref) };
        // SAFETY: NULL or the reference's id.
        (!t.is_null()).then(|| unsafe { t.read() })
    };

    let mut parents = GitArray::new();
    let mut buf = GitStr::new();
    let r = (|| -> Result<()> {
        // SAFETY: `repo` is live (contract).
        unsafe {
            validate_tree_and_parents(
                &mut parents,
                repo,
                &args.tree,
                next,
                current_id,
                args.validate,
            )
        }?;
        args_buffer(&mut buf, args, parents.as_slice())?;

        if let Some(sign) = sign {
            let e = sign(&mut buf);
            if e < 0 {
                return Err(ec(e));
            }
        }

        // SAFETY: `repo` is live (contract).
        let odb = unsafe { repo_odb(repo) }?;
        // SAFETY: `odb` is the repository's; the tree id readable.
        ErrorCode::check(unsafe { crate::odb::ffi::git_odb__freshen(odb, &raw const args.tree) })?;
        let bytes = buf.as_bytes();
        // SAFETY: `bytes` are readable for its length; `id` writable.
        ErrorCode::check(unsafe {
            crate::odb::ffi::git_odb_write(id, odb, bytes.as_ptr().cast(), bytes.len(), OBJ_COMMIT)
        })?;

        if !update_ref.is_null() {
            // SAFETY: `ref` is NULL or the looked-up reference; the strings
            // are NUL-terminated.
            ErrorCode::check_zero(unsafe {
                crate::refs::ffi::git_reference__update_for_commit(
                    repo,
                    r#ref,
                    update_ref,
                    id,
                    c"commit".as_ptr(),
                )
            })?;
        }
        Ok(())
    })();
    // SAFETY: NULL or the reference looked up above, freed once.
    unsafe { crate::refs::ffi::git_reference_free(r#ref) };
    r
}

/// `git_repository_odb__weakptr`.
///
/// # Safety
///
/// `repo` is a live repository, not freed during the call.
pub(crate) unsafe fn repo_odb(repo: *mut git_repository) -> Result<*mut pend::git_odb> {
    // SAFETY: `repo` is a live repository (every caller's contract), which
    // is `Sync`.
    let repo = unsafe { &*repo };
    repo.odb_weak().map(core::ptr::NonNull::as_ptr)
}

/// `git_object_owner` of a commit or tree pointer (asserting, as C's, that
/// it is not NULL).
///
/// # Safety
///
/// `obj` is NULL or a live object.
pub(crate) unsafe fn object_owner<T>(obj: *const T) -> Result<*mut git_repository> {
    if obj.is_null() {
        return Err(errors::assert_arg_failed("obj"));
    }
    // SAFETY: a live object starts with its `git_object` head.
    let p = unsafe { &raw const (*obj.cast::<GitObjectHead>()).repo };
    // SAFETY: `repo` is set before the object is published and never
    // changed.
    Ok(unsafe { p.read() })
}

/// `git_object_id` of a live object (NULL asserted as C's).
///
/// # Safety
///
/// As [`object_owner`].
pub(crate) unsafe fn object_id<T>(obj: *const T) -> Result<Oid> {
    if obj.is_null() {
        return Err(errors::assert_arg_failed("obj"));
    }
    // SAFETY: a live object starts with its `git_object` head.
    let p = unsafe { &raw const (*obj.cast::<GitObjectHead>()).cached.oid };
    // SAFETY: the id is set before publication and never changed.
    Ok(unsafe { p.read() })
}

/// The parents of a commit (as `commit_parent_for_amend`).
///
/// # Safety
///
/// `commit` is a live commit whose parents nobody changes meanwhile.
unsafe fn parent_ids_of<'a>(commit: *const GitCommit) -> &'a [Oid] {
    // SAFETY: a live commit (contract).
    let a = unsafe { &raw const (*commit).parent_ids };
    // SAFETY: only that field is borrowed: the parse wrote the array before
    // the commit was published; nothing writes it after.
    unsafe { &*a }.as_slice()
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
    use crate::git_arg;
    use crate::util::cabi::CStrArg;

    use super::*;
    use crate::buf::GitBuf;
    use crate::buf::ffi::{git_buf_fromstr, git_buf_tostr};
    use crate::commit::deps::GitCommitArray;
    use crate::util::export::{barrier, cstr_bytes};

    /// `GIT_BUF_WRAP_PRIVATE(buf, fn, …)`.
    ///
    /// # Safety
    ///
    /// `buf` is NULL or a valid `git_buf`.
    pub(crate) unsafe fn wrap_buf(
        buf: *mut GitBuf,
        f: impl FnOnce(&mut GitStr) -> Result<()>,
    ) -> c_int {
        // SAFETY: `buf` is NULL or a valid `git_buf` (contract), which `f`
        // does not reach.
        unsafe { crate::buf::wrap_private(buf, f) }
    }

    /// `git_commit__free`.
    ///
    /// # Safety
    ///
    /// `commit` is a commit allocated with `git__allocator` (zeroed, then
    /// parsed or not), which nothing uses after.
    #[unsafe(no_mangle)]
    #[allow(clippy::multiple_unsafe_ops_per_block)] // the frees of the commit's own allocations, each once
    pub unsafe extern "C" fn git_commit__free(commit: *mut c_void) {
        barrier(|| {
            let c = commit.cast::<GitCommit>();
            if c.is_null() {
                return;
            }
            // SAFETY: the caller hands over the commit: it is ours alone.
            let cm = unsafe { &mut *c };
            cm.parent_ids.dispose();
            // SAFETY: the commit's own allocations, freed once.
            unsafe {
                free_signature(cm.author);
                free_signature(cm.committer);
                alloc::free(cm.raw_header.cast());
                alloc::free(cm.raw_message.cast());
                alloc::free(cm.message_encoding.cast());
                alloc::free(cm.summary.cast());
                alloc::free(cm.body.cast());
                alloc::free(commit);
            }
        })
    }

    /// `git_commit_create_ext_options_init`.
    ///
    /// # Safety
    ///
    /// `opts` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create_ext_options_init(
        opts: *mut GitCommitCreateExtOptions,
        version: c_uint,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!opts.is_null(), "opts", -1);
            let mut v = GitCommitCreateExtOptions::INIT;
            v.version = version;
            // SAFETY: writable (contract).
            unsafe { opts.write(v) };
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
        opts: *mut GitCommitCreateOptions,
        version: c_uint,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!opts.is_null(), "opts", -1);
            let mut v = GitCommitCreateOptions::INIT;
            v.version = version;
            // SAFETY: writable (contract).
            unsafe { opts.write(v) };
            0
        })
    }

    /// `git_commit_create_from_callback`.
    ///
    /// # Safety
    ///
    /// The C's: `id` writable; `repo` live; the signatures live; strings
    /// NUL-terminated or NULL; `tree` readable; `parent_cb` callable with
    /// `parent_payload`, returning NULL or a readable id.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create_from_callback(
        id: *mut Oid,
        repo: *mut git_repository,
        update_ref: *const c_char,
        author: *const GitSignature,
        committer: *const GitSignature,
        message_encoding: *const c_char,
        message: *const c_char,
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
                // SAFETY: the callback's contract (the caller's).
                let p = unsafe { cb(i, parent_payload) };
                // SAFETY: NULL or a readable id.
                (!p.is_null()).then(|| unsafe { p.read() })
            };
            // SAFETY: the caller's contract.
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
        })
    }

    /// [`create_internal`] with C's out-parameter and tree pointer.
    ///
    /// # Safety
    ///
    /// As [`create_internal`]; `id` writable, `tree` readable (both may be
    /// NULL, which crashes the C: here an error).
    #[allow(clippy::too_many_arguments)]
    unsafe fn create_from_c(
        id: *mut Oid,
        repo: *mut git_repository,
        author: *const GitSignature,
        committer: *const GitSignature,
        message: *const c_char,
        tree: *const Oid,
        next: ParentSource<'_>,
        opts: Option<&GitCommitCreateExtOptions>,
        validate: bool,
    ) -> c_int {
        if id.is_null() {
            return errors::assert_arg_failed("id").raw();
        }
        if tree.is_null() {
            return errors::assert_arg_failed("tree").raw();
        }
        // The caller's contract: the strings, signatures and headers are
        // live and unchanged during the call.
        let headers = match opts {
            // SAFETY: the options' headers are readable (contract).
            Some(o) => match unsafe { extra_headers(o.extra_headers, o.extra_headers_len) } {
                Ok(h) => h,
                Err(e) => return e.raw(),
            },
            None => Ok(Vec::new()),
        };
        let args = CreateArgs {
            // SAFETY: NULL or a live signature (contract).
            author: unsafe { sig_out(author) },
            // SAFETY: as above.
            committer: unsafe { sig_out(committer) },
            // SAFETY: NULL or NUL-terminated (contract).
            message: unsafe { opt_cbytes(message) },
            // SAFETY: readable, checked non-NULL above.
            tree: unsafe { tree.read() },
            // SAFETY: NULL or NUL-terminated (contract).
            encoding: opts.and_then(|o| unsafe { opt_cbytes(o.message_encoding) }),
            headers,
            // SAFETY: as above.
            update_ref: opts.and_then(|o| unsafe { crate::util::export::cstr(o.update_ref) }),
            validate,
        };
        let mut sign_c;
        let sign: Option<Signer<'_>> = match opts.and_then(|o| o.sign.map(|f| (f, o.payload))) {
            Some((f, payload)) => {
                sign_c = move |buf: &mut GitStr| {
                    let content = buf.as_ptr();
                    let mut builder = GitCommitbuilder { contents: buf };
                    // SAFETY: the callback's contract (the caller's); the
                    // builder and the content are valid for the call.
                    unsafe { f(&raw mut builder, repo, content, payload) }
                };
                Some(&mut sign_c)
            }
            None => None,
        };
        let mut out = Oid::default();
        // SAFETY: the caller's contract.
        let r = unsafe { create_internal(&mut out, repo, &args, next, sign) };
        // The C writes `id` with git_odb_write before updating the ref.
        if r.is_ok() || out != Oid::default() {
            // SAFETY: writable (contract).
            unsafe { id.write(out) };
        }
        code(r)
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
        id: *mut Oid,
        repo: *mut git_repository,
        update_ref: *const c_char,
        author: *const GitSignature,
        committer: *const GitSignature,
        message_encoding: *const c_char,
        message: *const c_char,
        tree: *const crate::tree::GitTree,
        parent_count: usize,
        parents: *const *const GitCommit,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!tree.is_null(), "tree", -1);
            // SAFETY: a live tree (contract).
            let owner = unsafe { object_owner(tree) };
            git_assert_arg!(owner == Ok(repo), "git_tree_owner(tree) == repo", -1);
            let mut opts = GitCommitCreateExtOptions::INIT;
            opts.update_ref = update_ref;
            opts.message_encoding = message_encoding;
            let mut next = |i: usize| -> Option<Oid> {
                if i >= parent_count {
                    return None;
                }
                // SAFETY: `i < parent_count` pointers are readable.
                let c = unsafe { nth(parents, i) };
                if c.is_null() {
                    return None;
                }
                // SAFETY: a live commit (contract).
                unsafe { object_id(c) }.ok()
            };
            // SAFETY: a live tree; the rest is the caller's contract.
            let tree_id = unsafe { object_id(tree) }.unwrap_or_default();
            // SAFETY: the caller's contract.
            unsafe {
                create_from_c(
                    id,
                    repo,
                    author,
                    committer,
                    message,
                    &raw const tree_id,
                    &mut next,
                    Some(&opts),
                    false,
                )
            }
        })
    }

    /// `git_commit_create_from_ids`.
    ///
    /// # Safety
    ///
    /// As `git_commit_create_from_callback`; `parents` holds `parent_count`
    /// readable ids.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create_from_ids(
        id: *mut Oid,
        repo: *mut git_repository,
        update_ref: *const c_char,
        author: *const GitSignature,
        committer: *const GitSignature,
        message_encoding: *const c_char,
        message: *const c_char,
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
                // SAFETY: `i < parent_count` entries are readable.
                let p = unsafe { nth(parents, i) };
                // SAFETY: NULL or a readable id.
                (!p.is_null()).then(|| unsafe { p.read() })
            };
            // SAFETY: the caller's contract.
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
        })
    }

    /// `commit_parent_from_array`: the parents in `parents` that belong to
    /// `repo` (the first that does not ends the list).
    ///
    /// # Safety
    ///
    /// `parents` holds `n` readable pointers to live commits (or NULL).
    unsafe fn from_array<'a>(
        parents: *const *mut GitCommit,
        n: usize,
        repo: *mut git_repository,
    ) -> impl FnMut(usize) -> Option<Oid> + 'a {
        move |i| {
            if i >= n {
                return None;
            }
            // SAFETY: `i < n` pointers are readable (contract).
            let c = unsafe { nth(parents, i) };
            // SAFETY: NULL or a live commit (contract).
            if unsafe { object_owner(c) }.ok()? != repo {
                return None;
            }
            // SAFETY: as above.
            unsafe { object_id(c) }.ok()
        }
    }

    /// `git_commit_create_ext`.
    ///
    /// # Safety
    ///
    /// As `git_commit_create`; `opts` NULL or a valid options struct.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create_ext(
        id: *mut Oid,
        repo: *mut git_repository,
        author: *const GitSignature,
        committer: *const GitSignature,
        message: *const c_char,
        tree: *const crate::tree::GitTree,
        parent_count: usize,
        parents: *const *mut GitCommit,
        opts: *const GitCommitCreateExtOptions,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!tree.is_null(), "tree", -1);
            // SAFETY: a live tree (contract).
            let owner = unsafe { object_owner(tree) };
            git_assert_arg!(owner == Ok(repo), "git_tree_owner(tree) == repo", -1);
            // SAFETY: the caller's contract.
            let mut next = unsafe { from_array(parents, parent_count, repo) };
            // SAFETY: a live tree.
            let tree_id = unsafe { object_id(tree) }.unwrap_or_default();
            // SAFETY: NULL or a valid options struct, only read.
            let opts = unsafe { opts.as_ref() };
            // SAFETY: the caller's contract.
            unsafe {
                create_from_c(
                    id,
                    repo,
                    author,
                    committer,
                    message,
                    &raw const tree_id,
                    &mut next,
                    opts,
                    false,
                )
            }
        })
    }

    /// `git_commitbuilder_add_header`.
    ///
    /// # Safety
    ///
    /// `builder` is the builder a signing callback was given, during that
    /// callback; `field` and `value` are NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commitbuilder_add_header(
        builder: *mut GitCommitbuilder,
        field: *const c_char,
        value: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            if builder.is_null() {
                return errors::assert_arg_failed("builder").raw();
            }
            // SAFETY: the builder a signing callback was given (contract).
            let contents = unsafe { (*builder).contents };
            // SAFETY: the builder's string is live during the callback, and
            // only this call uses it meanwhile.
            let contents = unsafe { &mut *contents };
            // SAFETY: NUL-terminated or NULL (contract).
            let f = unsafe { opt_cbytes(field) };
            let v = value.get().map(CStr::to_bytes);
            let Some(f) = f else {
                return errors::assert_arg_failed("field").raw();
            };
            let Some(v) = v else {
                return errors::assert_arg_failed("content").raw();
            };
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
    /// The C's: `id` writable; `repo` live; `tree` a live tree of `repo`;
    /// `parents` holds `parent_count` live commits; the signatures live;
    /// strings NUL-terminated or NULL.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create(
        id: *mut Oid,
        repo: *mut git_repository,
        update_ref: *const c_char,
        author: *const GitSignature,
        committer: *const GitSignature,
        message_encoding: *const c_char,
        message: *const c_char,
        tree: *const crate::tree::GitTree,
        parent_count: usize,
        parents: *const *mut GitCommit,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!tree.is_null(), "tree", -1);
            // SAFETY: a live tree (contract).
            let owner = unsafe { object_owner(tree) };
            git_assert_arg!(owner == Ok(repo), "git_tree_owner(tree) == repo", -1);
            let mut opts = GitCommitCreateExtOptions::INIT;
            opts.update_ref = update_ref;
            opts.message_encoding = message_encoding;
            // SAFETY: the caller's contract.
            let mut next = unsafe { from_array(parents, parent_count, repo) };
            // SAFETY: a live tree.
            let tree_id = unsafe { object_id(tree) }.unwrap_or_default();
            // SAFETY: the caller's contract.
            unsafe {
                create_from_c(
                    id,
                    repo,
                    author,
                    committer,
                    message,
                    &raw const tree_id,
                    &mut next,
                    Some(&opts),
                    false,
                )
            }
        })
    }

    /// `git_commit_amend`.
    ///
    /// # Safety
    ///
    /// `id` writable; `commit_to_amend` a live commit; the rest NULL or as
    /// `git_commit_create`'s.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_amend(
        id: *mut Oid,
        commit_to_amend: *const GitCommit,
        update_ref: *const c_char,
        author: *const GitSignature,
        committer: *const GitSignature,
        message_encoding: *const c_char,
        message: *const c_char,
        tree: *const crate::tree::GitTree,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!id.is_null(), "id", -1);
            git_assert_arg!(!commit_to_amend.is_null(), "commit_to_amend", -1);
            let c = commit_to_amend;
            // A live commit (contract): its fields, set by the parse and not
            // changed after, are read through the pointer.
            // SAFETY: as above.
            let repo = unsafe { object_owner(c) }.unwrap_or(ptr::null_mut());
            let author = if author.is_null() {
                // SAFETY: as above.
                unsafe { (*c).author }.cast_const()
            } else {
                author
            };
            let committer = if committer.is_null() {
                // SAFETY: as above.
                unsafe { (*c).committer }.cast_const()
            } else {
                committer
            };
            let message_encoding = if message_encoding.is_null() {
                // SAFETY: as above.
                unsafe { (*c).message_encoding }.cast_const()
            } else {
                message_encoding
            };
            let message = if message.is_null() {
                // SAFETY: as above.
                unsafe { message_ptr(c) }
            } else {
                message
            };
            let tree_id = if tree.is_null() {
                let mut old: *mut crate::tree::GitTree = ptr::null_mut();
                // SAFETY: `old` is writable; the commit is live.
                let e = unsafe { ffi::git_commit_tree(&raw mut old, c) };
                if e < 0 {
                    return e;
                }
                // SAFETY: the tree just looked up.
                let t = unsafe { object_id(old) }.unwrap_or_default();
                // SAFETY: freed once.
                unsafe { pend::git_tree_free(old) };
                t
            } else {
                // SAFETY: a live tree (contract).
                let owner = unsafe { object_owner(tree) };
                git_assert_arg!(owner == Ok(repo), "git_tree_owner(tree) == repo", -1);
                // SAFETY: as above.
                unsafe { object_id(tree) }.unwrap_or_default()
            };

            let mut r#ref = ptr::null_mut();
            if !update_ref.is_null() {
                // SAFETY: `repo` is live; `update_ref` NUL-terminated.
                let e = unsafe {
                    crate::refs::ffi::git_reference_lookup_resolved(
                        &raw mut r#ref,
                        repo,
                        update_ref,
                        5,
                    )
                };
                if e < 0 {
                    return e;
                }
                // SAFETY: the reference just looked up.
                let target = unsafe { crate::refs::ffi::git_reference_target(r#ref) };
                // SAFETY: the commit is live.
                let cid = unsafe { object_id(c) }.unwrap_or_default();
                // SAFETY: NULL or the reference's id.
                if target.is_null() || unsafe { target.read() } != cid {
                    // SAFETY: freed once.
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
            // SAFETY: the commit is live and its parents unchanged.
            let pids = unsafe { parent_ids_of(c) };
            let mut next = |i: usize| pids.get(i).copied();
            // SAFETY: the caller's contract.
            let mut error = unsafe {
                create_from_c(
                    id,
                    repo,
                    author,
                    committer,
                    message,
                    &raw const tree_id,
                    &mut next,
                    Some(&opts),
                    false,
                )
            };
            if error == 0 && !update_ref.is_null() {
                // SAFETY: the reference looked up above; `id` was written.
                error = unsafe {
                    crate::refs::ffi::git_reference__update_for_commit(
                        repo,
                        r#ref,
                        CStrArg::NULL,
                        id,
                        c"commit".as_ptr(),
                    )
                };
            }
            if !update_ref.is_null() {
                // SAFETY: freed once.
                unsafe { crate::refs::ffi::git_reference_free(r#ref) };
            }
            error
        })
    }

    /// `git_commit__parse`.
    ///
    /// # Safety
    ///
    /// `commit` is a fresh (zeroed) commit that only the caller holds;
    /// `obj` a live odb object.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit__parse(
        commit: *mut c_void,
        obj: *mut pend::git_odb_object,
        oid_type: git_oid_t,
    ) -> c_int {
        let mut opts = GitCommitParseOptions { oid_type, flags: 0 };
        // SAFETY: the caller's contract.
        unsafe { git_commit__parse_ext(commit.cast(), obj, &raw mut opts) }
    }

    /// `git_commit__parse_raw`.
    ///
    /// # Safety
    ///
    /// `commit` is a fresh (zeroed) commit that only the caller holds;
    /// `data` is readable for `size` bytes.
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
            // SAFETY: an exclusive, initialized commit (contract).
            let c = unsafe { &mut *commit.cast::<GitCommit>() };
            // SAFETY: `size` readable bytes (contract).
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
    /// `commit` is a zeroed commit that only the caller holds, whose
    /// `object.repo` is NULL or a live repository; `odb_obj` a live odb
    /// object; `parse_opts` readable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit__parse_ext(
        commit: *mut GitCommit,
        odb_obj: *mut pend::git_odb_object,
        parse_opts: *mut GitCommitParseOptions,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!commit.is_null(), "commit", -1);
            git_assert_arg!(!parse_opts.is_null(), "opts", -1);
            // SAFETY: a live odb object (contract).
            let p = unsafe { crate::odb::ffi::git_odb_object_data(odb_obj) }.cast::<c_char>();
            // SAFETY: as above.
            let n = unsafe { crate::odb::ffi::git_odb_object_size(odb_obj) };
            if p.is_null() {
                return errors::assert_arg_failed("data").raw();
            }
            // SAFETY: exclusive access to the commit (contract).
            let c = unsafe { &mut *commit };
            // SAFETY: readable options (contract).
            let opts = unsafe { parse_opts.read() };
            // SAFETY: the object's data is readable for its size, unchanged
            // during the call.
            let data = unsafe { crate::util::export::bytes(p, n) };
            // SAFETY: the object's id, live with it.
            let id = unsafe { crate::odb::ffi::git_odb_object_id(odb_obj) };
            // SAFETY: as above.
            let id = unsafe { id.read() };
            if let Err(e) = parse_into(c, data, &opts) {
                return e.raw();
            }
            // Perform necessary grafts
            // SAFETY: the repository is live (contract).
            if let Some(g) = unsafe { find_graft(c.object.repo, &id) } {
                // SAFETY: the graft is the repository's, live meanwhile.
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
            #[allow(clippy::multiple_unsafe_ops_per_block)] // reads of fields of one live commit
            pub unsafe extern "C" fn $name(commit: *const GitCommit) -> $ret {
                barrier(|| {
                    git_assert_arg!(!commit.is_null(), "commit", $invalid);
                    let $c = commit;
                    // SAFETY: a live commit (contract), whose fields the parse
                    // set before it was published; read through the pointer.
                    unsafe { $body }
                })
            }
        };
    }

    getter!(
        /// `git_commit_author`.
        git_commit_author -> *const GitSignature, ptr::null(), |c| (*c).author.cast_const()
    );
    getter!(
        /// `git_commit_committer`.
        git_commit_committer -> *const GitSignature, ptr::null(), |c| (*c).committer.cast_const()
    );
    getter!(
        /// `git_commit_message_raw`.
        git_commit_message_raw -> *const c_char, ptr::null(), |c| (*c).raw_message.cast_const()
    );
    getter!(
        /// `git_commit_message_encoding`.
        git_commit_message_encoding -> *const c_char, ptr::null(), |c| (*c).message_encoding.cast_const()
    );
    getter!(
        /// `git_commit_raw_header`.
        git_commit_raw_header -> *const c_char, ptr::null(), |c| (*c).raw_header.cast_const()
    );
    getter!(
        /// `git_commit_time` (`INT64_MIN` without a committer, which the C
        /// dereferences).
        git_commit_time -> i64, i64::MIN, |c| {
            let s = (*c).committer;
            if s.is_null() { i64::MIN } else { (*s).when.time }
        }
    );
    getter!(
        /// `git_commit_time_offset`.
        git_commit_time_offset -> c_int, -1, |c| {
            let s = (*c).committer;
            if s.is_null() { -1 } else { (*s).when.offset }
        }
    );
    getter!(
        /// `git_commit_parentcount`.
        git_commit_parentcount -> c_uint, 0, |c| parent_ids_of(c).len() as c_uint
    );
    getter!(
        /// `git_commit_tree_id`.
        git_commit_tree_id -> *const Oid, ptr::null(), |c| ptr::addr_of!((*c).tree_id)
    );
    getter!(
        /// `git_commit_message`: the raw message without its leading
        /// newlines.
        git_commit_message -> *const c_char, ptr::null(), |c| message_ptr(c)
    );

    /// `git_commit_summary`: computed on first use and kept in the commit.
    ///
    /// # Safety
    ///
    /// `commit` is NULL or a live commit, and no other thread calls this
    /// (or `git_commit__free`) on it meanwhile (as the C's).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_summary(commit: *mut GitCommit) -> *const c_char {
        barrier(|| {
            git_assert_arg!(!commit.is_null(), "commit", ptr::null());
            // SAFETY: a live commit (contract).
            let slot = unsafe { &raw mut (*commit).summary };
            // SAFETY: `summary` is written only here (and by the free), by
            // the contract not concurrently.
            if unsafe { slot.read() }.is_null() {
                // SAFETY: a live commit.
                let msg = unsafe { message_ptr(commit) };
                let mut s = GitStr::new();
                if !msg.is_null() {
                    // SAFETY: the commit's NUL-terminated message, unchanged
                    // during the call.
                    let m = unsafe { CStr::from_ptr(msg) };
                    let _ = summary_into(m.to_bytes(), &mut s);
                }
                let mut p = s.detach().map_or(ptr::null_mut(), |d| d.into_raw());
                if p.is_null() {
                    p = alloc::strdup(c"");
                }
                // SAFETY: as above.
                unsafe { slot.write(p) };
            }
            // SAFETY: as above.
            unsafe { slot.read() }.cast_const()
        })
    }

    /// `git_commit_body`: computed on first use and kept in the commit.
    ///
    /// # Safety
    ///
    /// As [`git_commit_summary`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_body(commit: *mut GitCommit) -> *const c_char {
        barrier(|| {
            git_assert_arg!(!commit.is_null(), "commit", ptr::null());
            // SAFETY: a live commit (contract).
            let slot = unsafe { &raw mut (*commit).body };
            // SAFETY: `body` is written only here (and by the free), by the
            // contract not concurrently.
            if unsafe { slot.read() }.is_null() {
                // SAFETY: a live commit.
                let msg = unsafe { message_ptr(commit) };
                if !msg.is_null() {
                    // SAFETY: the commit's NUL-terminated message, unchanged
                    // during the call.
                    let m = unsafe { CStr::from_ptr(msg) };
                    if let Some(b) = body_of(m.to_bytes()) {
                        // SAFETY: as above.
                        unsafe { slot.write(alloc::strndup(b, b.len())) };
                    }
                }
            }
            // SAFETY: as above.
            unsafe { slot.read() }.cast_const()
        })
    }

    /// `git_commit_tree`.
    ///
    /// # Safety
    ///
    /// `tree_out` writable; `commit` NULL or a live commit.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_tree(
        tree_out: *mut *mut crate::tree::GitTree,
        commit: *const GitCommit,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!commit.is_null(), "commit", -1);
            // SAFETY: a live commit (contract): its fields set by the parse.
            let repo = unsafe { (*commit).object.repo };
            // SAFETY: as above.
            let tree_id = unsafe { &raw const (*commit).tree_id };
            // SAFETY: `tree_out` writable; the repository is live while the
            // commit is.
            unsafe { pend::git_tree_lookup(tree_out, repo, tree_id) }
        })
    }

    /// `git_commit_parent_id`.
    ///
    /// # Safety
    ///
    /// `commit` is NULL or a live commit.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_parent_id(
        commit: *const GitCommit,
        n: c_uint,
    ) -> *const Oid {
        barrier(|| {
            git_assert_arg!(!commit.is_null(), "commit", ptr::null());
            // SAFETY: a live commit (contract).
            unsafe { parent_ids_of(commit) }
                .get(n as usize)
                .map_or(ptr::null(), ptr::from_ref)
        })
    }

    /// `git_commit_parent`.
    ///
    /// # Safety
    ///
    /// `parent` writable; `commit` NULL or a live commit.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_parent(
        parent: *mut *mut GitCommit,
        commit: *const GitCommit,
        n: c_uint,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!commit.is_null(), "commit", -1);
            // SAFETY: a live commit (contract).
            let Some(id) = unsafe { parent_ids_of(commit) }.get(n as usize).copied() else {
                let _ = git_error_set!(ErrorClass::INVALID, "parent {} does not exist", n);
                return ErrorCode::NOT_FOUND.raw();
            };
            // SAFETY: a live commit (contract); `repo` set by its parse.
            let repo = unsafe { (*commit).object.repo };
            // SAFETY: `parent` writable; the repository live with the commit.
            unsafe { pend::git_commit_lookup(parent, repo, &raw const id) }
        })
    }

    /// `git_commit_nth_gen_ancestor`.
    ///
    /// # Safety
    ///
    /// `ancestor` NULL or writable; `commit` NULL or a live commit.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_nth_gen_ancestor(
        ancestor: *mut *mut GitCommit,
        commit: *const GitCommit,
        n: c_uint,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!ancestor.is_null(), "ancestor", -1);
            git_assert_arg!(!commit.is_null(), "commit", -1);
            let mut current = ptr::null_mut();
            // SAFETY: a live commit; `current` writable.
            if unsafe { pend::git_commit_dup(&raw mut current, commit.cast_mut()) } < 0 {
                return -1;
            }
            if n == 0 {
                // SAFETY: writable (contract).
                unsafe { ancestor.write(current) };
                return 0;
            }
            let mut parent = ptr::null_mut();
            for _ in 0..n {
                // SAFETY: `current` is a commit we hold.
                let error = unsafe { git_commit_parent(&raw mut parent, current, 0) };
                // SAFETY: released once.
                unsafe { pend::git_commit_free(current) };
                if error < 0 {
                    return error;
                }
                current = parent;
            }
            // SAFETY: writable (contract).
            unsafe { ancestor.write(parent) };
            0
        })
    }

    /// `git_commit_header_field`.
    ///
    /// # Safety
    ///
    /// `out` a valid `git_buf`; `commit` a live commit; `field`
    /// NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_header_field(
        out: *mut GitBuf,
        commit: *const GitCommit,
        field: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: a live commit; `field` NUL-terminated (contract).
            let f = |s: &mut GitStr| unsafe { header_field_of(s, commit, field) };
            // SAFETY: `out` a valid `git_buf` (contract).
            unsafe { wrap_buf(out, f) }
        })
    }

    /// The header field of a commit for the exports.
    ///
    /// # Safety
    ///
    /// `commit` a live commit; `field` NUL-terminated.
    unsafe fn header_field_of(
        out: &mut GitStr,
        commit: *const GitCommit,
        field: *const c_char,
    ) -> Result<()> {
        if commit.is_null() {
            return Err(errors::assert_arg_failed("commit"));
        }
        // SAFETY: a live commit (contract); `raw_header` set by its parse.
        let h = unsafe { (*commit).raw_header };
        // SAFETY: NUL-terminated (contract).
        let f = unsafe { cstr_bytes(field) };
        if h.is_null() {
            // A quick-parsed commit.
            return header_field_cstr(c"", f, out);
        }
        // SAFETY: the commit's own NUL-terminated header, unwritten while
        // it lives (it outlives the call, by the contract).
        unsafe { header_field_raw(h, f, out) }
    }

    /// `git_commit__header_field`.
    ///
    /// # Safety
    ///
    /// `out` a valid `git_str`; `commit` a live commit; `field`
    /// NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit__header_field(
        out: *mut GitStr,
        commit: *const GitCommit,
        field: *const c_char,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null(), "out", -1);
            // SAFETY: a valid, unaliased `git_str` (contract).
            let out = unsafe { &mut *out };
            // SAFETY: a live commit; `field` NUL-terminated (contract).
            code(unsafe { header_field_of(out, commit, field) })
        })
    }

    /// `git_commit_extract_signature`.
    ///
    /// # Safety
    ///
    /// `signature_out`, `signed_data_out` valid `git_buf`s; `repo` live;
    /// `commit_id` readable; `field` NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_extract_signature(
        signature_out: *mut GitBuf,
        signed_data_out: *mut GitBuf,
        repo: *mut git_repository,
        commit_id: *mut Oid,
        field: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let mut signature = GitStr::new();
            let mut signed_data = GitStr::new();
            // SAFETY: `signature_out` a valid `git_buf` (contract).
            let mut error = unsafe { git_buf_tostr(&raw mut signature, signature_out) };
            if error == 0 {
                // SAFETY: `signed_data_out` a valid `git_buf` (contract).
                error = unsafe { git_buf_tostr(&raw mut signed_data, signed_data_out) };
            }
            if error == 0 {
                // SAFETY: two distinct strings; `repo` live, `commit_id`
                // readable, `field` NULL or NUL-terminated (contract).
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
                error = unsafe { git_buf_fromstr(signature_out, &raw mut signature) };
            }
            if error == 0 {
                // SAFETY: as above.
                error = unsafe { git_buf_fromstr(signed_data_out, &raw mut signed_data) };
            }
            error
        })
    }

    /// `git_commit__extract_signature`.
    ///
    /// # Safety
    ///
    /// As `git_commit_extract_signature`, with valid `git_str`s.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit__extract_signature(
        signature: *mut GitStr,
        signed_data: *mut GitStr,
        repo: *mut git_repository,
        commit_id: *mut Oid,
        field: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!signature.is_null(), "signature", -1);
            git_assert_arg!(!signed_data.is_null(), "signed_data", -1);
            // SAFETY: a valid `git_str`, distinct from `signed_data`
            // (contract).
            let sig = unsafe { &mut *signature };
            // SAFETY: as above.
            let sd = unsafe { &mut *signed_data };
            sig.clear();
            sd.clear();
            let field = field.get().map(CStr::to_bytes).unwrap_or(b"gpgsig");
            // SAFETY: a live repository (contract).
            let odb = match unsafe { repo_odb(repo) } {
                Ok(o) => o,
                Err(e) => return e.raw(),
            };
            let mut obj = ptr::null_mut();
            // SAFETY: `obj` writable; `commit_id` readable (contract).
            let error = unsafe { crate::odb::ffi::git_odb_read(&raw mut obj, odb, commit_id) };
            if error < 0 {
                return error;
            }
            // SAFETY: the object just read, live until freed below.
            let r = if unsafe { crate::odb::ffi::git_odb_object_type(obj) } != OBJ_COMMIT {
                let _ = git_error_set!(
                    ErrorClass::INVALID,
                    "the requested type does not match the type in the ODB"
                );
                Err(ErrorCode::NOT_FOUND)
            } else {
                // SAFETY: as above.
                let p = unsafe { crate::odb::ffi::git_odb_object_data(obj) }.cast::<c_char>();
                // SAFETY: as above.
                let n = unsafe { crate::odb::ffi::git_odb_object_size(obj) };
                let data = if p.is_null() {
                    &[][..]
                } else {
                    // SAFETY: the object's data, readable for its size and
                    // unchanged until it is freed below.
                    unsafe { crate::util::export::bytes(p, n) }
                };
                extract_signature(data, field, sig, sd)
            };
            // SAFETY: freed once.
            unsafe { crate::odb::ffi::git_odb_object_free(obj) };
            if let Err(e) = r {
                sig.clear();
                sd.clear();
                return e.raw();
            }
            0
        })
    }

    /// `git_commit_create_buffer`.
    ///
    /// # Safety
    ///
    /// `out` a valid `git_buf`; the rest as `git_commit_create`'s.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create_buffer(
        out: *mut GitBuf,
        repo: *mut git_repository,
        author: *const GitSignature,
        committer: *const GitSignature,
        message_encoding: CStrArg<'_>,
        message: *const c_char,
        tree: *const crate::tree::GitTree,
        parent_count: usize,
        parents: *const *mut GitCommit,
    ) -> c_int {
        barrier(|| {
            let f = |s: &mut GitStr| {
                // SAFETY: the caller's contract on the arguments; `s` a valid
                // `git_str`.
                ErrorCode::check_zero(unsafe {
                    git_commit__create_buffer(
                        s,
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
            // SAFETY: `out` a valid `git_buf` (contract).
            unsafe { wrap_buf(out, f) }
        })
    }

    /// `git_commit__create_buffer`.
    ///
    /// # Safety
    ///
    /// `out` a valid `git_str`; the rest as `git_commit_create`'s.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit__create_buffer(
        out: *mut GitStr,
        repo: *mut git_repository,
        author: *const GitSignature,
        committer: *const GitSignature,
        message_encoding: CStrArg<'_>,
        message: *const c_char,
        tree: *const crate::tree::GitTree,
        parent_count: usize,
        parents: *const *mut GitCommit,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!tree.is_null(), "tree", -1);
            // SAFETY: a live tree (contract).
            let owner = unsafe { object_owner(tree) };
            git_assert_arg!(owner == Ok(repo), "git_tree_owner(tree) == repo", -1);
            git_assert_arg!(!out.is_null(), "out", -1);
            // SAFETY: a live tree.
            let tree_id = unsafe { object_id(tree) }.unwrap_or_default();
            // SAFETY: the caller's contract.
            let mut next = unsafe { from_array(parents, parent_count, repo) };
            let mut parents_arr = GitArray::new();
            // SAFETY: `repo` live.
            if let Err(e) = unsafe {
                validate_tree_and_parents(&mut parents_arr, repo, &tree_id, &mut next, None, true)
            } {
                return e.raw();
            }
            let args = CreateArgs {
                // SAFETY: NULL or a live signature (contract).
                author: unsafe { sig_out(author) },
                // SAFETY: as above.
                committer: unsafe { sig_out(committer) },
                // SAFETY: NULL or NUL-terminated (contract).
                message: unsafe { opt_cbytes(message) },
                tree: tree_id,
                encoding: message_encoding.get().map(CStr::to_bytes),
                headers: Ok(Vec::new()),
                update_ref: None,
                validate: true,
            };
            // SAFETY: a valid, unaliased `git_str` (contract).
            let out = unsafe { &mut *out };
            code(args_buffer(out, &args, parents_arr.as_slice()))
        })
    }

    /// `git_commit_create_with_signature`.
    ///
    /// # Safety
    ///
    /// `out` writable; `repo` live; `commit_content` NUL-terminated;
    /// `signature`, `signature_field` NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create_with_signature(
        out: *mut Oid,
        repo: *mut git_repository,
        commit_content: CStrArg<'_>,
        signature: CStrArg<'_>,
        signature_field: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null(), "out", -1);
            git_assert_arg!(!repo.is_null(), "repo", -1);
            let commit_content = git_arg!(commit_content.get(), "commit_content", -1);
            let content = commit_content.to_bytes();
            let sig = signature.get().map(CStr::to_bytes);
            let field = signature_field
                .get()
                .map(CStr::to_bytes)
                .unwrap_or(b"gpgsig");
            // SAFETY: a live repository (contract).
            let oid_type: git_oid_t = unsafe { (*repo).oid_type_raw() };

            // The first step is to verify that all the tree and parents exist
            let parsed = alloc::calloc(1, size_of::<GitCommit>()).cast::<GitCommit>();
            if parsed.is_null() {
                return -1;
            }
            let r = (|| -> Result<()> {
                // SAFETY: a zeroed commit, all-zero being a valid (empty)
                // `GitCommit`; ours alone.
                let pc = unsafe { &mut *parsed };
                if parse_into(pc, content, &GitCommitParseOptions { oid_type, flags: 0 }).is_err() {
                    return Err(ErrorCode::GENERIC);
                }
                let pids = pc.parent_ids.as_slice();
                let mut next = |i: usize| pids.get(i).copied();
                let mut parents = GitArray::new();
                // SAFETY: `repo` live.
                unsafe {
                    validate_tree_and_parents(
                        &mut parents,
                        repo,
                        &pc.tree_id,
                        &mut next,
                        None,
                        true,
                    )
                }?;
                drop(parents);

                let mut signed = GitStr::new();
                let bytes = match sig {
                    Some(s) => {
                        append_header(&mut signed, content, field, s)?;
                        signed.as_bytes()
                    }
                    None => content,
                };
                // SAFETY: `repo` is live (contract).
                let odb = unsafe { repo_odb(repo) }?;
                // SAFETY: readable bytes; `out` writable.
                ErrorCode::check(unsafe {
                    crate::odb::ffi::git_odb_write(
                        out,
                        odb,
                        bytes.as_ptr().cast(),
                        bytes.len(),
                        OBJ_COMMIT,
                    )
                })
                .map(drop)
            })();
            // SAFETY: the commit allocated above, freed once.
            unsafe { git_commit__free(parsed.cast()) };
            code(r)
        })
    }

    /// `check_for_empty_commit`.
    ///
    /// # Safety
    ///
    /// `repo` live; `tree` NULL or a live tree; `index` a live index when
    /// `tree` is NULL.
    unsafe fn check_for_empty_commit(
        repo: *mut git_repository,
        tree: *const crate::tree::GitTree,
        index: *mut pend::git_index,
    ) -> c_int {
        let mut head_tree: *mut crate::tree::GitTree = ptr::null_mut();
        // SAFETY: `repo` live; `head_tree` writable (a `git_tree *`).
        let error = unsafe {
            crate::repository::ffi::git_repository_head_tree((&raw mut head_tree).cast(), repo)
        };
        let r = if error != 0 && error != ErrorCode::UNBORN_BRANCH.raw() {
            error
        } else {
            // SAFETY: the trees and the index are live (contract).
            match unsafe { crate::sys::diff::count_deltas(repo, head_tree, tree.cast_mut(), index) }
            {
                Err(e) => e,
                Ok(0) => {
                    let _ =
                        git_error_set!(ErrorClass::REPOSITORY, "no changes are staged for commit");
                    ErrorCode::UNCHANGED.raw()
                }
                Ok(_) => 0,
            }
        };
        // SAFETY: NULL or the tree looked up, freed once.
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
    /// `out` writable; `repo` live; `tree` a live tree of it; `message`
    /// NUL-terminated; `opts` valid.
    unsafe fn create_from_tree(
        out: *mut Oid,
        repo: *mut git_repository,
        tree: *const crate::tree::GitTree,
        message: *const c_char,
        opts: &GitCommitCreateOptions,
    ) -> c_int {
        let mut ext = ext_options(Some(opts));
        ext.update_ref = c"HEAD".as_ptr();
        let mut author = opts.author;
        let mut committer = opts.committer;
        let mut default_signature = ptr::null_mut();
        let mut parents = GitCommitArray {
            commits: ptr::null(),
            count: 0,
        };
        let error = (|| {
            if author.is_null() || committer.is_null() {
                // SAFETY: `repo` live; writable out-parameter.
                if unsafe { pend::git_signature_default(&raw mut default_signature, repo) } < 0 {
                    return -1;
                }
                if author.is_null() {
                    author = default_signature;
                }
                if committer.is_null() {
                    committer = default_signature;
                }
            }
            // SAFETY: `repo` live; writable out-parameter.
            // (`l_commit::GitCommitArray` and the repository's are both
            // `git_commitarray`.)
            if unsafe {
                crate::repository::ffi::git_repository_commit_parents(
                    (&raw mut parents).cast(),
                    repo,
                )
            } < 0
            {
                return -1;
            }
            // SAFETY: the caller's contract; the parents just looked up.
            unsafe {
                git_commit_create_ext(
                    out,
                    repo,
                    author,
                    committer,
                    message,
                    tree,
                    parents.count,
                    parents.commits,
                    &raw const ext,
                )
            }
        })();
        // SAFETY: the array filled above (or empty), released once.
        unsafe { git_commitarray_dispose(&raw mut parents) };
        // SAFETY: NULL or the signature made above, freed once.
        unsafe { pend::git_signature_free(default_signature) };
        error
    }

    /// `git_commit_create_from_stage`.
    ///
    /// # Safety
    ///
    /// `out` writable; `repo` live; `message` NUL-terminated; `given_opts`
    /// NULL or valid.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create_from_stage(
        out: *mut Oid,
        repo: *mut git_repository,
        message: *const c_char,
        given_opts: *const GitCommitCreateOptions,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(
                !out.is_null() && !repo.is_null() && !message.is_null(),
                "out && repo && message",
                -1
            );
            // SAFETY: NULL or valid (contract): a copy, as the C's memcpy.
            let opts = unsafe { given_opts.as_ref() }
                .copied()
                .unwrap_or(GitCommitCreateOptions::INIT);
            let mut index = ptr::null_mut();
            let mut tree = ptr::null_mut();
            let error = (|| {
                // SAFETY: `repo` live; writable out-parameter.
                if unsafe { crate::repository::ffi::git_repository_index(&raw mut index, repo) } < 0
                {
                    return -1;
                }
                if !opts.allow_empty_commit() {
                    // SAFETY: `repo` and `index` live.
                    let e = unsafe { check_for_empty_commit(repo, ptr::null(), index) };
                    if e < 0 {
                        return e;
                    }
                }
                let mut tree_id = Oid::default();
                // SAFETY: the index looked up above; `tree_id` writable.
                let written =
                    unsafe { crate::index::ffi::git_index_write_tree(&raw mut tree_id, index) };
                if written < 0
                    // SAFETY: `repo` live; `tree` writable; `tree_id` set.
                    || unsafe { pend::git_tree_lookup(&raw mut tree, repo, &raw const tree_id) } < 0
                {
                    return -1;
                }
                // SAFETY: the caller's contract; the tree just looked up.
                unsafe { create_from_tree(out, repo, tree, message, &opts) }
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
    /// `out` writable; `repo` live; `tree` a live tree of it; `message`
    /// NUL-terminated; `given_opts` NULL or valid.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_create_from_tree(
        out: *mut Oid,
        repo: *mut git_repository,
        tree: *const crate::tree::GitTree,
        message: *const c_char,
        given_opts: *const GitCommitCreateOptions,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(
                !out.is_null() && !repo.is_null() && !tree.is_null() && !message.is_null(),
                "out && repo && tree && message",
                -1
            );
            // SAFETY: NULL or valid (contract).
            let opts = unsafe { given_opts.as_ref() }
                .copied()
                .unwrap_or(GitCommitCreateOptions::INIT);
            if !opts.allow_empty_commit()
                // SAFETY: live repository and tree (contract).
                && unsafe { check_for_empty_commit(repo, tree, ptr::null_mut()) } < 0
            {
                return -1;
            }
            // SAFETY: the caller's contract.
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
        out: *mut Oid,
        repo: *mut git_repository,
        message: *const c_char,
        opts: *const GitCommitCreateOptions,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null() && !repo.is_null(), "out && repo", -1);
            let mut index = ptr::null_mut();
            let mut tree = ptr::null_mut();
            let mut tree_id = Oid::default();
            // `repo` is live and the out-parameters writable (contract).
            // SAFETY: as above.
            let error = if unsafe { crate::repository::ffi::git_repository_index(&raw mut index, repo) } < 0
                // SAFETY: the index just looked up.
                || unsafe { crate::index::ffi::git_index_write_tree(&raw mut tree_id, index) } < 0
                // SAFETY: as above; `tree_id` set.
                || unsafe { pend::git_tree_lookup(&raw mut tree, repo, &raw const tree_id) } < 0
            {
                -1
            } else {
                // SAFETY: the caller's contract; the tree just looked up.
                unsafe { git_commit_amend_from_tree(out, repo, tree, message, opts) }
            };
            // SAFETY: NULL or the tree looked up above, released once.
            unsafe { pend::git_tree_free(tree) };
            // SAFETY: NULL or the index looked up above, released once.
            unsafe { crate::index::ffi::git_index_free(index) };
            error
        })
    }

    /// `git_commit_amend_from_tree`.
    ///
    /// # Safety
    ///
    /// `out` writable; `repo` live; `tree` a live tree of it; `given_message`
    /// NULL or NUL-terminated; `opts` NULL or valid.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_amend_from_tree(
        out: *mut Oid,
        repo: *mut git_repository,
        tree: *const crate::tree::GitTree,
        given_message: *const c_char,
        opts: *const GitCommitCreateOptions,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(
                !out.is_null() && !repo.is_null() && !tree.is_null(),
                "out && repo && tree",
                -1
            );
            // SAFETY: NULL or valid (contract).
            let opts = unsafe { opts.as_ref() };
            let mut ext = ext_options(opts);
            let mut head_ref: *mut pend::git_reference = ptr::null_mut();
            let mut head_commit: *mut GitCommit = ptr::null_mut();
            let mut new_committer = ptr::null_mut();
            let error = (|| {
                // SAFETY: `repo` live; writable out-parameters.
                let mut e = unsafe {
                    crate::repository::ffi::git_repository_head((&raw mut head_ref).cast(), repo)
                };
                if e >= 0 {
                    // SAFETY: the reference just looked up.
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
                let author = match opts.map(|o| o.author).filter(|a| !a.is_null()) {
                    Some(a) => a,
                    // SAFETY: the commit just looked up.
                    None => unsafe { (*head_commit).author.cast_const() },
                };
                let committer = match opts.map(|o| o.committer).filter(|c| !c.is_null()) {
                    Some(c) => c,
                    None => {
                        // SAFETY: `repo` live; writable out-parameter.
                        let e =
                            unsafe { pend::git_signature_default(&raw mut new_committer, repo) };
                        if e < 0 {
                            return e;
                        }
                        new_committer.cast_const()
                    }
                };
                let message = if given_message.is_null() {
                    // SAFETY: the commit just looked up, live until freed.
                    ext.message_encoding = unsafe { (*head_commit).message_encoding };
                    // SAFETY: as above.
                    unsafe { message_ptr(head_commit) }
                } else {
                    given_message
                };
                // SAFETY: the head commit is live; its parents unchanged.
                let pids = unsafe { parent_ids_of(head_commit) };
                let mut next = |i: usize| pids.get(i).copied();
                // SAFETY: a live tree.
                let tree_id = unsafe { object_id(tree) }.unwrap_or_default();
                // SAFETY: the caller's contract.
                let e = unsafe {
                    create_from_c(
                        out,
                        repo,
                        author,
                        committer,
                        message,
                        &raw const tree_id,
                        &mut next,
                        Some(&ext),
                        false,
                    )
                };
                if e != 0 {
                    return e;
                }
                // SAFETY: the reference looked up; `out` written.
                unsafe {
                    crate::refs::ffi::git_reference__update_for_commit(
                        repo,
                        head_ref,
                        CStrArg::NULL,
                        out,
                        c"commit".as_ptr(),
                    )
                }
            })();
            // SAFETY: NULL or the signature made above, freed once.
            unsafe { pend::git_signature_free(new_committer) };
            // SAFETY: NULL or the commit looked up above, released once.
            unsafe { pend::git_commit_free(head_commit) };
            // SAFETY: NULL or the reference looked up above, freed once.
            unsafe { crate::refs::ffi::git_reference_free(head_ref) };
            error
        })
    }

    /// `git_commit_committer_with_mailmap`.
    ///
    /// # Safety
    ///
    /// `out` writable; `commit` a live commit; `mailmap` NULL or live.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commit_committer_with_mailmap(
        out: *mut *mut GitSignature,
        commit: *const GitCommit,
        mailmap: *const c_void,
    ) -> c_int {
        barrier(|| {
            // SAFETY: a live commit (the C dereferences it too).
            let sig = unsafe { (*commit).committer };
            // SAFETY: `out` writable, `mailmap` NULL or live (contract);
            // `sig` the commit's signature.
            unsafe { crate::sys::mailmap::resolve_signature(out, mailmap, sig) }
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
        commit: *const GitCommit,
        mailmap: *const c_void,
    ) -> c_int {
        barrier(|| {
            // SAFETY: a live commit (the C dereferences it too).
            let sig = unsafe { (*commit).author };
            // SAFETY: `out` writable, `mailmap` NULL or live (contract);
            // `sig` the commit's signature.
            unsafe { crate::sys::mailmap::resolve_signature(out, mailmap, sig) }
        })
    }

    /// `git_commitarray_dispose`.
    ///
    /// # Safety
    ///
    /// `array` NULL or a `git_commitarray` that libgit2 filled.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_commitarray_dispose(array: *mut GitCommitArray) {
        barrier(|| {
            if array.is_null() {
                return;
            }
            // SAFETY: a filled array (contract).
            let a = unsafe { array.read() };
            for i in 0..a.count {
                // SAFETY: `i < count` readable commit pointers.
                let c = unsafe { nth(a.commits, i) };
                // SAFETY: a commit reference the array owns, released once.
                unsafe { pend::git_commit_free(c) };
            }
            // SAFETY: the buffer from `git__allocator`, freed once.
            unsafe { alloc::free(a.commits.cast_mut().cast()) };
            // SAFETY: writable (contract).
            unsafe {
                array.write(GitCommitArray {
                    commits: ptr::null(),
                    count: 0,
                })
            };
        })
    }
}

/// `git_commit_message`'s pointer: into the raw message, past its leading
/// newlines (NULL without one).
///
/// # Safety
///
/// `c` is a live commit.
pub(crate) unsafe fn message_ptr(c: *const GitCommit) -> *const c_char {
    // SAFETY: a live commit (contract); the field is set by the parse and
    // unchanged after.
    let mut m = unsafe { (*c).raw_message }.cast_const();
    if m.is_null() {
        return m;
    }
    // The raw message is NUL-terminated: the loop stops at its NUL at the
    // latest, so every byte read is within it.
    // SAFETY: as above.
    while unsafe { m.read() } == b'\n' as c_char {
        // SAFETY: `m` is a newline, not the NUL: `m + 1` is in the string.
        m = unsafe { m.add(1) };
    }
    m
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
