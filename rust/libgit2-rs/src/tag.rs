//! Port of `src/libgit2/tag.c`: annotated tags (parsing and accessors),
//! creating annotated and lightweight tags, deleting, listing and
//! iterating them (through the references of the refs module).
//!
//! # Types
//!
//! - [`GitTag`] is `struct git_tag`, mirrored (kind 2 with every field in
//!   the prefix): C never allocates one, but reads all its fields
//!   (`indexer.c` `target`, `describe.c` `tagger`, the tests the rest). Tags
//!   are allocated with `git__allocator` ([`alloc`]) and freed by [`free`],
//!   through the object table; [`TagData`] is what the parser returns.
//! - [`Tag`] is the safe API's tag.
//!
//! # Parsing
//!
//! [`parse`] is `tag_parse` over a byte slice, in safe Rust (Miri-tested
//! and difftested against the C: `rust/difftest/obj_diff.c`).

use crate::util::cabi::{CStrArg, Out};
use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr::{self, NonNull};

use crate::object::deps::{self as p, git_reference, git_repository};
use crate::object::{
    self, GIT_OBJECT_ANY, GIT_OBJECT_INVALID, GIT_OBJECT_TAG, GitObject, Object, ObjectType,
    Repository, git_object_t,
};
use crate::oid::{Oid, OidType};
use crate::signature::{self, GitSignature, Signature, SignatureRef};
use crate::util::alloc::{self, GitCStr};
use crate::util::errors::{self, Bytes, Error, ErrorClass, ErrorCode, Result};
use crate::util::str::GitStr;
use crate::util::vector::GitVector;
use crate::{git_assert, git_assert_arg, git_error_set};

/// `GIT_REFS_TAGS_DIR`.
pub const GIT_REFS_TAGS_DIR: &[u8] = b"refs/tags/";

/// The parsed fields of a tag, after its target.
#[derive(Debug, Default)]
pub struct TagData {
    /// The target's type.
    pub(crate) kind: git_object_t,
    /// The tag's name.
    pub name: Option<GitCStr>,
    /// The tagger (none for some old tags).
    pub tagger: Option<Signature>,
    /// The message (none when the tag ends after its header).
    pub message: Option<GitCStr>,
}

/// `struct git_tag`, mirrored (the fields are crate-private: the accessors
/// trust the invariant below, which only this crate's constructors make):
/// C reads every field (`describe.c` reads
/// `tagger`, `indexer.c` `target`, the tests all of them), though only this
/// module allocates and frees tags. `tag_name` and `message` are NULL or
/// NUL-terminated `git__allocator` strings, `tagger` NULL or a
/// `git__allocator` block of a [`Signature`]'s (its strings owned), all
/// owned by the tag, written only before it is published.
#[repr(C)]
#[derive(Debug)]
pub struct GitTag {
    /// The object header.
    pub(crate) object: GitObject,
    /// The target's id.
    pub(crate) target: Oid,
    /// The target's type (`type`).
    pub kind: git_object_t,
    /// The name.
    pub(crate) tag_name: *mut c_char,
    /// The tagger.
    pub(crate) tagger: *mut GitSignature,
    /// The message.
    pub(crate) message: *mut c_char,
}

// The header is first (C casts), and the alignment is what `git__allocator`
// gives.
const _: () = assert!(core::mem::offset_of!(GitTag, object) == 0);
const _: () = assert!(align_of::<GitTag>() <= 8);

/// Allocate an empty tag (`git__calloc(1, sizeof(git_tag))`).
pub(crate) fn alloc() -> Result<NonNull<GitTag>> {
    let p = NonNull::new(alloc::malloc(size_of::<GitTag>()).cast::<GitTag>())
        .ok_or(ErrorCode::GENERIC)?;
    // SAFETY: a fresh `git__allocator` block of a `GitTag`'s size, 8-aligned.
    unsafe {
        p.as_ptr().write(GitTag {
            object: GitObject::zeroed(),
            target: Oid::NONE,
            kind: 0,
            tag_name: ptr::null_mut(),
            tagger: ptr::null_mut(),
            message: ptr::null_mut(),
        })
    };
    Ok(p)
}

/// Free a tag's fields (C's `git_tag__free` before the block).
///
/// # Safety
///
/// `tag_name`, `tagger` and `message` are NULL or the tag's own
/// `git__allocator` allocations (the type's invariant), and nothing (C
/// holding what the accessors returned included) uses them after.
unsafe fn free_fields(t: &mut GitTag) {
    // SAFETY: NULL or a `git__allocator` signature block whose strings the
    // tag owns, unused after (the contract); freed once (set to NULL below).
    unsafe { signature::free_raw(t.tagger) };
    // SAFETY: NULL or the tag's own `git__allocator` string, unused after
    // (the contract), freed once.
    unsafe { alloc::free(t.message.cast()) };
    // SAFETY: as for `message`.
    unsafe { alloc::free(t.tag_name.cast()) };
    t.tagger = ptr::null_mut();
    t.message = ptr::null_mut();
    t.tag_name = ptr::null_mut();
}

/// `git_tag__free`.
///
/// # Safety
///
/// `tag` is a tag of [`alloc`]'s, with no reference left.
pub(crate) unsafe fn free(tag: NonNull<GitTag>) {
    // SAFETY: a live tag of `alloc`'s (initialized, aligned) that nothing
    // else references any more (the contract): exclusive until freed.
    let t = unsafe { &mut *tag.as_ptr() };
    // SAFETY: its fields are its own (the type's invariant) and, the last
    // reference being gone, nothing uses them after.
    unsafe { free_fields(t) };
    // SAFETY: `alloc`'s `git__allocator` block, freed once; `t` is not
    // used after.
    unsafe { alloc::free(tag.as_ptr().cast()) };
}

/// `tag_error`: `GIT_EINVALID` with `failed to parse tag: <msg>`.
fn tag_error(msg: &str) -> ErrorCode {
    let _ = git_error_set!(ErrorClass::TAG, "failed to parse tag: {}", msg);
    ErrorCode::INVALID
}

/// `git__malloc` + copy of `s` with a NUL (`GIT_ERROR_CHECK_ALLOC_ADD`,
/// `GIT_ERROR_CHECK_ALLOC`).
fn copy(s: &[u8]) -> Result<GitCStr> {
    crate::util::integer::alloc_add(s.len(), 1)?;
    // SAFETY: NULL or a fresh NUL-terminated `git__allocator` string.
    unsafe { GitCStr::from_raw(alloc::substrdup(s)) }.ok_or(ErrorCode::GENERIC)
}

