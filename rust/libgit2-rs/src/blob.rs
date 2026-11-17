//! Port of `src/libgit2/blob.c`: blobs (their contents, from the odb or
//! from raw data), creating them from a buffer, a work-tree file, a file on
//! disk or a stream, binary detection, and filtering (`filter.c`, which
//! stays C).
//!
//! # Types
//!
//! - [`GitBlob`] is `struct git_blob`: a shared prefix (kind 2). C sees
//!   only the [`GitObject`] header (through casts: the cache, `filter.c`
//!   calls the public accessors); the rest is Rust's, and `blob.h` keeps
//!   only the typedef under `GIT_RUST`. Blobs are allocated with
//!   `git__allocator` ([`alloc`]) and freed by [`free`], through the object
//!   table.
//! - [`GitWriteStream`] is the public `git_writestream` (kind 3): the
//!   stream [`ffi::git_blob_create_from_stream`] returns is a ffibox
//!   [`CTable`] over [`BlobWriteStream`], released through its `free`
//!   slot.
//! - [`Blob`] is the safe API's blob: an [`Object`] of type blob.

use crate::util::cabi::CStrArg;
use crate::util::cabi::Out;
use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::{self, NonNull};

use ffibox::vtable::CTable;
use ffibox::{define_ctype, impl_cvtable};

use crate::buf::GitBuf;
use crate::object::deps::{self as p, git_odb, git_odb_object, git_odb_stream, git_repository};
use crate::object::{self, GIT_OBJECT_BLOB, GitObject, Object, ObjectType, Repository};
use crate::oid::Oid;
use crate::sys::blob_filter::{self as cfilter, FilterList};
use crate::util::alloc::{self, GitCStr};
use crate::util::errors::{self, Bytes, Error, ErrorClass, ErrorCode, Result};
use crate::util::filebuf::{GIT_FILEBUF_TEMPORARY, GitFilebuf};
use crate::util::futils;
use crate::util::posix;
use crate::util::str::GitStr;
use crate::{git_assert_arg, git_error_set};

/// The contents of a blob.
#[derive(Debug)]
enum BlobData {
    /// From the odb: the odb object (one reference held) and its buffer.
    Odb {
        obj: NonNull<git_odb_object>,
        data: *const u8,
        size: usize,
    },
    /// Raw data the blob points into (`git_object__from_raw`).
    Raw { data: *const u8, size: usize },
}

/// `struct git_blob`: the object header, then Rust's state.
#[repr(C)]
#[derive(Debug)]
pub struct GitBlob {
    /// The object header (C-visible).
    pub object: GitObject,
    data: BlobData,
}

// The header is first (C casts a `git_blob *` to `git_object *`), and the
// alignment is what `git__allocator` gives.
const _: () = assert!(core::mem::offset_of!(GitBlob, object) == 0);
const _: () = assert!(align_of::<GitBlob>() <= 8);

/// Allocate an empty blob (`git__calloc(1, sizeof(git_blob))`).
pub(crate) fn alloc() -> Result<NonNull<GitBlob>> {
    let p = NonNull::new(alloc::malloc(size_of::<GitBlob>()).cast::<GitBlob>())
        .ok_or(ErrorCode::GENERIC)?;
    // SAFETY: a fresh `git__allocator` block of a `GitBlob`'s size;
    // `git__allocator` memory is malloc-aligned, enough for `GitBlob`
    // (`align_of <= 8`, asserted below).
    unsafe {
        p.as_ptr().write(GitBlob {
            object: GitObject::zeroed(),
            data: BlobData::Raw {
                data: ptr::null(),
                size: 0,
            },
        })
    };
    Ok(p)
}

/// `git_blob__parse`: the blob holds `odb_obj` (a new reference).
///
/// # Safety
///
/// `blob` is a fresh blob; `odb_obj` a live odb object.
pub(crate) unsafe fn parse(blob: NonNull<GitBlob>, odb_obj: *mut git_odb_object) -> c_int {
    let Some(obj) = NonNull::new(odb_obj) else {
        return errors::assert_arg_failed("obj").raw();
    };
    // SAFETY: a live odb object (this fn's contract), not written once
    // shared but for its atomic count; the `&` ends with this fn.
    let o = unsafe { obj.as_ref() };
    // `git_cached_obj_incref`: the blob's reference.
    o.cached.refcount.inc();
    // Its buffer lives as long as the object, which the blob now holds.
    let (data, size) = (o.buffer.cast::<u8>().cast_const(), o.cached.size);
    // SAFETY: a fresh blob that no one else reaches (this fn's contract):
    // a place write (the old value, `alloc`'s empty raw data, needs no
    // drop).
    unsafe { (*blob.as_ptr()).data = BlobData::Odb { obj, data, size } };
    0
}

/// `git_blob__parse_raw`: the blob points into `data`.
///
/// # Safety
///
/// `blob` is a fresh blob; `data` is readable for `size` bytes for the
/// blob's life.
pub(crate) unsafe fn parse_raw(blob: NonNull<GitBlob>, data: *const u8, size: usize) {
    // SAFETY: a fresh blob, not shared yet.
    unsafe { (*blob.as_ptr()).data = BlobData::Raw { data, size } };
}

/// `git_blob__free`.
///
/// # Safety
///
/// `blob` is a blob of [`alloc`]'s, with no reference left.
pub(crate) unsafe fn free(blob: NonNull<GitBlob>) {
    // SAFETY: the blob, no longer shared, read once.
    let b = unsafe { blob.as_ptr().read() };
    if let BlobData::Odb { obj, .. } = b.data {
        // SAFETY: the reference the blob held.
        unsafe { p::git_odb_object_free(obj.as_ptr()) };
    }
    // SAFETY: the blob's block, freed once.
    unsafe { alloc::free(blob.as_ptr().cast()) };
}

