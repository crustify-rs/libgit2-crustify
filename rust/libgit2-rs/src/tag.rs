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

use crate::util::cabi::CStrArg;
use crate::util::cabi::Out;
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
/// The fields are the tag's own (the type's invariant), not used again.
unsafe fn free_fields(t: &mut GitTag) {
    // SAFETY: the caller's contract.
    unsafe {
        signature::free_raw(t.tagger);
    }
    // SAFETY: as above.
    unsafe {
        alloc::free(t.message.cast());
    }
    // SAFETY: as above.
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
    // SAFETY: the tag, no longer shared (the caller's contract).
    let t = unsafe { &mut *tag.as_ptr() };
    // SAFETY: its own fields, freed once.
    unsafe { free_fields(t) };
    // SAFETY: its block, freed once.
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
            // SAFETY: a fresh, unshared tag.
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
    // SAFETY: a live odb object (its `git_cached_obj` prefix); the size is
    // read only.
    let size = unsafe { (*c).size };
    // SAFETY: `odb_obj` is NULL or a live odb object (as the call below requires), not written
    // once shared but for its atomic count.
    let odb_obj_ref = unsafe { odb_obj.as_ref() };
    // SAFETY: a live odb object.
    let d = unsafe { p::git_odb_object_data(odb_obj_ref) };
    // SAFETY: its buffer holds `size` bytes, unwritten, for its life.
    let data = unsafe { crate::util::export::bytes(d.cast(), size) };
    // SAFETY: a fresh tag.
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
        // SAFETY: NULL or the tag's own signature block, whose strings it
        // owns (a `Signature`'s invariant; `Signature` is transparent over
        // `git_signature`).
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
/// `tag` is a live tag.
pub unsafe fn target(tag: NonNull<GitTag>) -> Result<NonNull<GitObject>> {
    // SAFETY: a live tag, read only.
    let t = unsafe { tag.as_ref() };
    // (C dereferences a NULL repository: a tag parsed from raw data.)
    git_assert!(!t.object.repo.is_null(), "t->object.repo");
    // SAFETY: the tag's live repository.
    object::lookup(unsafe { &*t.object.repo }, &t.target, t.kind)
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
///
/// # Safety
///
/// `repo` is a live repository.
unsafe fn retrieve_oid(
    oid: *mut Oid,
    refname: &mut GitStr,
    repo: *mut git_repository,
    tag_name: &[u8],
) -> c_int {
    match ref_name(tag_name) {
        Ok(s) => *refname = s,
        Err(e) => return e.raw(),
    }
    // SAFETY: a live repository, a NUL-terminated name, a writable id.
    unsafe { p::git_reference_name_to_id(oid, repo, as_c(refname)) }
}

/// `write_tag_annotation`: write the tag object; its id.
///
/// # Safety
///
/// `repo` is a live repository; `target` a live object.
pub unsafe fn write_annotation(
    repo: *mut git_repository,
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
        // SAFETY: a live repository, only read through `&` (its state is
        // interior-mutable).
        let odb = object::repo_odb_weak(unsafe { &*repo })?;
        crate::blob::odb_write(odb, tag.as_bytes(), GIT_OBJECT_TAG)
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
    /// `sig` is a valid signature whose strings live for `'a`.
    pub unsafe fn from_c(sig: *const GitSignature) -> GitSignatureView<'a> {
        // SAFETY: valid, read by value.
        let s = unsafe { sig.read() };
        let b = |p: *mut c_char| -> &'a [u8] {
            if p.is_null() {
                b"(null)"
            } else {
                // SAFETY: NUL-terminated, live for `'a`.
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
/// to a new annotation of it); the reference's target id.
///
/// # Safety
///
/// `repo` is a live repository; `target` a live object; `oid` writable (as
/// C, it may be written on failure: an existing tag's target).
pub unsafe fn create_internal(
    oid: *mut Oid,
    repo: *mut git_repository,
    tag_name: &[u8],
    target: &GitObject,
    annotation: Option<(&GitSignatureView<'_>, &[u8])>,
    allow_ref_overwrite: bool,
) -> Result<()> {
    if target.repo != repo {
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
    // SAFETY: a live repository.
    let error = unsafe { retrieve_oid(oid, &mut refname, repo, tag_name) };
    if error < 0 && error != ErrorCode::NOT_FOUND.raw() {
        return Err(ErrorCode::from_raw(error).unwrap_or(ErrorCode::GENERIC));
    }
    // The name must not conflict with an existing reference, unless asked.
    if error == 0 && !allow_ref_overwrite {
        let _ = git_error_set!(ErrorClass::TAG, "tag already exists");
        return Err(ErrorCode::EXISTS);
    }
    match annotation {
        Some((tagger, message)) => {
            // SAFETY: the caller's contract.
            let id = unsafe { write_annotation(repo, tag_name, target, tagger, message) }
                .map_err(|_| ErrorCode::GENERIC)?;
            // SAFETY: writable.
            unsafe { oid.write(id) };
        }
        // SAFETY: writable.
        None => unsafe { oid.write(*target.id()) },
    }
    let mut new_ref = RefHandle(ptr::null_mut());
    // SAFETY: a live repository, a NUL-terminated name, a valid id.
    let r = unsafe {
        p::git_reference_create(
            &raw mut new_ref.0,
            repo,
            as_c(&refname),
            oid,
            c_int::from(allow_ref_overwrite),
            CStrArg::NULL,
        )
    };
    ErrorCode::check(r).map(|_| ())
}

/// `git_tag_create_from_buffer`: validate the tag in `buffer`, write it,
/// and create its reference.
///
/// # Safety
///
/// `repo` is a live repository; `oid` writable.
pub unsafe fn create_from_buffer(
    oid: *mut Oid,
    repo: *mut git_repository,
    buffer: &[u8],
    allow_ref_overwrite: bool,
) -> Result<()> {
    // SAFETY: a live repository.
    let odb = object::repo_odb_weak(unsafe { &*repo }).map_err(|_| ErrorCode::GENERIC)?;
    // Validate the buffer (C leaks what it parsed when this fails:
    // rust/DEFECTS.md).
    // SAFETY: as above.
    let oid_type = object::repo_oid_type(unsafe { &*repo });
    let (target, tag) = parse(buffer, oid_type).map_err(|_| ErrorCode::GENERIC)?;
    let mut refname = GitStr::new();
    let error = {
        // Validate the target.
        let mut target_obj = ptr::null_mut();
        // SAFETY: a live odb, a valid id, a writable out.
        if unsafe { p::git_odb_read(Out::from(&mut target_obj), Some(odb), Some(&target)) } < 0 {
            return Err(ErrorCode::GENERIC);
        }
        /// The odb object, freed on drop.
        struct Held(*mut p::git_odb_object);
        impl Drop for Held {
            fn drop(&mut self) {
                // SAFETY: the reference this value holds.
                unsafe { p::git_odb_object_free(self.0) }
            }
        }
        let held = Held(target_obj);
        // SAFETY: a live object's `git_cached_obj` prefix.
        let ttype = git_object_t::from(unsafe { (*held.0.cast::<object::GitCachedObj>()).type_ });
        if tag.kind != ttype {
            let _ = git_error_set!(ErrorClass::TAG, "the type for the given target is invalid");
            return Err(ErrorCode::GENERIC);
        }
        let name = tag
            .name
            .as_ref()
            .map_or(&[][..], |n| n.as_c_str().to_bytes());
        // SAFETY: a live repository.
        let error = unsafe { retrieve_oid(oid, &mut refname, repo, name) };
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
    // SAFETY: a live odb, a writable out.
    ErrorCode::check(unsafe {
        p::git_odb_open_wstream(
            Out::from(&mut stream),
            Some(odb),
            buffer.len() as u64,
            GIT_OBJECT_TAG,
        )
    })?;
    // SAFETY: a live stream, readable data.
    let mut error =
        unsafe { p::git_odb_stream_write(stream, buffer.as_ptr().cast(), buffer.len()) };
    if error == 0 {
        // SAFETY: a live stream, a writable id.
        error = unsafe { p::git_odb_stream_finalize_write(oid, stream) };
    }
    // SAFETY: the stream, freed once.
    unsafe { p::git_odb_stream_free(stream) };
    ErrorCode::check(error)?;
    let mut new_ref = RefHandle(ptr::null_mut());
    // SAFETY: a live repository, a NUL-terminated name, a valid id.
    let r = unsafe {
        p::git_reference_create(
            &raw mut new_ref.0,
            repo,
            as_c(&refname),
            oid,
            c_int::from(allow_ref_overwrite),
            CStrArg::NULL,
        )
    };
    ErrorCode::check(r).map(|_| ())
}

/// `git_tag_delete`: delete `refs/tags/<name>`.
///
/// # Safety
///
/// `repo` is a live repository.
pub unsafe fn delete(repo: *mut git_repository, tag_name: &[u8]) -> Result<()> {
    let refname = ref_name(tag_name).map_err(|_| ErrorCode::GENERIC)?;
    let mut r = RefHandle(ptr::null_mut());
    // SAFETY: a live repository, a NUL-terminated name, a writable out.
    ErrorCode::check(unsafe { p::git_reference_lookup(&raw mut r.0, repo, as_c(&refname)) })?;
    // SAFETY: a live reference.
    ErrorCode::check(unsafe { p::git_reference_delete(r.0) }).map(|_| ())
}

/// `git_tag_foreach`: call `cb(refname, id)` for each tag reference; a
/// non-zero return stops, and is returned (with
/// `git_error_set_after_callback_function`).
///
/// # Safety
///
/// `repo` is a live repository.
pub unsafe fn foreach(
    repo: *mut git_repository,
    cb: &mut dyn FnMut(&CStr, &Oid) -> c_int,
) -> c_int {
    struct Data<'a> {
        repo: *mut git_repository,
        cb: &'a mut dyn FnMut(&CStr, &Oid) -> c_int,
    }
    unsafe extern "C" fn tags_cb(name: *const c_char, payload: *mut c_void) -> c_int {
        crate::util::export::barrier(|| {
            // SAFETY: `payload` is the `Data` below, alive and borrowed by
            // nothing else during the iteration.
            let d = unsafe { &mut *payload.cast::<Data<'_>>() };
            // SAFETY: a reference name, NUL-terminated.
            let name = unsafe { CStr::from_ptr(name) };
            if crate::util::util::prefixcmp(name.to_bytes(), GIT_REFS_TAGS_DIR) != 0 {
                return 0; // not a tag
            }
            let mut oid = Oid::NONE;
            // SAFETY: a live repository, a NUL-terminated name, a writable id.
            let mut error =
                unsafe { p::git_reference_name_to_id(&raw mut oid, d.repo, name.as_ptr()) };
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
    // SAFETY: a live repository; the callback and its payload live for the
    // call.
    unsafe {
        p::git_reference_foreach_name(repo, tags_cb, crate::util::export::payload(&raw mut data))
    }
}

/// `git_tag_list_match`: the names of the tags matching `pattern` (all,
/// when empty), as `git_strarray`'s `strings` and `count`. Failures give an
/// empty list (C returns 0 anyway).
///
/// # Safety
///
/// `repo` is a live repository.
pub unsafe fn list_match(repo: *mut git_repository, pattern: &[u8]) -> (*mut *mut c_char, usize) {
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
    // SAFETY: a live repository.
    let error = unsafe { foreach(repo, &mut cb) };
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
        // SAFETY: a live tag of a repository.
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
        // SAFETY: a live repository and object (`create_internal` checks
        // that the object is the repository's).
        unsafe {
            create_internal(
                &raw mut oid,
                crate::object::repo_ptr(repo),
                no_nul(name)?,
                target.header(),
                Some((&v, no_nul(message)?)),
                force,
            )?
        };
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
        // SAFETY: a live repository and object.
        Ok(unsafe {
            write_annotation(
                crate::object::repo_ptr(repo),
                no_nul(name)?,
                target.header(),
                &v,
                no_nul(message)?,
            )?
        })
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
        // SAFETY: a live repository and object.
        unsafe {
            create_internal(
                &raw mut oid,
                crate::object::repo_ptr(repo),
                no_nul(name)?,
                target.header(),
                None,
                force,
            )?
        };
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
        // SAFETY: a live repository.
        unsafe {
            create_from_buffer(
                &raw mut oid,
                crate::object::repo_ptr(repo),
                no_nul(buffer)?,
                force,
            )?
        };
        Ok(oid)
    }

    /// `git_tag_delete`.
    pub fn delete(repo: &Repository, name: impl AsRef<[u8]>) -> core::result::Result<(), Error> {
        let name = name.as_ref();
        // SAFETY: a live repository.
        let _: () = unsafe { delete(crate::object::repo_ptr(repo), no_nul(name)?)? };
        Ok(())
    }

    /// `git_tag_list_match`: the tag names matching `pattern` (`None`: all).
    pub fn list(
        repo: &Repository,
        pattern: Option<&[u8]>,
    ) -> core::result::Result<Vec<Vec<u8>>, Error> {
        let pattern = no_nul(pattern.unwrap_or(b""))?;
        // SAFETY: a live repository.
        let (strings, count) = unsafe { list_match(crate::object::repo_ptr(repo), pattern) };
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
        // SAFETY: a live repository.
        let r = unsafe { foreach(crate::object::repo_ptr(repo), &mut cb) };
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
    use crate::util::export::{barrier, cstr_bytes, status};

    /// `git_tag__free`.
    ///
    /// # Safety
    ///
    /// `tag` is a tag with no reference left.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag__free(tag: *mut c_void) {
        barrier(|| {
            if let Some(t) = NonNull::new(tag.cast::<GitTag>()) {
                // SAFETY: the caller's contract.
                unsafe { free(t) }
            }
        })
    }

    /// `git_tag_target`.
    ///
    /// # Safety
    ///
    /// `target` is writable; `t` NULL or a live tag.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_target(
        target: *mut *mut GitObject,
        t: *const GitTag,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!t.is_null(), "t", -1);
            // SAFETY: non-null (checked above).
            let nn = unsafe { NonNull::new_unchecked(t.cast_mut()) };
            // As C's `git_object_lookup`: `repo` (C dereferences a NULL one
            // first), then the out-parameter.
            // SAFETY: a live tag, read only.
            git_assert_arg!(!unsafe { nn.as_ref() }.object.repo.is_null(), "repo", -1);
            git_assert_arg!(!target.is_null(), "object_out", -1);
            // SAFETY: a live tag.
            match unsafe { super::target(nn) } {
                Ok(o) => {
                    // SAFETY: writable.
                    unsafe { target.write(o.as_ptr()) };
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
    pub unsafe extern "C" fn git_tag_target_id(t: *const GitTag) -> *const Oid {
        git_assert_arg!(!t.is_null(), "t", ptr::null());
        // SAFETY: a field of a live tag.
        unsafe { &raw const (*t).target }
    }

    /// `git_tag_target_type`.
    ///
    /// # Safety
    ///
    /// `t` is NULL or a live tag.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_target_type(t: *const GitTag) -> git_object_t {
        git_assert_arg!(!t.is_null(), "t", GIT_OBJECT_INVALID);
        // SAFETY: a live tag, read only.
        unsafe { (*t).kind }
    }

    /// `git_tag_name`.
    ///
    /// # Safety
    ///
    /// `t` is NULL or a live tag.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_name(t: *const GitTag) -> *const c_char {
        git_assert_arg!(!t.is_null(), "t", ptr::null());
        // SAFETY: a live tag, read only.
        unsafe { (*t).tag_name }
    }

    /// `git_tag_tagger` (NULL for a NULL tag; C crashes).
    ///
    /// # Safety
    ///
    /// `t` is NULL or a live tag.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_tagger(t: *const GitTag) -> *const GitSignature {
        if t.is_null() {
            return ptr::null();
        }
        // SAFETY: a live tag, read only.
        unsafe { (*t).tagger }
    }

    /// `git_tag_message`.
    ///
    /// # Safety
    ///
    /// `t` is NULL or a live tag.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_message(t: *const GitTag) -> *const c_char {
        git_assert_arg!(!t.is_null(), "t", ptr::null());
        // SAFETY: a live tag, read only.
        unsafe { (*t).message }
    }

    /// The arguments common to the create functions.
    ///
    /// # Safety
    ///
    /// `target` is NULL or a live object.
    unsafe fn target_ref<'a>(target: *const GitObject) -> Option<&'a GitObject> {
        // SAFETY: NULL or live (read only once published).
        unsafe { target.as_ref() }
    }

    fn write_id(oid: *mut Oid, local: &Oid) {
        if !oid.is_null() {
            // SAFETY: the exports' callers pass a writable (or NULL) id.
            unsafe { oid.write(*local) };
        }
    }

    /// `git_tag_create`.
    ///
    /// # Safety
    ///
    /// `oid` is writable; `repo` NULL or live; `tag_name` and `message`
    /// NULL or NUL-terminated; `target` NULL or a live object; `tagger`
    /// NULL or a valid signature.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_create(
        oid: *mut Oid,
        repo: *mut git_repository,
        tag_name: CStrArg<'_>,
        target: *const GitObject,
        tagger: *const GitSignature,
        message: *const c_char,
        allow_ref_overwrite: c_int,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!repo.is_null(), "repo", -1);
            let tag_name = git_arg!(tag_name.get(), "tag_name", -1);
            git_assert_arg!(!target.is_null(), "target", -1);
            git_assert_arg!(
                !tagger.is_null() && !message.is_null(),
                "!create_tag_annotation || (tagger && message)",
                -1
            );
            let name = tag_name.to_bytes();
            // SAFETY: as above.
            let msg = unsafe { cstr_bytes(message) };
            // SAFETY: a live object.
            let t = unsafe { target_ref(target) };
            // SAFETY: a valid signature, live for the call.
            let v = unsafe { GitSignatureView::from_c(tagger) };
            let Some(t) = t else { return -1 };
            git_assert_arg!(!oid.is_null(), "oid", -1);
            // SAFETY: a live repository and object; `oid` writable.
            status(unsafe {
                create_internal(
                    oid,
                    repo,
                    name,
                    t,
                    Some((&v, msg)),
                    allow_ref_overwrite != 0,
                )
            })
        })
    }

    /// `git_tag_annotation_create`.
    ///
    /// # Safety
    ///
    /// As [`git_tag_create`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_annotation_create(
        oid: *mut Oid,
        repo: *mut git_repository,
        tag_name: CStrArg<'_>,
        target: Option<&GitObject>,
        tagger: *const GitSignature,
        message: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!oid.is_null(), "oid", -1);
            git_assert_arg!(!repo.is_null(), "repo", -1);
            let tag_name = git_arg!(tag_name.get(), "tag_name", -1);
            let target = git_arg!(target, "target", -1);
            git_assert_arg!(!tagger.is_null(), "tagger", -1);
            let message = git_arg!(message.get(), "message", -1);
            let name = tag_name.to_bytes();
            let msg = message.to_bytes();
            let t = target;
            // SAFETY: a valid signature.
            let v = unsafe { GitSignatureView::from_c(tagger) };
            // SAFETY: a live repository and object.
            match unsafe { write_annotation(repo, name, t, &v, msg) } {
                Ok(id) => {
                    write_id(oid, &id);
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
        oid: *mut Oid,
        repo: *mut git_repository,
        tag_name: CStrArg<'_>,
        target: Option<&GitObject>,
        allow_ref_overwrite: c_int,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!repo.is_null(), "repo", -1);
            let tag_name = git_arg!(tag_name.get(), "tag_name", -1);
            let target = git_arg!(target, "target", -1);
            let name = tag_name.to_bytes();
            let t = target;
            git_assert_arg!(!oid.is_null(), "oid", -1);
            // SAFETY: a live repository and object; `oid` writable.
            status(unsafe { create_internal(oid, repo, name, t, None, allow_ref_overwrite != 0) })
        })
    }

    /// `git_tag_create_from_buffer`.
    ///
    /// # Safety
    ///
    /// `oid` is NULL or writable; `repo` a live repository; `buffer` NULL
    /// or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_create_from_buffer(
        oid: *mut Oid,
        repo: *mut git_repository,
        buffer: CStrArg<'_>,
        allow_ref_overwrite: c_int,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!oid.is_null(), "oid", -1);
            let buffer = git_arg!(buffer.get(), "buffer", -1);
            let b = buffer.to_bytes();
            // SAFETY: a live repository; `oid` writable.
            status(unsafe { create_from_buffer(oid, repo, b, allow_ref_overwrite != 0) })
        })
    }

    /// `git_tag_delete`.
    ///
    /// # Safety
    ///
    /// `repo` is a live repository; `tag_name` NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_delete(
        repo: *mut git_repository,
        tag_name: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let name = tag_name.to_bytes();
            // SAFETY: a live repository.
            status(unsafe { delete(repo, name) })
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
    /// `cb_data`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_foreach(
        repo: *mut git_repository,
        cb: GitTagForeachCb,
        cb_data: *mut c_void,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!repo.is_null(), "repo", -1);
            let Some(cb) = cb else {
                return errors::assert_arg_failed("cb").raw();
            };
            let mut f = |name: &CStr, oid: &Oid| -> c_int {
                let mut o = *oid;
                // SAFETY: the user's callback, with its payload, a
                // NUL-terminated name and a writable id (a copy).
                unsafe { cb(name.as_ptr(), &raw mut o, cb_data) }
            };
            // SAFETY: a live repository.
            unsafe { foreach(repo, &mut f) }
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
        tag_names: *mut GitStrArray,
        pattern: CStrArg<'_>,
        repo: *mut git_repository,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!tag_names.is_null(), "tag_names", -1);
            git_assert_arg!(!repo.is_null(), "repo", -1);
            let pattern = git_arg!(pattern.get(), "pattern", -1);
            let pattern = pattern.to_bytes();
            // SAFETY: a live repository.
            let (strings, count) = unsafe { list_match(repo, pattern) };
            // SAFETY: writable.
            unsafe { tag_names.write(GitStrArray { strings, count }) };
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
        tag_names: *mut GitStrArray,
        repo: *mut git_repository,
    ) -> c_int {
        // SAFETY: forwarded.
        unsafe { git_tag_list_match(tag_names, c"".into(), repo) }
    }

    /// `git_tag_peel`.
    ///
    /// # Safety
    ///
    /// As `git_object_peel`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_peel(
        tag_target: *mut *mut GitObject,
        tag: *const GitTag,
    ) -> c_int {
        // SAFETY: `tag_target` is NULL or writable (as the call below requires), and no reference
        // to it is live.
        let tag_target_out = unsafe { Out::from_ptr(tag_target) };
        // SAFETY: forwarded.
        unsafe { crate::object::ffi::git_object_peel(tag_target_out, tag.cast(), GIT_OBJECT_ANY) }
    }

    /// `git_tag_name_is_valid`.
    ///
    /// # Safety
    ///
    /// `valid` is NULL or writable; `name` NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tag_name_is_valid(valid: *mut c_int, name: CStrArg<'_>) -> c_int {
        barrier(|| {
            git_assert!(!valid.is_null(), "valid", -1);
            // SAFETY: writable.
            unsafe { valid.write(0) };
            let name = name.get().map(CStr::to_bytes);
            match name_is_valid(name) {
                Ok(v) => {
                    // SAFETY: writable.
                    unsafe { valid.write(c_int::from(v)) };
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
        let tagger = unsafe { ffi::git_tag_tagger(t.as_ptr()) };
        // SAFETY: the tagger lives as long as the tag.
        let email = unsafe { (*tagger).email };
        // SAFETY: a NUL-terminated string of the tagger.
        let email = unsafe { CStr::from_ptr(email) };
        assert_eq!(email.to_bytes(), b"vicent@github.com");
        // SAFETY: the tag made above, freed once.
        unsafe { free(t) };
    }
}