/// `git__memmem(s, len, "\n\n", 2)`: where the first blank line starts.
fn find_blank_line(s: &[u8]) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = memchr::memchr(b'\n', &s[from..]) {
        let at = from + i;
        if s.get(at + 1) == Some(&b'\n') {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

/// `tag_parse`: the target and the rest of a tag's contents. Errors are
/// `GIT_EINVALID` with C's messages (a tagger's are the signature
/// parser's), or -1 when out of memory.
pub fn parse(buf: &[u8], oid_type: crate::oid::git_oid_t) -> Result<(Oid, TagData)> {
    const TAG_TYPES: [&[u8]; 4] = [b"commit\n", b"tree\n", b"blob\n", b"tag\n"];
    let Some(t) = OidType::from_raw(oid_type) else {
        // C's checks with a hex size of 0 all fail here.
        return Err(tag_error("object field invalid"));
    };
    let (target, mut pos) = match object::parse_oid_header(buf, b"object ", t) {
        Ok(v) => v,
        Err(_) => return Err(tag_error("object field invalid")),
    };
    let end = buf.len();
    if pos + 5 >= end {
        return Err(tag_error("object too short"));
    }
    if &buf[pos..pos + 5] != b"type " {
        return Err(tag_error("type field not found"));
    }
    pos += 5;
    let mut kind = GIT_OBJECT_INVALID;
    for (i, name) in TAG_TYPES.iter().enumerate() {
        if pos + name.len() >= end {
            return Err(tag_error("object too short"));
        }
        if &buf[pos..pos + name.len()] == *name {
            kind = i as git_object_t + 1;
            pos += name.len();
            break;
        }
    }
    if kind == GIT_OBJECT_INVALID {
        return Err(tag_error("invalid object type"));
    }
    if pos + 4 >= end {
        return Err(tag_error("object too short"));
    }
    if &buf[pos..pos + 4] != b"tag " {
        return Err(tag_error("tag field not found"));
    }
    pos += 4;
    let Some(nl) = memchr::memchr(b'\n', &buf[pos..]) else {
        return Err(tag_error("object too short"));
    };
    let name = copy(&buf[pos..pos + nl])?;
    pos += nl + 1;

    let mut tagger = None;
    if pos < end && buf[pos] != b'\n' {
        let (sig, n) = signature::parse(&buf[pos..], Some(b"tagger "), b'\n')?;
        tagger = Some(sig);
        pos += n;
    }

    let mut message = None;
    if pos < end {
        // Not at the end of the header: look for it.
        if buf[pos] != b'\n' {
            match find_blank_line(&buf[pos..]) {
                Some(i) => pos += i + 1,
                None => return Err(tag_error("tag contains no message")),
            }
        }
        pos += 1;
        message = Some(copy(&buf[pos..])?);
    }
    Ok((
        target,
        TagData {
            kind,
            name: Some(name),
            tagger,
            message,
        },
    ))
}

/// `git_tag__parse_raw` into a fresh tag.
///
/// # Safety
///
/// `tag` is a fresh tag, not shared.
pub(crate) unsafe fn parse_into(
    tag: NonNull<GitTag>,
    data: &[u8],
    oid_type: crate::oid::git_oid_t,
) -> c_int {
    match parse(data, oid_type) {
        Ok((target, d)) => {
            let tagger = match d.tagger.map(Signature::into_raw).transpose() {
                Ok(t) => t.unwrap_or(ptr::null_mut()),
                Err(e) => return e.raw(),
            };
            // SAFETY: a live, initialized tag that only the caller holds
            // (the contract), so exclusive for this block.
            let t = unsafe { &mut *tag.as_ptr() };
            t.target = target;
            t.kind = d.kind;
            t.tag_name = d.name.map_or(ptr::null_mut(), GitCStr::into_raw);
            t.tagger = tagger;
            t.message = d.message.map_or(ptr::null_mut(), GitCStr::into_raw);
            0
        }
        Err(e) => e.raw(),
    }
}

/// `git_tag__parse`.
///
/// # Safety
///
/// `tag` is a fresh tag; `odb_obj` a live odb object.
pub(crate) unsafe fn parse_odb(
    tag: NonNull<GitTag>,
    odb_obj: *mut p::git_odb_object,
    oid_type: crate::oid::git_oid_t,
) -> c_int {
    let c = odb_obj.cast::<object::GitCachedObj>();
    // SAFETY: a live odb object (the contract), which starts with its
    // `git_cached_obj`; `size` is set before it is returned and not written
    // after. A place read, no reference.
    let size = unsafe { (*c).size };
    // SAFETY: a live odb object (the contract).
    let d = unsafe { p::git_odb_object_data(odb_obj) };
    // SAFETY: an odb object's buffer holds `size` bytes, unwritten while it
    // lives, which is the whole call (the contract).
    let data = unsafe { crate::util::export::bytes(d.cast(), size) };
    // SAFETY: `tag` is fresh and only the caller's (the contract).
    unsafe { parse_into(tag, data, oid_type) }
}

impl GitTag {
    /// The target's type.
    pub(crate) fn target_kind(&self) -> git_object_t {
        self.kind
    }

    /// The name.
    pub(crate) fn name(&self) -> &[u8] {
        if self.tag_name.is_null() {
            return &[];
        }
        // SAFETY: the tag's own NUL-terminated string (the invariant).
        unsafe { CStr::from_ptr(self.tag_name) }.to_bytes()
    }

    /// The tagger.
    pub(crate) fn tagger(&self) -> Option<&Signature> {
        // SAFETY: NULL or the tag's own signature block (the type's
        // invariant), made by `Signature::into_raw`: a `git_signature` whose
        // strings are owned and non-NULL, which `Signature` (of the same
        // layout) is; borrowed from `self`, so live and unwritten.
        unsafe { self.tagger.cast::<Signature>().as_ref() }
    }

    /// The message.
    pub(crate) fn message(&self) -> Option<&[u8]> {
        if self.message.is_null() {
            return None;
        }
        // SAFETY: the tag's own NUL-terminated string.
        Some(unsafe { CStr::from_ptr(self.message) }.to_bytes())
    }
}

/// `git_tag_target`: the target, with a new reference.
///
/// # Safety
///
/// `tag` is a live tag of the object table, not freed during the call.
pub unsafe fn target(tag: NonNull<GitTag>) -> Result<NonNull<GitObject>> {
    // SAFETY: a live tag (the contract); published tags are only read, so a
    // shared reference for the call is valid.
    let t = unsafe { tag.as_ref() };
    // (C dereferences a NULL repository: a tag parsed from raw data.)
    git_assert!(!t.object.repo.is_null(), "t->object.repo");
    // SAFETY: a non-NULL owner of a live object is a live repository (it
    // outlives its objects).
    unsafe { object::lookup(t.object.repo, &t.target, t.kind) }
}

// ---------------------------------------------------------------------------
// Creating, deleting and listing tags
// ---------------------------------------------------------------------------

/// A held reference, freed on drop.
struct RefHandle(*mut git_reference);

impl Drop for RefHandle {
    fn drop(&mut self) {
        // SAFETY: the reference this value holds (or NULL), freed once.
        unsafe { p::git_reference_free(self.0) }
    }
}

/// `refs/tags/<name>`, NUL-terminated (`git_str_joinpath`).
fn ref_name(tag_name: &[u8]) -> Result<GitStr> {
    let mut s = GitStr::new();
    s.joinpath(GIT_REFS_TAGS_DIR, tag_name)?;
    Ok(s)
}

fn as_c(s: &GitStr) -> *const c_char {
    s.as_c_str().map_or(c"".as_ptr(), CStr::as_ptr)
}

/// `retrieve_tag_reference_oid`: `git_reference_name_to_id` of
/// `refs/tags/<name>` into `oid`; the C status.
fn retrieve_oid(
    oid: &mut Oid,
    refname: &mut GitStr,
    repo: &git_repository,
    tag_name: &[u8],
) -> c_int {
    match ref_name(tag_name) {
        Ok(s) => *refname = s,
        Err(e) => return e.raw(),
    }
    // SAFETY: `repo` is a live repository (a reference), which the refs
    // module reads only through `&` (its state is interior-mutable);
    // `as_c` is a NUL-terminated name that `refname` keeps for the call;
    // `oid` is a writable id (a `&mut`, not otherwise used meanwhile).
    unsafe { p::git_reference_name_to_id(oid, repo_raw(repo), as_c(refname)) }
}

/// `repo` as the `git_repository *` the refs module's exports take. They
/// read it only through `&` (`repo_unchecked`), as the repository's state
/// is interior-mutable, so a pointer derived from a shared reference is
/// enough.
fn repo_raw(repo: &git_repository) -> *mut git_repository {
    ptr::from_ref(repo).cast_mut()
}

/// `write_tag_annotation`: write the tag object; its id.
pub fn write_annotation(
    repo: &git_repository,
    tag_name: &[u8],
    target: &GitObject,
    tagger: &GitSignatureView<'_>,
    message: &[u8],
) -> Result<Oid> {
    let r = (|| -> Result<Oid> {
        let mut tag = GitStr::new();
        object::write_oid_header(&mut tag, b"object ", target.id())?;
        let mut line = b"type ".to_vec();
        line.extend_from_slice(object::type2string(target.raw_kind()).to_bytes());
        line.push(b'\n');
        let _ = object::printf_bytes(&mut tag, 8, &line);
        let mut line = b"tag ".to_vec();
        line.extend_from_slice(tag_name);
        line.push(b'\n');
        let _ = object::printf_bytes(&mut tag, 7, &line);
        signature::writebuf(
            &mut tag,
            Some(b"tagger "),
            tagger.name,
            tagger.email,
            tagger.when,
        );
        let _ = tag.putc(b'\n');
        tag.puts(message)?;
        let odb = repo.odb_weak()?;
        // SAFETY: the repository's odb, which it keeps live while it is
        // (and `repo` is, for the call).
        unsafe { crate::blob::odb_write(odb.as_ptr(), tag.as_bytes(), GIT_OBJECT_TAG) }
    })();
    r.map_err(|_| git_error_set!(ErrorClass::OBJECT, "failed to create tag annotation"))
}

/// A signature's fields as C passes them (`name` and `email` may be
/// anything C's `%s` prints).
#[derive(Clone, Copy, Debug)]
pub struct GitSignatureView<'a> {
    /// The name.
    pub name: &'a [u8],
    /// The e-mail.
    pub email: &'a [u8],
    /// The time.
    pub when: signature::GitTime,
}

impl<'a> GitSignatureView<'a> {
    /// A view of `sig`.
    pub fn of(sig: &'a Signature) -> GitSignatureView<'a> {
        GitSignatureView::of_ref(&sig.as_ref())
    }

    /// A view of a borrowed signature.
    pub fn of_ref(sig: &SignatureRef<'a>) -> GitSignatureView<'a> {
        GitSignatureView {
            name: sig.name(),
            email: sig.email(),
            when: sig.when().raw(),
        }
    }

    /// A view of a C signature (NULL strings as glibc prints them).
    ///
    /// # Safety
    ///
    /// `s`'s `name` and `email` are NULL or NUL-terminated strings that
    /// stay live and unwritten for `'a`.
    pub unsafe fn from_c(s: &'a GitSignature) -> GitSignatureView<'a> {
        let b = |p: *mut c_char| -> &'a [u8] {
            if p.is_null() {
                b"(null)"
            } else {
                // SAFETY: a non-NULL string of `s`: NUL-terminated, live
                // and unwritten for `'a` (the contract).
                unsafe { CStr::from_ptr(p) }.to_bytes()
            }
        };
        GitSignatureView {
            name: b(s.name),
            email: b(s.email),
            when: s.when,
        }
    }
}

/// `tag_name_is_valid`: not starting with `-`, and not `HEAD`.
pub fn name_is_valid_basic(tag_name: &[u8]) -> bool {
    tag_name.first() != Some(&b'-') && tag_name != b"HEAD"
}

/// `git_tag_create__internal`: create `refs/tags/<name>` to `target` (or
/// to a new annotation of it); the reference's target id. As C, `oid` may
/// be written on failure (an existing tag's target).
pub fn create_internal(
    oid: &mut Oid,
    repo: &git_repository,
    tag_name: &[u8],
    target: &GitObject,
    annotation: Option<(&GitSignatureView<'_>, &[u8])>,
    allow_ref_overwrite: bool,
) -> Result<()> {
    if target.repo != repo_raw(repo) {
        return Err(git_error_set!(
            ErrorClass::INVALID,
            "the given target does not belong to this repository"
        ));
    }
    if !name_is_valid_basic(tag_name) {
        return Err(git_error_set!(
            ErrorClass::TAG,
            "'{}' is not a valid tag name",
            Bytes(tag_name)
        ));
    }
    let mut refname = GitStr::new();
    let error = retrieve_oid(oid, &mut refname, repo, tag_name);
    if error < 0 && error != ErrorCode::NOT_FOUND.raw() {
        return Err(ErrorCode::from_raw(error).unwrap_or(ErrorCode::GENERIC));
    }
    // The name must not conflict with an existing reference, unless asked.
    if error == 0 && !allow_ref_overwrite {
        let _ = git_error_set!(ErrorClass::TAG, "tag already exists");
        return Err(ErrorCode::EXISTS);
    }
    *oid = match annotation {
        Some((tagger, message)) => write_annotation(repo, tag_name, target, tagger, message)
            .map_err(|_| ErrorCode::GENERIC)?,
        None => *target.id(),
    };
    create_ref(repo, &refname, oid, allow_ref_overwrite)
}

/// `git_reference_create(&new_ref, repo, refname, oid, allow_ref_overwrite,
/// NULL)`, the new reference freed.
fn create_ref(
    repo: &git_repository,
    refname: &GitStr,
    oid: &Oid,
    allow_ref_overwrite: bool,
) -> Result<()> {
    let mut new_ref = RefHandle(ptr::null_mut());
    // SAFETY: `new_ref.0` is writable; `repo` is live (a reference, read by
    // the refs module through `&` only); `as_c` is a NUL-terminated name
    // that `refname` keeps for the call; `oid` is a readable id.
    let r = unsafe {
        p::git_reference_create(
            &raw mut new_ref.0,
            repo_raw(repo),
            as_c(refname),
            oid,
            c_int::from(allow_ref_overwrite),
            CStrArg::NULL,
        )
    };
    ErrorCode::check(r).map(|_| ())
}

/// `git_tag_create_from_buffer`: validate the tag in `buffer`, write it,
/// and create its reference.
pub fn create_from_buffer(
    oid: &mut Oid,
    repo: &git_repository,
    buffer: &[u8],
    allow_ref_overwrite: bool,
) -> Result<()> {
    // The repository's odb, live while it is (and `repo` is, for the call).
    let odb = repo.odb_weak().map_err(|_| ErrorCode::GENERIC)?.as_ptr();
    // Validate the buffer (C leaks what it parsed when this fails:
    // rust/DEFECTS.md).
    let oid_type = repo.oid_type_raw();
    let (target, tag) = parse(buffer, oid_type).map_err(|_| ErrorCode::GENERIC)?;
    let mut refname = GitStr::new();
    let error = {
        // Validate the target.
        let mut target_obj = ptr::null_mut();
        // SAFETY: the repository's live odb (above), a readable id, a
        // writable out-parameter.
        if unsafe { p::git_odb_read(&raw mut target_obj, odb, &raw const target) } < 0 {
            return Err(ErrorCode::GENERIC);
        }
        /// The odb object, freed on drop.
        struct Held(*mut p::git_odb_object);
        impl Drop for Held {
            fn drop(&mut self) {
                // SAFETY: the odb object `git_odb_read` gave this value,
                // freed once, here.
                unsafe { p::git_odb_object_free(self.0) }
            }
        }
        let held = Held(target_obj);
        // SAFETY: `held.0` is the odb object read above (non-NULL on
        // success), live until `held` drops; it starts with its
        // `git_cached_obj`, whose `type_` is set before `git_odb_read`
        // returns it and not written after. A place read, no reference.
        let ttype = git_object_t::from(unsafe { (*held.0.cast::<object::GitCachedObj>()).type_ });
        if tag.kind != ttype {
            let _ = git_error_set!(ErrorClass::TAG, "the type for the given target is invalid");
            return Err(ErrorCode::GENERIC);
        }
        let name = tag
            .name
            .as_ref()
            .map_or(&[][..], |n| n.as_c_str().to_bytes());
        let error = retrieve_oid(oid, &mut refname, repo, name);
        if error < 0 && error != ErrorCode::NOT_FOUND.raw() {
            return Err(ErrorCode::GENERIC);
        }
        error
    };
    drop(tag);
    if error == 0 && !allow_ref_overwrite {
        let _ = git_error_set!(ErrorClass::TAG, "tag already exists");
        return Err(ErrorCode::EXISTS);
    }
    // Write the buffer.
    let mut stream = ptr::null_mut();
    // SAFETY: the repository's live odb, a writable out-parameter.
    ErrorCode::check(unsafe {
        p::git_odb_open_wstream(&raw mut stream, odb, buffer.len() as u64, GIT_OBJECT_TAG)
    })?;
    // SAFETY: the stream just opened (live until freed below); `buffer` is
    // readable for its length.
    let mut error =
        unsafe { p::git_odb_stream_write(stream, buffer.as_ptr().cast(), buffer.len()) };
    if error == 0 {
        // SAFETY: the live stream; `oid` a writable id (a `&mut`).
        error = unsafe { p::git_odb_stream_finalize_write(oid, stream) };
    }
    // SAFETY: the stream opened above, freed once, not used after.
    unsafe { p::git_odb_stream_free(stream) };
    ErrorCode::check(error)?;
    create_ref(repo, &refname, oid, allow_ref_overwrite)
}

/// `git_tag_delete`: delete `refs/tags/<name>`.
pub fn delete(repo: &git_repository, tag_name: &[u8]) -> Result<()> {
    let refname = ref_name(tag_name).map_err(|_| ErrorCode::GENERIC)?;
    let mut r = RefHandle(ptr::null_mut());
    // SAFETY: `r.0` is writable; `repo` is live (a reference, read by the
    // refs module through `&` only); `as_c` is a NUL-terminated name that
    // `refname` keeps for the call.
    ErrorCode::check(unsafe {
        p::git_reference_lookup(&raw mut r.0, repo_raw(repo), as_c(&refname))
    })?;
    // SAFETY: the reference just looked up (non-NULL on success), live
    // until `r` drops.
    ErrorCode::check(unsafe { p::git_reference_delete(r.0) }).map(|_| ())
}

/// `git_tag_foreach`: call `cb(refname, id)` for each tag reference; a
/// non-zero return stops, and is returned (with
/// `git_error_set_after_callback_function`).
pub fn foreach(repo: &git_repository, cb: &mut dyn FnMut(&CStr, &Oid) -> c_int) -> c_int {
    struct Data<'a> {
        repo: &'a git_repository,
        cb: &'a mut dyn FnMut(&CStr, &Oid) -> c_int,
    }
    /// # Safety
    ///
    /// `payload` is the `Data` of the running `foreach`; `name` a
    /// reference name (NUL-terminated, live for the call).
    unsafe extern "C" fn tags_cb(name: *const c_char, payload: *mut c_void) -> c_int {
        crate::util::export::barrier(|| {
            // SAFETY: `payload` is `foreach`'s `data`, which lives on its
            // stack for the whole iteration and is used by nothing else
            // while the refs module runs this callback (it only passes the
            // pointer on).
            let d = unsafe { &mut *payload.cast::<Data<'_>>() };
            // SAFETY: a reference name the refs module passes: NUL-terminated
            // and unchanged during the call (this function's contract).
            let name = unsafe { CStr::from_ptr(name) };
            if crate::util::util::prefixcmp(name.to_bytes(), GIT_REFS_TAGS_DIR) != 0 {
                return 0; // not a tag
            }
            let mut oid = Oid::NONE;
            // SAFETY: `oid` is a writable local; `d.repo` is live (a
            // reference, read through `&` only); `name` is NUL-terminated.
            let mut error = unsafe {
                p::git_reference_name_to_id(&raw mut oid, repo_raw(d.repo), name.as_ptr())
            };
            if error == 0 {
                error = (d.cb)(name, &oid);
                if error != 0 {
                    error = errors::set_after_callback(error, "git_tag_foreach");
                }
            }
            error
        })
    }
    let mut data = Data { repo, cb };
    // SAFETY: `repo` is live (a reference, read through `&` only);
    // `tags_cb` is sound with `data`'s address as its payload (its
    // contract), and `data` outlives the call.
    unsafe {
        p::git_reference_foreach_name(
            repo_raw(repo),
            tags_cb,
            crate::util::export::payload(&raw mut data),
        )
    }
}

/// `git_tag_list_match`: the names of the tags matching `pattern` (all,
/// when empty), as `git_strarray`'s `strings` and `count`. Failures give an
/// empty list (C returns 0 anyway).
pub fn list_match(repo: &git_repository, pattern: &[u8]) -> (*mut *mut c_char, usize) {
    let Ok(mut taglist) = GitVector::with_capacity(8, None) else {
        return (ptr::null_mut(), 0);
    };
    let skip = GIT_REFS_TAGS_DIR.len();
    let mut cb = |name: &CStr, _: &Oid| -> c_int {
        let short = &name.to_bytes()[skip..];
        if pattern.is_empty()
            || matches!(
                crate::util::wildmatch::wildmatch(
                    pattern,
                    short,
                    crate::util::wildmatch::WildmatchFlags::default()
                ),
                crate::util::wildmatch::WildMatch::Match
            )
        {
            let m = alloc::substrdup(short);
            if m.is_null() {
                return -1;
            }
            if let Err(e) = taglist.insert(m.cast()) {
                // SAFETY: the copy just made.
                unsafe { alloc::free(m.cast()) };
                return e.raw();
            }
        }
        0
    };
    let error = foreach(repo, &mut cb);
    if error < 0 {
        // (C leaks the names here: rust/DEFECTS.md.)
        for s in taglist.iter() {
            // SAFETY: the copies this function made.
            unsafe { alloc::free(s) };
        }
        taglist.dispose();
    }
    let (strings, count, _) = taglist.detach();
    (strings.cast(), count)
}

/// `git_tag_name_is_valid`.
pub fn name_is_valid(name: Option<&[u8]>) -> Result<bool> {
    let Some(name) = name else { return Ok(false) };
    if !name_is_valid_basic(name) {
        return Ok(false);
    }
    let mut s = GitStr::new();
    s.puts(GIT_REFS_TAGS_DIR)?;
    s.puts(name)?;
    let mut valid: c_int = 0;
    // SAFETY: a NUL-terminated name, a writable out.
    ErrorCode::check(unsafe { p::git_reference_name_is_valid(&raw mut valid, as_c(&s)) })?;
    Ok(valid != 0)
}

// ---------------------------------------------------------------------------
// The safe API
// ---------------------------------------------------------------------------

/// An annotated tag of a repository: an [`Object`] of type tag.
#[derive(Clone, Debug)]
#[repr(transparent)]
pub struct Tag<'repo>(Object<'repo>);

impl<'repo> TryFrom<Object<'repo>> for Tag<'repo> {
    type Error = Object<'repo>;

    /// The object as a tag, or the object back if it is not one.
    fn try_from(o: Object<'repo>) -> core::result::Result<Tag<'repo>, Object<'repo>> {
        if o.kind() == ObjectType::Tag {
            Ok(Tag(o))
        } else {
            Err(o)
        }
    }
}

impl<'repo> From<Tag<'repo>> for Object<'repo> {
    fn from(t: Tag<'repo>) -> Object<'repo> {
        t.0
    }
}

impl<'repo> Tag<'repo> {
    /// An object of type tag, borrowed as a tag.
    pub(crate) fn from_object_ref<'s>(obj: &'s Object<'repo>) -> &'s Tag<'repo> {
        debug_assert_eq!(obj.kind(), ObjectType::Tag);
        // SAFETY: `Tag` is a `repr(transparent)` wrapper of `Object`, whose
        // only invariant (the object is a tag) the caller checked.
        unsafe { &*core::ptr::from_ref(obj).cast::<Tag<'repo>>() }
    }

    /// `git_tag_lookup`.
    pub fn lookup(repo: &'repo Repository, id: &Oid) -> core::result::Result<Tag<'repo>, Error> {
        Ok(Tag(Object::lookup(repo, id, Some(ObjectType::Tag))?))
    }

    /// `git_tag_lookup_prefix`.
    pub fn lookup_prefix(
        repo: &'repo Repository,
        id: &Oid,
        len: usize,
    ) -> core::result::Result<Tag<'repo>, Error> {
        Ok(Tag(Object::lookup_prefix(
            repo,
            id,
            len,
            Some(ObjectType::Tag),
        )?))
    }

    fn raw(&self) -> &GitTag {
        // SAFETY: a live tag (checked type), read only once published.
        unsafe { self.0.as_ptr().cast::<GitTag>().as_ref() }
    }

    /// `git_tag_id`.
    pub fn id(&self) -> &Oid {
        self.0.id()
    }

    /// The tag as an object.
    pub fn as_object(&self) -> &Object<'repo> {
        &self.0
    }

    /// The tag as an object.
    pub fn into_object(self) -> Object<'repo> {
        self.0
    }

    /// `git_tag_name`.
    pub fn name(&self) -> &[u8] {
        self.raw().name()
    }

    /// `git_tag_message`.
    pub fn message(&self) -> Option<&[u8]> {
        self.raw().message()
    }

    /// `git_tag_tagger`.
    pub fn tagger(&self) -> Option<SignatureRef<'_>> {
        self.raw().tagger().map(Signature::as_ref)
    }

    /// `git_tag_target_id`.
    pub fn target_id(&self) -> &Oid {
        &self.raw().target
    }

    /// `git_tag_target_type`.
    pub fn target_type(&self) -> ObjectType {
        ObjectType::from_raw(self.raw().target_kind()).unwrap_or(ObjectType::Blob)
    }

    /// `git_tag_target`.
    pub fn target(&self) -> core::result::Result<Object<'repo>, Error> {
        // SAFETY: the live tag this value holds a reference to (its type
        // checked when it was made), so not freed during the call.
        let o = unsafe { target(self.0.as_ptr().cast())? };
        // SAFETY: an object of the same repository.
        Ok(unsafe { self.0.sibling(o) })
    }

    /// `git_tag_peel`: the first non-tag object the tag leads to.
    pub fn peel(&self) -> core::result::Result<Object<'repo>, Error> {
        self.0.peel(None)
    }

    /// `git_tag_create`: an annotated tag `name` of `target`, and its
    /// reference; the tag's id.
    pub fn create<'s>(
        repo: &Repository,
        name: impl AsRef<[u8]>,
        target: &Object<'_>,
        tagger: impl Into<SignatureRef<'s>>,
        message: impl AsRef<[u8]>,
        force: bool,
    ) -> core::result::Result<Oid, Error> {
        let (name, message) = (name.as_ref(), message.as_ref());
        let mut oid = Oid::NONE;
        let v = GitSignatureView::of_ref(&tagger.into());
        no_nul(v.name)?;
        no_nul(v.email)?;
        create_internal(
            &mut oid,
            repo.raw(),
            no_nul(name)?,
            target.header(),
            Some((&v, no_nul(message)?)),
            force,
        )?;
        Ok(oid)
    }

    /// `git_tag_annotation_create`: write the tag object only.
    pub fn annotation_create<'s>(
        repo: &Repository,
        name: impl AsRef<[u8]>,
        target: &Object<'_>,
        tagger: impl Into<SignatureRef<'s>>,
        message: impl AsRef<[u8]>,
    ) -> core::result::Result<Oid, Error> {
        let (name, message) = (name.as_ref(), message.as_ref());
        let v = GitSignatureView::of_ref(&tagger.into());
        no_nul(v.name)?;
        no_nul(v.email)?;
        Ok(write_annotation(
            repo.raw(),
            no_nul(name)?,
            target.header(),
            &v,
            no_nul(message)?,
        )?)
    }

    /// `git_tag_create_lightweight`: a reference `refs/tags/<name>` to
    /// `target`.
    pub fn create_lightweight(
        repo: &Repository,
        name: impl AsRef<[u8]>,
        target: &Object<'_>,
        force: bool,
    ) -> core::result::Result<Oid, Error> {
        let name = name.as_ref();
        let mut oid = Oid::NONE;
        create_internal(
            &mut oid,
            repo.raw(),
            no_nul(name)?,
            target.header(),
            None,
            force,
        )?;
        Ok(oid)
    }

    /// `git_tag_create_from_buffer`.
    pub fn create_from_buffer(
        repo: &Repository,
        buffer: impl AsRef<[u8]>,
        force: bool,
    ) -> core::result::Result<Oid, Error> {
        let buffer = buffer.as_ref();
        let mut oid = Oid::NONE;
        create_from_buffer(&mut oid, repo.raw(), no_nul(buffer)?, force)?;
        Ok(oid)
    }

    /// `git_tag_delete`.
    pub fn delete(repo: &Repository, name: impl AsRef<[u8]>) -> core::result::Result<(), Error> {
        let name = name.as_ref();
        delete(repo.raw(), no_nul(name)?)?;
        Ok(())
    }

    /// `git_tag_list_match`: the tag names matching `pattern` (`None`: all).
    pub fn list(
        repo: &Repository,
        pattern: Option<&[u8]>,
    ) -> core::result::Result<Vec<Vec<u8>>, Error> {
        let pattern = no_nul(pattern.unwrap_or(b""))?;
        let (strings, count) = list_match(repo.raw(), pattern);
        // SAFETY: the array and strings `list_match` made, owned here.
        let arr = unsafe {
            crate::strarray::StrArray::from_raw(crate::strarray::GitStrArray { strings, count })
        };
        Ok(arr.to_vec())
    }

    /// `git_tag_foreach`: `f(refname, id)` for each tag reference; `false`
    /// stops the iteration (with `Ok`). A panic in `f` stops it and is
    /// resumed after.
    pub fn foreach(
        repo: &Repository,
        mut f: impl FnMut(&[u8], &Oid) -> bool,
    ) -> core::result::Result<(), Error> {
        let mut panic = None;
        let mut stopped = false;
        let mut cb = |n: &CStr, id: &Oid| -> c_int {
            if panic.is_some() || stopped {
                return 1;
            }
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(n.to_bytes(), id))) {
                Ok(true) => 0,
                Ok(false) => {
                    stopped = true;
                    1
                }
                Err(p) => {
                    panic = Some(p);
                    1
                }
            }
        };
        let r = foreach(repo.raw(), &mut cb);
        // A panic in `f` resumes here, after the walk let go of its state
        // (S-6d), as the other closures of the API do.
        if let Some(p) = panic {
            std::panic::resume_unwind(p);
        }
        if stopped {
            crate::util::errors::clear();
            return Ok(());
        }
        ErrorCode::check_zero(r)?;
        Ok(())
    }

    /// `git_tag_name_is_valid`.
    pub fn name_is_valid(name: impl AsRef<[u8]>) -> core::result::Result<bool, Error> {
        Ok(name_is_valid(Some(no_nul(name.as_ref())?))?)
    }
}