impl GitBlob {
    /// The contents (`git_blob_rawcontent`, `git_blob_rawsize`).
    pub fn content(&self) -> &[u8] {
        let (data, size) = match self.data {
            BlobData::Odb { data, size, .. } | BlobData::Raw { data, size } => (data, size),
        };
        // SAFETY: the odb object's buffer (held by the blob) or the raw
        // data (which outlives the blob, `from_raw`'s contract), read only.
        unsafe { crate::util::export::bytes(data.cast(), size) }
    }

    fn raw_parts(&self) -> (*const u8, usize) {
        match self.data {
            BlobData::Odb { data, size, .. } | BlobData::Raw { data, size } => (data, size),
        }
    }
}

/// `git_blob_is_binary` of contents: the first 8000 bytes.
pub fn content_is_binary(content: &[u8]) -> bool {
    let n = content.len().min(cfilter::GIT_FILTER_BYTES_TO_CHECK_NUL);
    crate::util::str::is_binary(&content[..n])
}

/// `GIT_ERROR_CHECK_BLOBSIZE` then `git_str_set`: the blob's contents into
/// `buffer` (`git_blob__getbuf`).
pub fn getbuf(buffer: &mut GitStr, blob: &GitBlob) -> Result<()> {
    buffer.set(blob.content())
}

// ---------------------------------------------------------------------------
// Creating blobs
// ---------------------------------------------------------------------------

/// A held odb write stream, freed on drop.
///
/// Invariant: NULL (a backend passed through, leaving C's `*stream`
/// unset), or a live stream that this value alone uses and frees.
struct OdbStream(*mut git_odb_stream);

impl OdbStream {
    /// `git_odb_open_wstream`.
    fn open(odb: &git_odb, size: u64, t: c_int) -> Result<OdbStream> {
        match odb.open_wstream(size, t) {
            Ok(s) => Ok(OdbStream(s.as_ptr())),
            // C: 0, with the stream left NULL (`git_odb_open_wstream`).
            Err(ErrorCode::PASSTHROUGH) => Ok(OdbStream(ptr::null_mut())),
            Err(e) => Err(e),
        }
    }

    /// `git_odb_stream_write`: its status.
    fn write(&mut self, data: &[u8]) -> c_int {
        if self.0.is_null() {
            // (C dereferences the NULL stream.)
            return errors::assert_arg_failed("stream").raw();
        }
        // SAFETY: the live stream this value alone uses (the invariant);
        // `data` is readable for its length and not written meanwhile.
        unsafe { p::git_odb_stream_write(self.0, data.as_ptr().cast(), data.len()) }
    }

    /// `git_odb_stream_finalize_write`.
    fn finalize(&mut self, out: &mut Oid) -> c_int {
        if self.0.is_null() {
            // (C dereferences the NULL stream.)
            return errors::assert_arg_failed("stream").raw();
        }
        // SAFETY: as in `write`; `out` is readable and writable (`&mut`).
        unsafe { p::git_odb_stream_finalize_write(out, self.0) }
    }
}

impl Drop for OdbStream {
    fn drop(&mut self) {
        // SAFETY: NULL or the live stream this value owns (the invariant),
        // freed once.
        unsafe { p::git_odb_stream_free(self.0) }
    }
}

/// `git_odb_write`.
pub(crate) fn odb_write(odb: &git_odb, data: &[u8], t: c_int) -> Result<Oid> {
    let mut id = None;
    odb.write(data, t, &mut id)?;
    Ok(id.unwrap_or(Oid::NONE))
}

/// `git_blob_create_from_buffer`.
pub fn create_from_buffer(repo: &git_repository, buffer: &[u8]) -> Result<Oid> {
    let odb = repo.odb()?;
    let mut stream = OdbStream::open(odb, buffer.len() as u64, GIT_OBJECT_BLOB)?;
    let mut id = Oid::NONE;
    let mut error = stream.write(buffer);
    if error == 0 {
        error = stream.finalize(&mut id);
    }
    ErrorCode::check(error)?;
    Ok(id)
}

/// `write_file_stream`: stream the file at `path` into the odb.
fn write_file_stream(odb: &git_odb, path: &CStr, file_size: u64) -> Result<Oid> {
    let mut stream = OdbStream::open(odb, file_size, GIT_OBJECT_BLOB)?;
    let fd = futils::open_ro(path).map_err(|_| ErrorCode::GENERIC)?;
    let mut buffer = match object::FileReadBuffer::new() {
        Ok(b) => b,
        Err(e) => {
            // SAFETY: the descriptor this fn opened above, closed once.
            let _ = unsafe { posix::close(fd) };
            return Err(e);
        }
    };
    let mut error = 0;
    let mut written: u64 = 0;
    let mut read_failed = false;
    while error == 0 {
        match buffer.read(fd) {
            Ok([]) => break,
            Ok(data) => {
                error = stream.write(data);
                written += data.len() as u64;
            }
            Err(posix::Errno(e)) => {
                errors::set_errno(e);
                read_failed = true;
                break;
            }
        }
    }
    // SAFETY: the descriptor this fn opened above, closed once.
    let _ = unsafe { posix::close(fd) };
    if written != file_size || read_failed {
        let _ = git_error_set!(ErrorClass::OS, "failed to read file into stream");
        error = -1;
    }
    let mut id = Oid::NONE;
    if error == 0 {
        error = stream.finalize(&mut id);
    }
    ErrorCode::check(error)?;
    Ok(id)
}

/// `write_symlink`: a blob of the symlink's target.
fn write_symlink(odb: &git_odb, path: &CStr, link_size: usize) -> Result<Oid> {
    let mut link = Vec::new();
    link.try_reserve_exact(link_size)
        .map_err(|_| errors::oom())?;
    link.resize(link_size, 0);
    let ok = matches!(posix::readlink(path, &mut link), Ok(n) if n == link_size);
    if !ok {
        return Err(git_error_set!(
            ErrorClass::OS,
            "failed to create blob: cannot read symlink '{}'",
            Bytes(path.to_bytes())
        ));
    }
    odb_write(odb, &link, GIT_OBJECT_BLOB)
}

/// An odb reference, freed on drop.
///
/// Invariant: NULL, or a live odb of which this value holds one reference.
struct OdbRef(*mut git_odb);

impl OdbRef {
    /// `git_repository_odb`: the repository's odb, with a new reference.
    fn of(repo: Option<&git_repository>) -> Result<OdbRef> {
        let mut odb = OdbRef(ptr::null_mut());
        // SAFETY: `repo` is NULL (an argument error) or a live repository;
        // `odb.0` is writable, and receives the reference `odb` holds.
        ErrorCode::check(unsafe { p::git_repository_odb(&raw mut odb.0, repo) })?;
        Ok(odb)
    }

    /// The odb, for as long as this reference lives.
    fn get(&self) -> Result<&git_odb> {
        // SAFETY: NULL or a live odb kept alive by this value's reference
        // (the invariant) for this borrow; only read through `&` (odb
        // module docs).
        let db = unsafe { self.0.as_ref() };
        db.ok_or_else(|| errors::assert_failed("odb"))
    }
}

impl Drop for OdbRef {
    fn drop(&mut self) {
        // SAFETY: NULL or the reference `git_repository_odb` gave this
        // value (the invariant), given back once.
        unsafe { p::git_odb_free(self.0) }
    }
}

/// `git_blob__create_from_paths`: a blob of the file at `content_path` (or
/// at `hint_path` in the work tree), filtered for `hint_path` when
/// `try_load_filters`; its `stat` into `out_st`. A NULL `repo` fails C's
/// argument checks, in C's order.
pub fn create_from_paths(
    mut out_st: Out<'_, posix::Stat>,
    repo: Option<&git_repository>,
    content_path: Option<&CStr>,
    hint_path: Option<&CStr>,
    hint_mode: libc::mode_t,
    try_load_filters: bool,
) -> Result<Oid> {
    git_assert_arg!(
        hint_path.is_some() || !try_load_filters,
        "hint_path || !try_load_filters"
    );
    let mut path = GitStr::new();
    let content_path: &CStr = match content_path {
        Some(c) => c,
        None => {
            let hp = CStrArg::from(hint_path);
            // SAFETY: `repo` is NULL or a live repository, `path` a valid
            // `git_str` (`&raw mut` of a local), `hp` NULL or a string.
            if unsafe { p::git_repository_workdir_path(&raw mut path, repo, hp) } < 0 {
                return Err(ErrorCode::GENERIC);
            }
            match path.as_c_str() {
                Some(c) => c,
                None => return Err(ErrorCode::GENERIC),
            }
        }
    };
    let st = crate::util::fs_path::lstat(content_path)?;
    let odb = OdbRef::of(repo)?;
    // `git_repository_odb` succeeded: `repo` is not NULL.
    let Some(repo) = repo else {
        return Err(errors::assert_arg_failed("repo"));
    };
    let db = odb.get()?;
    if posix::s_isdir(st.st_mode) {
        let _ = git_error_set!(
            ErrorClass::ODB,
            "cannot create blob from '{}': it is a directory",
            Bytes(content_path.to_bytes())
        );
        return Err(ErrorCode::DIRECTORY);
    }
    out_st.write(st);
    let size = st.st_size as u64;
    let mode = if hint_mode != 0 {
        hint_mode
    } else {
        st.st_mode
    };
    if posix::s_islnk(mode) {
        return write_symlink(db, content_path, size as usize);
    }
    let repo_ptr = ptr::from_ref(repo).cast_mut();
    let fl = if try_load_filters {
        let hp = hint_path.unwrap_or(c"");
        // SAFETY: `repo_ptr` is the live repository `repo`; no blob.
        unsafe {
            FilterList::load(
                repo_ptr,
                ptr::null_mut(),
                hp,
                cfilter::GIT_FILTER_TO_ODB,
                cfilter::GIT_FILTER_DEFAULT,
            )?
        }
    } else {
        None
    };
    match fl {
        // No filters: stream from disk.
        None => write_file_stream(db, content_path, size),
        Some(fl) => {
            // `write_file_filtered`.
            let mut tgt = GitStr::new();
            // SAFETY: `repo_ptr` is the live repository the list was
            // loaded for.
            unsafe { fl.apply_to_file(&mut tgt, repo_ptr, content_path)? };
            odb_write(db, tgt.as_bytes(), GIT_OBJECT_BLOB)
        }
    }
}

/// `git_blob_create_from_disk`: a blob of the file at `path`, filtered as
/// its work-tree path when it is in the work tree (a NULL `repo` fails
/// C's argument checks).
pub fn create_from_disk(repo: Option<&git_repository>, path: &CStr) -> Result<Oid> {
    let mut full_path = GitStr::new();
    crate::util::fs_path::prettify(&mut full_path, path, None)?;
    // `git_repository_workdir(repo)`.
    let workdir = match repo {
        Some(r) => r.workdir(),
        None => {
            let _ = errors::assert_arg_failed("repo");
            None
        }
    };
    let full = full_path.as_bytes();
    let mut hint: Option<&CStr> = None;
    if let Some(w) = workdir {
        let w = w.to_bytes();
        if crate::util::util::prefixcmp(full, w) == 0 {
            hint = full_path.as_c_str().map(|c| {
                // A suffix of a NUL-terminated string is one.
                CStr::from_bytes_with_nul(&c.to_bytes_with_nul()[w.len()..]).unwrap_or(c"")
            });
        }
    }
    let content = full_path.as_c_str();
    create_from_paths(Out::null(), repo, content, hint, 0, hint.is_some())
}

// ---------------------------------------------------------------------------
// Streams
// ---------------------------------------------------------------------------

/// `git_writestream`, mirrored (public).
#[repr(C)]
#[derive(Debug, Default)]
pub struct GitWriteStream {
    /// `write`.
    pub write: Option<unsafe extern "C" fn(*mut GitWriteStream, *const c_char, usize) -> c_int>,
    /// `close`.
    pub close: Option<unsafe extern "C" fn(*mut GitWriteStream) -> c_int>,
    /// `free`.
    pub free: Option<unsafe extern "C" fn(*mut GitWriteStream)>,
}

define_ctype!(WriteStream, WriteStreamRef, WriteStreamMut, GitWriteStream);