fn no_nul(s: &[u8]) -> core::result::Result<&[u8], Error> {
    if memchr::memchr(0, s).is_some() {
        Err(Error::from(git_error_set!(
            ErrorClass::INVALID,
            "string contains a NUL byte"
        )))
    } else {
        Ok(s)
    }
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `tag.c`.
pub mod ffi {
    use super::*;
    use crate::git_arg;
    use crate::object::GitObject;
    use crate::strarray::GitStrArray;
    use crate::util::cabi::CStrArg;
    use crate::util::export::{barrier, status};

    /// `git_tag__free`.
    ///
    /// # Safety
    ///
    /// `tag` is a tag with no reference left.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag__free(tag: *mut c_void) {
        barrier(|| {
            if let Some(t) = NonNull::new(tag.cast::<GitTag>()) {
                // SAFETY: a tag (the object table allocates them with
                // `alloc`) whose last reference is gone (this export's
                // contract).
                unsafe { free(t) }
            }
        })
    }

    /// `git_tag_target`.
    ///
    /// # Safety
    ///
    /// `target` is NULL or writable; `t` is NULL or a live tag of the
    /// object table (its repository live).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_target(
        mut target: Out<'_, *mut GitObject>,
        t: Option<&GitTag>,
    ) -> c_int {
        barrier(|| {
            let t = git_arg!(t, "t", -1);
            // As C's `git_object_lookup`: `repo` (C dereferences a NULL one
            // first), then the out-parameter.
            git_assert_arg!(!t.object.repo.is_null(), "repo", -1);
            git_assert_arg!(!target.is_null(), "object_out", -1);
            // SAFETY: a live tag whose repository is live (this export's
            // contract).
            match unsafe { super::target(NonNull::from(t)) } {
                Ok(o) => {
                    target.write(o.as_ptr());
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_tag_target_id`.
    ///
    /// # Safety
    ///
    /// `t` is NULL or a live tag.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_target_id(t: Option<&GitTag>) -> *const Oid {
        let t = git_arg!(t, "t", ptr::null());
        &raw const t.target
    }

    /// `git_tag_target_type`.
    ///
    /// # Safety
    ///
    /// `t` is NULL or a live tag.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_target_type(t: Option<&GitTag>) -> git_object_t {
        git_arg!(t, "t", GIT_OBJECT_INVALID).kind
    }

    /// `git_tag_name`.
    ///
    /// # Safety
    ///
    /// `t` is NULL or a live tag.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_name(t: Option<&GitTag>) -> *const c_char {
        git_arg!(t, "t", ptr::null()).tag_name
    }

    /// `git_tag_tagger` (NULL for a NULL tag; C crashes).
    ///
    /// # Safety
    ///
    /// `t` is NULL or a live tag.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_tagger(t: Option<&GitTag>) -> *const GitSignature {
        t.map_or(ptr::null(), |t| t.tagger)
    }

    /// `git_tag_message`.
    ///
    /// # Safety
    ///
    /// `t` is NULL or a live tag.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_message(t: Option<&GitTag>) -> *const c_char {
        git_arg!(t, "t", ptr::null()).message
    }

    /// `git_tag_create`.
    ///
    /// # Safety
    ///
    /// `oid` is NULL or a writable id that no other argument points into;
    /// `repo` NULL or live; `tag_name` and `message` NULL or NUL-terminated;
    /// `target` NULL or a live object; `tagger` NULL or a valid signature
    /// (NULL or NUL-terminated strings). All live and unwritten by others
    /// during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_create(
        oid: Option<&mut Oid>,
        repo: Option<&git_repository>,
        tag_name: CStrArg<'_>,
        target: Option<&GitObject>,
        tagger: Option<&GitSignature>,
        message: CStrArg<'_>,
        allow_ref_overwrite: c_int,
    ) -> c_int {
        barrier(|| {
            let repo = git_arg!(repo, "repo", -1);
            let tag_name = git_arg!(tag_name.get(), "tag_name", -1);
            let target = git_arg!(target, "target", -1);
            let (Some(tagger), Some(message)) = (tagger, message.get()) else {
                let _ = errors::assert_arg_failed("!create_tag_annotation || (tagger && message)");
                return -1;
            };
            // SAFETY: `tagger`'s strings are NULL or NUL-terminated, live
            // and unwritten for the call (this export's contract).
            let v = unsafe { GitSignatureView::from_c(tagger) };
            let oid = git_arg!(oid, "oid", -1);
            status(create_internal(
                oid,
                repo,
                tag_name.to_bytes(),
                target,
                Some((&v, message.to_bytes())),
                allow_ref_overwrite != 0,
            ))
        })
    }

    /// `git_tag_annotation_create`.
    ///
    /// # Safety
    ///
    /// `oid` is NULL or writable; the rest as [`git_tag_create`]'s.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_annotation_create(
        mut oid: Out<'_, Oid>,
        repo: Option<&git_repository>,
        tag_name: CStrArg<'_>,
        target: Option<&GitObject>,
        tagger: Option<&GitSignature>,
        message: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!oid.is_null(), "oid", -1);
            let repo = git_arg!(repo, "repo", -1);
            let tag_name = git_arg!(tag_name.get(), "tag_name", -1);
            let target = git_arg!(target, "target", -1);
            let tagger = git_arg!(tagger, "tagger", -1);
            let message = git_arg!(message.get(), "message", -1);
            // SAFETY: as in `git_tag_create`.
            let v = unsafe { GitSignatureView::from_c(tagger) };
            match write_annotation(repo, tag_name.to_bytes(), target, &v, message.to_bytes()) {
                Ok(id) => {
                    oid.write(id);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_tag_create_lightweight`.
    ///
    /// # Safety
    ///
    /// As [`git_tag_create`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_create_lightweight(
        oid: Option<&mut Oid>,
        repo: Option<&git_repository>,
        tag_name: CStrArg<'_>,
        target: Option<&GitObject>,
        allow_ref_overwrite: c_int,
    ) -> c_int {
        barrier(|| {
            let repo = git_arg!(repo, "repo", -1);
            let tag_name = git_arg!(tag_name.get(), "tag_name", -1);
            let target = git_arg!(target, "target", -1);
            let oid = git_arg!(oid, "oid", -1);
            status(create_internal(
                oid,
                repo,
                tag_name.to_bytes(),
                target,
                None,
                allow_ref_overwrite != 0,
            ))
        })
    }

    /// `git_tag_create_from_buffer`.
    ///
    /// # Safety
    ///
    /// `oid` is NULL or a writable id that `buffer` does not overlap;
    /// `repo` NULL or a live repository; `buffer` NULL or NUL-terminated,
    /// unwritten during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_create_from_buffer(
        oid: Option<&mut Oid>,
        repo: Option<&git_repository>,
        buffer: CStrArg<'_>,
        allow_ref_overwrite: c_int,
    ) -> c_int {
        barrier(|| {
            let oid = git_arg!(oid, "oid", -1);
            let buffer = git_arg!(buffer.get(), "buffer", -1);
            // `git_repository_odb__weakptr`'s check (the C's first use).
            let repo = git_arg!(repo, "repo", -1);
            status(create_from_buffer(
                oid,
                repo,
                buffer.to_bytes(),
                allow_ref_overwrite != 0,
            ))
        })
    }

    /// `git_tag_delete`.
    ///
    /// # Safety
    ///
    /// `repo` is NULL or a live repository; `tag_name` NULL or
    /// NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_delete(
        repo: Option<&git_repository>,
        tag_name: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let name = tag_name.to_bytes();
            // `git_reference_lookup`'s check (the C's first use).
            let repo = git_arg!(repo, "repo", -1);
            status(delete(repo, name))
        })
    }

    /// `git_tag_foreach_cb`.
    pub type GitTagForeachCb =
        Option<unsafe extern "C" fn(*const c_char, *mut Oid, *mut c_void) -> c_int>;

    /// `git_tag_foreach`.
    ///
    /// # Safety
    ///
    /// `repo` is NULL or live; `cb` NULL or a function sound to call with
    /// a reference name, a writable id and `cb_data`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_foreach(
        repo: Option<&git_repository>,
        cb: GitTagForeachCb,
        cb_data: *mut c_void,
    ) -> c_int {
        barrier(|| {
            let repo = git_arg!(repo, "repo", -1);
            let Some(cb) = cb else {
                return errors::assert_arg_failed("cb").raw();
            };
            let mut f = |name: &CStr, oid: &Oid| -> c_int {
                let mut o = *oid;
                // SAFETY: the user's callback, sound with `cb_data` (this
                // export's contract), given a NUL-terminated name and a
                // writable id (a copy) that live for the call.
                unsafe { cb(name.as_ptr(), &raw mut o, cb_data) }
            };
            foreach(repo, &mut f)
        })
    }

    /// `git_tag_list_match` (0 even when listing fails, as C).
    ///
    /// # Safety
    ///
    /// `tag_names` is NULL or writable; `pattern` NULL or NUL-terminated;
    /// `repo` NULL or live.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_list_match(
        mut tag_names: Out<'_, GitStrArray>,
        pattern: CStrArg<'_>,
        repo: Option<&git_repository>,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!tag_names.is_null(), "tag_names", -1);
            let repo = git_arg!(repo, "repo", -1);
            let pattern = git_arg!(pattern.get(), "pattern", -1);
            let (strings, count) = list_match(repo, pattern.to_bytes());
            tag_names.write(GitStrArray { strings, count });
            0
        })
    }

    /// `git_tag_list`.
    ///
    /// # Safety
    ///
    /// As [`git_tag_list_match`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_list(
        tag_names: Out<'_, GitStrArray>,
        repo: Option<&git_repository>,
    ) -> c_int {
        // SAFETY: the arguments are this export's, whose contract is
        // `git_tag_list_match`'s; the pattern is a static string.
        unsafe { git_tag_list_match(tag_names, c"".into(), repo) }
    }

    /// `git_tag_peel`.
    ///
    /// # Safety
    ///
    /// As `git_object_peel`'s: `tag_target` NULL or writable, `tag` NULL
    /// or a live tag (an object).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_peel(
        tag_target: *mut *mut GitObject,
        tag: *const GitTag,
    ) -> c_int {
        // SAFETY: this export's arguments, under `git_object_peel`'s
        // contract (a tag is an object: it starts with its header).
        unsafe { crate::object::ffi::git_object_peel(tag_target, tag.cast(), GIT_OBJECT_ANY) }
    }

    /// `git_tag_name_is_valid`.
    ///
    /// # Safety
    ///
    /// `valid` is NULL or writable; `name` NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_name_is_valid(
        mut valid: Out<'_, c_int>,
        name: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            git_assert!(!valid.is_null(), "valid", -1);
            valid.write(0);
            let name = name.get().map(CStr::to_bytes);
            match name_is_valid(name) {
                Ok(v) => {
                    valid.write(c_int::from(v));
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }
}