/// The state of `blob_writestream`: a temporary file under `objects/`, and
/// where it goes on commit.
///
/// Invariant (made only by [`create_from_stream`], whose caller promises
/// it): `repo` is a live repository that outlives the state.
pub struct BlobWriteStream {
    fbuf: Box<GitFilebuf>,
    repo: *mut git_repository,
    hintpath: Option<GitCStr>,
}

// SAFETY: the state owns its filebuf and path, which move between threads
// freely; `repo` is a pointer only passed to C (`git_repository` is usable
// from any thread). Sync: no method or slot takes `&BlobWriteStream`
// (every slot is exclusive and the owner only uses `state_mut` /
// `into_inner`), so a shared reference is never used to reach the state;
// it is asserted only because `CTable::into_c` asks for it.
unsafe impl Send for BlobWriteStream {}
// SAFETY: as above.
unsafe impl Sync for BlobWriteStream {}

impl Drop for BlobWriteStream {
    fn drop(&mut self) {
        // `blob_writestream_free`.
        self.fbuf.cleanup();
    }
}

/// A stream slot refused because another one runs on the stream (re-entry
/// or a concurrent call): -1 with `GIT_ERROR_INTERNAL`.
fn busy() -> c_int {
    git_error_set!(ErrorClass::INTERNAL, "stream in use").raw()
}

impl_cvtable! {
    // SAFETY: libgit2's contract for a `git_writestream`: its slots are
    // called with the stream itself, first, and `write`, `close` and `free`
    // are all set.
    unsafe impl CVtable<BlobWriteStream> for WriteStream {}
    base = GitWriteStream::default();
    /// `blob_writestream_write`.
    fn write(@stream, buffer: *const c_char, len: usize) -> c_int {
        on panic: { errors::set_panic_error(); -1 },
        on busy: busy(),
        |s: &mut BlobWriteStream| {
            if buffer.is_null() && len > 0 {
                // (C dereferences it: rust/DEFECTS.md.)
                return errors::assert_arg_failed("buffer").raw();
            }
            // SAFETY: C passes `len` readable bytes at `buffer`.
            let data = unsafe { crate::util::export::bytes(buffer, len) };
            crate::util::export::status(s.fbuf.write(data))
        },
    }
    /// `blob_writestream_close`.
    fn close(@stream) -> c_int {
        on panic: { errors::set_panic_error(); -1 },
        on busy: busy(),
        |s: &mut BlobWriteStream| {
            s.fbuf.cleanup();
            0
        },
    }
    release free;
}

/// `git_blob_create_from_stream`: a stream that writes a blob, which
/// [`stream_commit`] creates.
///
/// # Safety
///
/// `repo` outlives the stream (the repository is kept as a pointer, and
/// used again by [`stream_commit`]).
pub unsafe fn create_from_stream(
    repo: &git_repository,
    hintpath: Option<&CStr>,
) -> Result<CTable<WriteStream, BlobWriteStream>> {
    let hintpath = match hintpath {
        Some(h) => {
            // SAFETY: `strdup`'s result: NULL, or a fresh NUL-terminated
            // `git__allocator` copy that the `GitCStr` owns.
            Some(unsafe { GitCStr::from_raw(alloc::strdup(h)) }.ok_or(ErrorCode::GENERIC)?)
        }
        None => None,
    };
    let mut path = GitStr::new();
    // `GIT_REPOSITORY_ITEM_OBJECTS`.
    // SAFETY: a live repository, a valid `git_str` (a local).
    ErrorCode::check(unsafe { p::git_repository__item_path(Some(&mut path), Some(repo), 4) })?;
    path.joinpath_self(b"streamed")?;
    let mut fbuf = Box::new(GitFilebuf::new());
    // SAFETY: the filebuf is boxed: it does not move until the stream's
    // drop cleans it up.
    unsafe {
        fbuf.open_withsize(
            path.as_bytes(),
            GIT_FILEBUF_TEMPORARY,
            0o666,
            2 * 1024 * 1024,
        )
    }?;
    Ok(CTable::new(BlobWriteStream {
        fbuf,
        repo: ptr::from_ref(repo).cast_mut(),
        hintpath,
    }))
}

/// `git_blob_create_from_stream_commit`: flush the stream and create the
/// blob (the stream is freed).
pub fn stream_commit(stream: CTable<WriteStream, BlobWriteStream>) -> Result<Oid> {
    // The state is taken out of the table first: filter loading and the
    // filters (user code) run below, and no slot may reach the state then.
    let mut s = stream.into_inner();
    ErrorCode::check(s.fbuf.flush())?;
    let lock = s.fbuf.lock_path();
    let hint = s.hintpath.as_ref().map(|h| h.as_c_str());
    // SAFETY: the state's repository, live as long as the state (its
    // invariant), only read through `&` (its state is interior-mutable).
    let repo = unsafe { s.repo.as_ref() };
    let r = create_from_paths(Out::null(), repo, lock, hint, 0, hint.is_some());
    drop(s);
    r
}

// ---------------------------------------------------------------------------
// Filtering
// ---------------------------------------------------------------------------

/// `git_blob_filter_options`, mirrored (public; with `GIT_DEPRECATE_HARD`,
/// `commit_id` is `void *reserved`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GitBlobFilterOptions {
    /// `GIT_BLOB_FILTER_OPTIONS_VERSION` (an `int`).
    pub version: c_int,
    /// `git_blob_filter_flag_t`.
    pub flags: u32,
    /// Reserved.
    pub reserved: *mut c_void,
    /// The commit to load attributes from.
    pub attr_commit_id: Oid,
}

/// `GIT_BLOB_FILTER_OPTIONS_VERSION`.
pub const GIT_BLOB_FILTER_OPTIONS_VERSION: c_int = 1;
/// `GIT_BLOB_FILTER_CHECK_FOR_BINARY`.
pub const GIT_BLOB_FILTER_CHECK_FOR_BINARY: u32 = 1 << 0;
/// `GIT_BLOB_FILTER_NO_SYSTEM_ATTRIBUTES`.
pub const GIT_BLOB_FILTER_NO_SYSTEM_ATTRIBUTES: u32 = 1 << 1;
/// `GIT_BLOB_FILTER_ATTRIBUTES_FROM_HEAD`.
pub const GIT_BLOB_FILTER_ATTRIBUTES_FROM_HEAD: u32 = 1 << 2;
/// `GIT_BLOB_FILTER_ATTRIBUTES_FROM_COMMIT`.
pub const GIT_BLOB_FILTER_ATTRIBUTES_FROM_COMMIT: u32 = 1 << 3;

impl GitBlobFilterOptions {
    /// `GIT_BLOB_FILTER_OPTIONS_INIT`.
    pub const INIT: GitBlobFilterOptions = GitBlobFilterOptions {
        version: GIT_BLOB_FILTER_OPTIONS_VERSION,
        flags: GIT_BLOB_FILTER_CHECK_FOR_BINARY,
        reserved: ptr::null_mut(),
        attr_commit_id: Oid::NONE,
    };
}

/// `git_blob_filter`: the blob's contents through the filters for `path`
/// (to the work tree), into `out` (left alone when the binary check skips
/// it).
///
/// # Safety
///
/// `blob` is a live blob of a repository; `out` a valid `git_buf`.
pub unsafe fn filter(
    out: *mut GitBuf,
    blob: NonNull<GitBlob>,
    path: &CStr,
    opts: &GitBlobFilterOptions,
) -> Result<()> {
    // SAFETY: a live blob.
    let b = unsafe { blob.as_ref() };
    if opts.flags & GIT_BLOB_FILTER_CHECK_FOR_BINARY != 0 && content_is_binary(b.content()) {
        return Ok(());
    }
    let mut fo = cfilter::git_filter_options::INIT;
    if opts.flags & GIT_BLOB_FILTER_NO_SYSTEM_ATTRIBUTES != 0 {
        fo.flags |= cfilter::GIT_FILTER_NO_SYSTEM_ATTRIBUTES;
    }
    if opts.flags & GIT_BLOB_FILTER_ATTRIBUTES_FROM_HEAD != 0 {
        fo.flags |= cfilter::GIT_FILTER_ATTRIBUTES_FROM_HEAD;
    }
    if opts.flags & GIT_BLOB_FILTER_ATTRIBUTES_FROM_COMMIT != 0 {
        fo.flags |= cfilter::GIT_FILTER_ATTRIBUTES_FROM_COMMIT;
        fo.attr_commit_id = opts.attr_commit_id;
    }
    let o = blob.as_ptr().cast::<c_void>();
    // SAFETY: the blob's live repository and the blob.
    let fl = unsafe {
        FilterList::load_ext(
            b.object.repo,
            o,
            path,
            cfilter::GIT_FILTER_TO_WORKTREE,
            &mut fo,
        )?
    };
    // SAFETY: a valid `git_buf`, a live blob; a NULL list copies the
    // contents.
    unsafe {
        cfilter::apply_to_blob(
            out,
            fl.as_ref().map_or(ptr::null_mut(), FilterList::as_ptr),
            o,
        )
    }
}

// ---------------------------------------------------------------------------
// The safe API
// ---------------------------------------------------------------------------

/// A blob of a repository: an [`Object`] of type blob.
#[derive(Clone, Debug)]
#[repr(transparent)]
pub struct Blob<'repo>(Object<'repo>);

impl<'repo> TryFrom<Object<'repo>> for Blob<'repo> {
    type Error = Object<'repo>;

    /// The object as a blob, or the object back if it is not one.
    fn try_from(o: Object<'repo>) -> core::result::Result<Blob<'repo>, Object<'repo>> {
        if o.kind() == ObjectType::Blob {
            Ok(Blob(o))
        } else {
            Err(o)
        }
    }
}

impl<'repo> From<Blob<'repo>> for Object<'repo> {
    fn from(b: Blob<'repo>) -> Object<'repo> {
        b.0
    }
}

impl<'repo> Blob<'repo> {
    /// An object of type blob, borrowed as a blob.
    pub(crate) fn from_object_ref<'s>(obj: &'s Object<'repo>) -> &'s Blob<'repo> {
        debug_assert_eq!(obj.kind(), ObjectType::Blob);
        // SAFETY: `Blob` is a `repr(transparent)` wrapper of `Object`, whose
        // only invariant (the object is a blob) the caller checked.
        unsafe { &*ptr::from_ref(obj).cast::<Blob<'repo>>() }
    }

    /// `git_blob_lookup`.
    pub fn lookup(repo: &'repo Repository, id: &Oid) -> core::result::Result<Blob<'repo>, Error> {
        Ok(Blob(Object::lookup(repo, id, Some(ObjectType::Blob))?))
    }

    /// `git_blob_lookup_prefix`.
    pub fn lookup_prefix(
        repo: &'repo Repository,
        id: &Oid,
        len: usize,
    ) -> core::result::Result<Blob<'repo>, Error> {
        Ok(Blob(Object::lookup_prefix(
            repo,
            id,
            len,
            Some(ObjectType::Blob),
        )?))
    }

    fn raw(&self) -> &GitBlob {
        // SAFETY: a live blob (checked type), read only after it was
        // published.
        unsafe { self.0.as_ptr().cast::<GitBlob>().as_ref() }
    }

    /// `git_blob_id`.
    pub fn id(&self) -> &Oid {
        self.0.id()
    }