crate::c_layout_test! {
    headers: ["tag.h"],
    types: [
        GitTag => "struct git_tag" { object, target, kind as "type", tag_name, tagger, message },
    ],
    asserts: [],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support;

    const OK: &[u8] = b"object e90810b8df3e80c413d903f631643c716887138d\n\
type commit\n\
tag v1.0\n\
tagger Vicent Marti <vicent@github.com> 1288977662 +0100\n\
\n\
This is a test tag\n";

    fn err(buf: &[u8]) -> (i32, Vec<u8>) {
        let e = parse(buf, 1).unwrap_err();
        (e.raw(), errors::last_message().unwrap_or_default())
    }

    #[test]
    fn parses_a_tag() {
        let _g = test_support::init();
        let (target, d) = parse(OK, 1).unwrap();
        assert_eq!(
            target.hex().as_str(),
            "e90810b8df3e80c413d903f631643c716887138d"
        );
        assert_eq!(d.kind, object::GIT_OBJECT_COMMIT);
        assert_eq!(d.name.unwrap().as_c_str().to_bytes(), b"v1.0");
        assert_eq!(d.tagger.unwrap().name(), b"Vicent Marti");
        assert_eq!(
            d.message.unwrap().as_c_str().to_bytes(),
            b"This is a test tag\n"
        );
    }

    #[test]
    fn tag_errors_are_cs() {
        let _g = test_support::init();
        assert_eq!(
            err(b"obj"),
            (-21, b"failed to parse tag: object field invalid".to_vec())
        );
        let mut b = OK[..48].to_vec();
        assert_eq!(
            err(&b),
            (-21, b"failed to parse tag: object too short".to_vec())
        );
        b.extend_from_slice(b"typo commit\n");
        assert_eq!(err(&b).1, b"failed to parse tag: type field not found");
        let b = [&OK[..48], b"type tag\nx"].concat();
        // "commit\n" does not fit: "object too short", as C.
        assert_eq!(err(&b).1, b"failed to parse tag: object too short");
        let b = [&OK[..48], b"type wtf\nxxxxxxxx"].concat();
        assert_eq!(err(&b).1, b"failed to parse tag: invalid object type");
        let b = [&OK[..48], b"type blob\nta v1\n"].concat();
        assert_eq!(err(&b).1, b"failed to parse tag: tag field not found");
        let b = [&OK[..48], b"type blob\ntag v1"].concat();
        assert_eq!(err(&b).1, b"failed to parse tag: object too short");
        let b = [
            &OK[..48],
            b"type blob\ntag v1\ntagger a <b> 1 +0000\nno message",
        ]
        .concat();
        assert_eq!(err(&b).1, b"failed to parse tag: tag contains no message");
        let b = [&OK[..48], b"type blob\ntag v1\ntagger a b\n\n"].concat();
        assert_eq!(err(&b).1, b"failed to parse signature - malformed e-mail");
        // No tagger, no message.
        let b = [&OK[..48], b"type blob\ntag v1\n"].concat();
        let (_, d) = parse(&b, 1).unwrap();
        assert!(d.tagger.is_none() && d.message.is_none());
        // No tagger, an empty message.
        let b = [&OK[..48], b"type tree\ntag v1\n\n"].concat();
        let (_, d) = parse(&b, 1).unwrap();
        assert_eq!(d.message.unwrap().as_c_str().to_bytes(), b"");
        assert_eq!(err(OK.split_at(10).0).0, -21);
        assert!(parse(OK, 0).is_err());
    }

    #[test]
    fn blank_lines() {
        assert_eq!(find_blank_line(b"a\n\nb"), Some(1));
        assert_eq!(find_blank_line(b"a\nb\n"), None);
        assert_eq!(find_blank_line(b"\n\n"), Some(0));
        assert_eq!(find_blank_line(b"x\n"), None);
        assert_eq!(find_blank_line(b""), None);
    }

    #[test]
    fn tag_names() {
        assert!(name_is_valid_basic(b"v1"));
        assert!(!name_is_valid_basic(b"-v1"));
        assert!(!name_is_valid_basic(b"HEAD"));
        assert!(name_is_valid_basic(b""));
    }

    /// NULL arguments that C checks (or reaches through a checking callee)
    /// fail with C's `invalid argument` error, before anything is read.
    #[test]
    fn null_arguments_are_invalid() {
        let _g = test_support::init();
        let mut oid = Oid::NONE;
        // SAFETY: a writable id; NULL repository; a string.
        let r = unsafe { ffi::git_tag_create_from_buffer(Some(&mut oid), None, c"x".into(), 0) };
        assert_eq!(r, -1);
        assert_eq!(errors::last_message().unwrap(), b"invalid argument: 'repo'");
        // SAFETY: as above.
        let r = unsafe { ffi::git_tag_delete(None, c"x".into()) };
        assert_eq!(r, -1);
        assert_eq!(errors::last_message().unwrap(), b"invalid argument: 'repo'");
        // SAFETY: NULL tag.
        assert!(unsafe { ffi::git_tag_target_id(None) }.is_null());
        assert_eq!(errors::last_message().unwrap(), b"invalid argument: 't'");
        // SAFETY: NULL out-parameter.
        let r = unsafe { ffi::git_tag_name_is_valid(Out::null(), c"v1".into()) };
        assert_eq!(r, -1);
    }

    #[test]
    fn raw_tag_object() {
        let _g = test_support::init();
        let t = alloc().unwrap();
        // SAFETY: a fresh tag.
        assert_eq!(unsafe { parse_into(t, OK, 1) }, 0);
        // SAFETY: a live tag.
        let r = unsafe { t.as_ref() };
        assert_eq!(r.name(), b"v1.0");
        // SAFETY: a live tag, read by the export as C would.
        let tagger = unsafe { ffi::git_tag_tagger(Some(r)) };
        // SAFETY: the tagger lives as long as the tag.
        let email = unsafe { (*tagger).email };
        // SAFETY: a NUL-terminated string of the tagger.
        let email = unsafe { CStr::from_ptr(email) };
        assert_eq!(email.to_bytes(), b"vicent@github.com");
        // SAFETY: the tag made above, freed once.
        unsafe { free(t) };
    }
}