    /// The blob as an object.
    pub fn as_object(&self) -> &Object<'repo> {
        &self.0
    }

    /// The blob as an object.
    pub fn into_object(self) -> Object<'repo> {
        self.0
    }

    /// `git_blob_rawcontent`.
    pub fn content(&self) -> &[u8] {
        self.raw().content()
    }

    /// `git_blob_rawsize`.
    pub fn size(&self) -> u64 {
        self.raw().raw_parts().1 as u64
    }

    /// `git_blob_is_binary`.
    pub fn is_binary(&self) -> bool {
        content_is_binary(self.content())
    }

    /// `git_blob_filter`: the contents as they would be checked out at
    /// `path` (`None` when `opts` asks for the binary check and the blob
    /// is binary).
    pub fn filter(
        &self,
        path: impl AsRef<[u8]>,
        opts: BlobFilterOptions,
    ) -> core::result::Result<Option<Vec<u8>>, Error> {
        let path = std::ffi::CString::new(path.as_ref()).map_err(|_| {
            Error::from(git_error_set!(
                ErrorClass::INVALID,
                "path contains a NUL byte"
            ))
        })?;
        let o = opts.raw();
        if o.flags & GIT_BLOB_FILTER_CHECK_FOR_BINARY != 0 && self.is_binary() {
            return Ok(None);
        }
        let mut out = GitBuf::new();
        // SAFETY: a live blob of a repository and a valid `git_buf`.
        let mut run = || unsafe { filter(&raw mut out, self.0.as_ptr().cast(), &path, &o) };
        match self.0.owner() {
            // The filters read attributes, which C may read from the index.
            Some(repo) => crate::index::api::with_repository_index_locked(repo, &mut run)?,
            None => run()?,
        }
        Ok(Some(out.into_vec()))
    }

    /// `git_blob_create_from_buffer`.
    pub fn create_from_buffer(
        repo: &Repository,
        data: impl AsRef<[u8]>,
    ) -> core::result::Result<Oid, Error> {
        Ok(create_from_buffer(repo.raw(), data.as_ref())?)
    }

    /// `git_blob_create_from_workdir`: a blob of the work-tree file at
    /// `path` (relative to the work tree, `/`-separated), filtered.
    pub fn create_from_workdir(
        repo: &Repository,
        path: impl AsRef<[u8]>,
    ) -> core::result::Result<Oid, Error> {
        let path = std::ffi::CString::new(path.as_ref()).map_err(|_| {
            Error::from(git_error_set!(
                ErrorClass::INVALID,
                "path contains a NUL byte"
            ))
        })?;
        // The filters read attributes, which C may read from the index.
        Ok(crate::index::api::with_repository_index_locked(
            repo.raw(),
            || create_from_paths(Out::null(), Some(repo.raw()), None, Some(&path), 0, true),
        )?)
    }

    /// `git_blob_create_from_disk`: a blob of the file at `path` (anywhere),
    /// filtered as the work-tree file of that path would be if it is in the
    /// work tree.
    pub fn create_from_disk(
        repo: &Repository,
        path: impl AsRef<std::path::Path>,
    ) -> core::result::Result<Oid, Error> {
        use std::os::unix::ffi::OsStrExt as _;
        let path = std::ffi::CString::new(path.as_ref().as_os_str().as_bytes()).map_err(|_| {
            Error::from(git_error_set!(
                ErrorClass::INVALID,
                "path contains a NUL byte"
            ))
        })?;
        // The filters read attributes, which C may read from the index.
        Ok(crate::index::api::with_repository_index_locked(
            repo.raw(),
            || create_from_disk(Some(repo.raw()), &path),
        )?)
    }

    /// `git_blob_create_from_stream`: a writer whose
    /// [`commit`](BlobWriter::commit) creates the blob.
    pub fn writer<'r>(
        repo: &'r Repository,
        hintpath: Option<&[u8]>,
    ) -> core::result::Result<BlobWriter<'r>, Error> {
        let hint = match hintpath {
            Some(h) => Some(std::ffi::CString::new(h).map_err(|_| {
                Error::from(git_error_set!(
                    ErrorClass::INVALID,
                    "path contains a NUL byte"
                ))
            })?),
            None => None,
        };
        // SAFETY: the writer borrows `repo`, so it outlives the stream.
        let t = unsafe { create_from_stream(repo.raw(), hint.as_deref())? };
        Ok(BlobWriter { table: t, repo })
    }
}

/// `git_blob_data_is_binary`.
pub fn data_is_binary(data: &[u8]) -> bool {
    crate::util::str::is_binary(data)
}

impl Blob<'_> {
    /// `git_blob_data_is_binary`: whether `data` looks binary (as
    /// [`is_binary`](Blob::is_binary) judges a blob's content).
    pub fn data_is_binary(data: impl AsRef<[u8]>) -> bool {
        data_is_binary(data.as_ref())
    }
}

/// A blob being written (`git_blob_create_from_stream`).
pub struct BlobWriter<'repo> {
    table: CTable<WriteStream, BlobWriteStream>,
    repo: &'repo Repository,
}

// SAFETY: the writer owns its table alone: the stream's C handle is never
// handed to C by the safe API (writes go to the state through
// `state_mut`, the commit takes the state out), so nothing else reaches
// the table or the state, which is `Send` (a filebuf and a path); the
// repository is `Sync`.
unsafe impl Send for BlobWriter<'_> {}
// SAFETY: no method takes `&BlobWriter` (writes and the commit need
// `&mut self` / `self`), so a shared reference reaches nothing.
unsafe impl Sync for BlobWriter<'_> {}

impl BlobWriter<'_> {
    /// Append `data`.
    pub fn write(&mut self, data: &[u8]) -> core::result::Result<(), Error> {
        Ok(self.table.state_mut().fbuf.write(data)?)
    }

    /// `git_blob_create_from_stream_commit`.
    pub fn commit(self) -> core::result::Result<Oid, Error> {
        let repo = self.repo;
        // The filters read attributes, which C may read from the index.
        Ok(crate::index::api::with_repository_index_locked(
            repo.raw(),
            || stream_commit(self.table),
        )?)
    }
}

impl std::io::Write for BlobWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        BlobWriter::write(self, buf).map_err(std::io::Error::other)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Options of [`Blob::filter`] (`git_blob_filter_options`): a by-value
/// builder (`BlobFilterOptions::new().check_for_binary(false)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlobFilterOptions {
    flags: u32,
    commit: Oid,
}

impl Default for BlobFilterOptions {
    fn default() -> Self {
        BlobFilterOptions {
            flags: GIT_BLOB_FILTER_CHECK_FOR_BINARY,
            commit: Oid::NONE,
        }
    }
}

impl BlobFilterOptions {
    /// The defaults: skip binary blobs, system attributes on, attributes
    /// from the work tree and index.
    pub fn new() -> BlobFilterOptions {
        BlobFilterOptions::default()
    }

    /// Skip binary blobs (on by default).
    pub fn check_for_binary(mut self, on: bool) -> Self {
        self.set(GIT_BLOB_FILTER_CHECK_FOR_BINARY, on);
        self
    }
    /// Ignore the system attributes file.
    pub fn no_system_attributes(mut self, on: bool) -> Self {
        self.set(GIT_BLOB_FILTER_NO_SYSTEM_ATTRIBUTES, on);
        self
    }
    /// Load attributes from `HEAD`.
    pub fn attributes_from_head(mut self, on: bool) -> Self {
        self.set(GIT_BLOB_FILTER_ATTRIBUTES_FROM_HEAD, on);
        self
    }
    /// Load attributes from `commit`.
    pub fn attributes_from_commit(mut self, commit: Oid) -> Self {
        self.set(GIT_BLOB_FILTER_ATTRIBUTES_FROM_COMMIT, true);
        self.commit = commit;
        self
    }
    fn set(&mut self, f: u32, on: bool) {
        if on {
            self.flags |= f;
        } else {
            self.flags &= !f;
        }
    }
    fn raw(&self) -> GitBlobFilterOptions {
        GitBlobFilterOptions {
            version: GIT_BLOB_FILTER_OPTIONS_VERSION,
            flags: self.flags,
            reserved: ptr::null_mut(),
            attr_commit_id: self.commit,
        }
    }
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

pub mod ffi {
    //! The exports of `blob.c`.
    //!
    //! A live repository is taken as `Option<&GitRepository>` (only read
    //! through `&`), and a blob that is only read as `Option<&GitBlob>` (a
    //! published blob is not written but for its atomic count).
    use super::*;
    use crate::git_arg;
    use crate::object::ffi::check_version;
    use crate::util::cabi::{CStrArg, Out};
    use crate::util::export::{barrier, bytes, status};

    /// `git_blob_rawcontent`.
    ///
    /// # Safety
    ///
    /// `blob` is NULL or a live blob.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_rawcontent(blob: Option<&GitBlob>) -> *const c_void {
        git_arg!(blob, "blob", ptr::null()).raw_parts().0.cast()
    }

    /// `git_blob_rawsize` (`(git_object_size_t)-1` for NULL, as C's
    /// `GIT_ASSERT_ARG`).
    ///
    /// # Safety
    ///
    /// `blob` is NULL or a live blob.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_rawsize(blob: Option<&GitBlob>) -> u64 {
        git_arg!(blob, "blob", u64::MAX).raw_parts().1 as u64
    }

    /// `git_blob__getbuf`.
    ///
    /// # Safety
    ///
    /// `buffer` is NULL or a valid `git_str` that is not the blob's data;
    /// `blob` NULL or a live blob.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob__getbuf(
        buffer: Option<&mut GitStr>,
        blob: Option<&GitBlob>,
    ) -> c_int {
        barrier(|| {
            let blob = git_arg!(blob, "blob", -1);
            let buffer = git_arg!(buffer, "buffer", -1);
            status(getbuf(buffer, blob))
        })
    }

    /// `git_blob__create_from_paths`.
    ///
    /// # Safety
    ///
    /// `id` and `out_st` are NULL or writable; `repo` NULL or a live
    /// repository; the paths NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob__create_from_paths(
        id: Out<'_, Oid>,
        out_st: Out<'_, posix::Stat>,
        repo: Option<&git_repository>,
        content_path: CStrArg<'_>,
        hint_path: CStrArg<'_>,
        hint_mode: libc::mode_t,
        try_load_filters: bool,
    ) -> c_int {
        barrier(|| {
            let c = content_path.get();
            let h = hint_path.get();
            write_oid(
                id,
                create_from_paths(out_st, repo, c, h, hint_mode, try_load_filters),
            )
        })
    }

    /// Write an id result to C's out-parameter.
    fn write_oid(mut id: Out<'_, Oid>, r: Result<Oid>) -> c_int {
        match r {
            Ok(oid) => {
                id.write(oid);
                0
            }
            Err(e) => e.raw(),
        }
    }

    /// `git_blob_create_from_buffer`.
    ///
    /// # Safety
    ///
    /// `id` is NULL or writable; `repo` NULL or live; `buffer` NULL or
    /// readable for `len` bytes, which nothing writes during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_create_from_buffer(
        id: Out<'_, Oid>,
        repo: Option<&git_repository>,
        buffer: *const c_void,
        len: usize,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!id.is_null(), "id", -1);
            let repo = git_arg!(repo, "repo", -1);
            // (C dereferences a NULL buffer: rust/DEFECTS.md.)
            git_assert_arg!(!buffer.is_null() || len == 0, "buffer", -1);
            // SAFETY: `buffer` is NULL only with `len == 0` (checked above);
            // otherwise readable for `len` unwritten bytes (this export's
            // contract).
            let data = unsafe { bytes(buffer.cast(), len) };
            write_oid(id, create_from_buffer(repo, data))
        })
    }

    /// `git_blob_create_from_workdir`.
    ///
    /// # Safety
    ///
    /// `id` is NULL or writable; `repo` NULL or live; `path` NULL or
    /// NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_create_from_workdir(
        id: Out<'_, Oid>,
        repo: Option<&git_repository>,
        path: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let h = path.get();
            write_oid(id, create_from_paths(Out::null(), repo, None, h, 0, true))
        })
    }

    /// `git_blob_create_from_disk`.
    ///
    /// # Safety
    ///
    /// As [`git_blob_create_from_workdir`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_create_from_disk(
        id: Out<'_, Oid>,
        repo: Option<&git_repository>,
        path: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            let path = git_arg!(path.get(), "path", -1);
            write_oid(id, create_from_disk(repo, path))
        })
    }

    /// `git_blob_create_from_stream`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `repo` NULL or a live repository that
    /// outlives the stream; `hintpath` NULL or NUL-terminated. The stream is
    /// used as a `git_writestream` (its slots called with itself) and freed
    /// once, by its `free` or by [`git_blob_create_from_stream_commit`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_create_from_stream(
        mut out: Out<'_, *mut GitWriteStream>,
        repo: Option<&git_repository>,
        hintpath: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null(), "out", -1);
            let repo = git_arg!(repo, "repo", -1);
            let h = hintpath.get();
            // SAFETY: `repo` outlives the stream (this export's contract).
            match unsafe { create_from_stream(repo, h) } {
                Ok(t) => {
                    // The table goes to C, which frees it once.
                    out.write(t.into_c().as_ptr());
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_blob_create_from_stream_commit`: create the blob and free the
    /// stream.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `stream` NULL or one that came from
    /// [`git_blob_create_from_stream`], has not been freed, and is not used
    /// again (no slot of it runs now).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_create_from_stream_commit(
        out: Out<'_, Oid>,
        stream: *mut GitWriteStream,
    ) -> c_int {
        barrier(|| {
            let Some(s) = NonNull::new(stream) else {
                return errors::assert_arg_failed("stream").raw();
            };
            // SAFETY: a stream of `git_blob_create_from_stream`, surrendered
            // with `into_c`, which C hands back for good with no slot
            // running (this export's contract).
            let table = unsafe { CTable::<WriteStream, BlobWriteStream>::from_c(s) };
            write_oid(out, stream_commit(table))
        })
    }

    /// `git_blob_is_binary`.
    ///
    /// # Safety
    ///
    /// `blob` is NULL or a live blob.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_is_binary(blob: Option<&GitBlob>) -> c_int {
        barrier(|| {
            let blob = git_arg!(blob, "blob", -1);
            c_int::from(content_is_binary(blob.content()))
        })
    }

    /// `git_blob_data_is_binary`.
    ///
    /// # Safety
    ///
    /// `str` is NULL or readable for `len` bytes, which nothing writes
    /// during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_data_is_binary(str: *const c_char, len: usize) -> c_int {
        barrier(|| {
            // (C dereferences a NULL buffer: rust/DEFECTS.md.)
            git_assert_arg!(!str.is_null() || len == 0, "str", -1);
            // SAFETY: `str` is NULL only with `len == 0` (checked above),
            // otherwise readable for `len` unwritten bytes (this export's
            // contract).
            c_int::from(data_is_binary(unsafe { bytes(str, len) }))
        })
    }

    /// `git_blob_filter_options_init`.
    ///
    /// # Safety
    ///
    /// `opts` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_filter_options_init(
        mut opts: Out<'_, GitBlobFilterOptions>,
        version: c_uint,
    ) -> c_int {
        barrier(|| {
            if let Err(e) = check_version(
                Some(&version),
                GIT_BLOB_FILTER_OPTIONS_VERSION as c_uint,
                "git_blob_filter_options",
            ) {
                return e.raw();
            }
            opts.write(GitBlobFilterOptions::INIT);
            0
        })
    }

    /// `git_blob_filter`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or a valid `git_buf` that nothing else uses during the
    /// call; `blob` NULL or a live blob of a live repository; `path` NULL or
    /// NUL-terminated; `given_opts` NULL or readable, unwritten during the
    /// call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_filter(
        out: *mut GitBuf,
        blob: *mut GitBlob,
        path: CStrArg<'_>,
        given_opts: Option<&GitBlobFilterOptions>,
    ) -> c_int {
        barrier(|| {
            let blob = git_arg!(NonNull::new(blob), "blob", -1);
            let path = git_arg!(path.get(), "path", -1);
            git_assert_arg!(!out.is_null(), "out", -1);
            // (`version` is the options' first member, an `int` C checks
            // as `unsigned int`.)
            let version = given_opts.map(|o| o.version as c_uint);
            if let Err(e) = check_version(
                version.as_ref(),
                GIT_BLOB_FILTER_OPTIONS_VERSION as c_uint,
                "git_blob_filter_options",
            ) {
                return e.raw();
            }
            let opts = given_opts.copied().unwrap_or(GitBlobFilterOptions::INIT);
            // SAFETY: a live blob of a live repository, a valid `git_buf`
            // that nothing else uses (this export's contract).
            status(unsafe { filter(out, blob, path, &opts) })
        })
    }
}

crate::c_layout_test! {
    headers: ["blob.h", "git2/blob.h", "git2/sys/filter.h"],
    types: [
        GitWriteStream => "git_writestream" { write, close, free },
        GitBlobFilterOptions => "git_blob_filter_options" { version, flags, reserved, attr_commit_id },
    ],
    asserts: [
        "GIT_BLOB_FILTER_CHECK_FOR_BINARY == 1 && GIT_BLOB_FILTER_ATTRIBUTES_FROM_COMMIT == 8",
        "GIT_REPOSITORY_ITEM_OBJECTS == 4",
    ],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_detection() {
        assert!(!content_is_binary(b"hello\nworld\n"));
        assert!(content_is_binary(b"a\0b"));
        let mut big = vec![b'a'; 9000];
        big[8500] = 0;
        // Only the first 8000 bytes count.
        assert!(!content_is_binary(&big));
        assert!(data_is_binary(&big));
    }

    #[test]
    fn raw_blob_contents() {
        let _g = crate::util::test_support::init();
        let data = b"some content";
        let b = alloc().unwrap();
        // SAFETY: a fresh blob; `data` outlives it.
        unsafe { parse_raw(b, data.as_ptr(), data.len()) };
        // SAFETY: a live blob.
        assert_eq!(unsafe { b.as_ref() }.content(), data);
        let mut s = GitStr::new();
        // SAFETY: a live blob.
        getbuf(&mut s, unsafe { b.as_ref() }).unwrap();
        assert_eq!(s.as_bytes(), data);
        // SAFETY: the blob made above, freed once.
        unsafe { free(b) };
    }
}
