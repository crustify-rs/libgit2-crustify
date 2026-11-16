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

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::{self, NonNull};

use ffibox::vtable::CTable;
use ffibox::{define_ctype, impl_cvtable};

use crate::buf::GitBuf;
use crate::object::{
    self, GIT_OBJECT_BLOB, GitCachedObj, GitObject, Object, ObjectType, Repository,
};
use crate::oid::Oid;
use crate::sys::filter::{self as cfilter, FilterList};
use crate::sys::pending::{self as p, git_odb, git_odb_object, git_odb_stream, git_repository};
use crate::util::alloc::{self, GitCStr};
use crate::util::errors::{self, Bytes, Error, ErrorClass, ErrorCode, Result};
use crate::util::filebuf::{GIT_FILEBUF_TEMPORARY, GitFilebuf};
use crate::util::futils::{self, GIT_BUFSIZE_FILEIO};
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

/// Allocate an empty blob (`git__calloc(1, sizeof(git_blob))`).
pub(crate) fn alloc() -> Result<NonNull<GitBlob>> {
    let p = NonNull::new(alloc::malloc(size_of::<GitBlob>()).cast::<GitBlob>()).ok_or(ErrorCode::GENERIC)?;
    // SAFETY: a fresh `git__allocator` block of a `GitBlob`'s size,
    // 8-aligned (as `git__allocator`'s memory is, checked by the layout
    // test's alignment).
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
    let c = odb_obj.cast::<GitCachedObj>();
    // SAFETY: a live odb object starts with its `git_cached_obj`
    // (PENDING(l-odb)); `git_cached_obj_incref`; the size is read only.
    let size = unsafe {
        (*c).refcount.inc();
        (*c).size
    };
    // SAFETY: a live odb object; its buffer lives as long as it does.
    let data = unsafe { p::git_odb_object_data(odb_obj) }.cast::<u8>();
    // SAFETY: a fresh blob, not shared yet.
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
struct OdbStream(*mut git_odb_stream);

impl OdbStream {
    /// `git_odb_open_wstream`.
    ///
    /// # Safety
    ///
    /// `odb` is a live odb.
    unsafe fn open(odb: *mut git_odb, size: u64, t: c_int) -> Result<OdbStream> {
        let mut s = ptr::null_mut();
        // SAFETY: a live odb, a writable out.
        ErrorCode::check(unsafe { p::git_odb_open_wstream(&mut s, odb, size, t) })?;
        Ok(OdbStream(s))
    }

    /// `git_odb_stream_write`: its status.
    fn write(&mut self, data: &[u8]) -> c_int {
        // SAFETY: a live stream; `data` readable.
        unsafe { p::git_odb_stream_write(self.0, data.as_ptr().cast(), data.len()) }
    }

    /// `git_odb_stream_finalize_write`.
    fn finalize(&mut self, out: &mut Oid) -> c_int {
        // SAFETY: a live stream, a writable id.
        unsafe { p::git_odb_stream_finalize_write(out, self.0) }
    }
}

impl Drop for OdbStream {
    fn drop(&mut self) {
        // SAFETY: the stream this value holds, freed once.
        unsafe { p::git_odb_stream_free(self.0) }
    }
}

/// `git_odb_write`.
///
/// # Safety
///
/// `odb` is a live odb.
pub(crate) unsafe fn odb_write(odb: *mut git_odb, data: &[u8], t: c_int) -> Result<Oid> {
    let mut id = Oid::NONE;
    // SAFETY: a live odb, readable data, a writable id.
    let r = unsafe { p::git_odb_write(&mut id, odb, data.as_ptr().cast(), data.len(), t) };
    ErrorCode::check(r)?;
    Ok(id)
}

/// `git_blob_create_from_buffer`.
///
/// # Safety
///
/// `repo` is a live repository.
pub unsafe fn create_from_buffer(repo: *mut git_repository, buffer: &[u8]) -> Result<Oid> {
    // SAFETY: a live repository.
    let odb = unsafe { object::repo_odb_weak(repo)? };
    // SAFETY: its live odb.
    let mut stream = unsafe { OdbStream::open(odb, buffer.len() as u64, GIT_OBJECT_BLOB)? };
    let mut id = Oid::NONE;
    let mut error = stream.write(buffer);
    if error == 0 {
        error = stream.finalize(&mut id);
    }
    ErrorCode::check(error)?;
    Ok(id)
}

/// `write_file_stream`: stream the file at `path` into the odb.
///
/// # Safety
///
/// `odb` is a live odb.
unsafe fn write_file_stream(odb: *mut git_odb, path: &CStr, file_size: u64) -> Result<Oid> {
    // SAFETY: a live odb.
    let mut stream = unsafe { OdbStream::open(odb, file_size, GIT_OBJECT_BLOB)? };
    let fd = futils::open_ro(path).map_err(|_| ErrorCode::GENERIC)?;
    let mut buffer = vec![0u8; GIT_BUFSIZE_FILEIO];
    let mut error = 0;
    let mut written: u64 = 0;
    let mut read_failed = false;
    while error == 0 {
        match posix::read(fd, &mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                error = stream.write(&buffer[..n]);
                written += n as u64;
            }
            Err(posix::Errno(e)) => {
                errors::set_errno(e);
                read_failed = true;
                break;
            }
        }
    }
    let _ = posix::close(fd);
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
///
/// # Safety
///
/// `odb` is a live odb.
unsafe fn write_symlink(odb: *mut git_odb, path: &CStr, link_size: usize) -> Result<Oid> {
    let mut link = Vec::new();
    link.try_reserve_exact(link_size).map_err(|_| errors::oom())?;
    link.resize(link_size, 0);
    let ok = matches!(posix::readlink(path, &mut link), Ok(n) if n == link_size);
    if !ok {
        return Err(git_error_set!(
            ErrorClass::OS,
            "failed to create blob: cannot read symlink '{}'",
            Bytes(path.to_bytes())
        ));
    }
    // SAFETY: a live odb.
    unsafe { odb_write(odb, &link, GIT_OBJECT_BLOB) }
}

/// An odb reference, freed on drop.
struct OdbRef(*mut git_odb);

impl Drop for OdbRef {
    fn drop(&mut self) {
        // SAFETY: the reference `git_repository_odb` gave (or NULL).
        unsafe { p::git_odb_free(self.0) }
    }
}

/// `git_blob__create_from_paths`: a blob of the file at `content_path` (or
/// at `hint_path` in the work tree), filtered for `hint_path` when
/// `try_load_filters`; its `stat` into `out_st`.
///
/// # Safety
///
/// `repo` is a live repository.
pub unsafe fn create_from_paths(
    out_st: Option<&mut posix::Stat>,
    repo: *mut git_repository,
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
            let hp = hint_path.map_or(ptr::null(), CStr::as_ptr);
            // SAFETY: a live repository, a valid `git_str`, NULL or a
            // NUL-terminated path.
            if unsafe { p::git_repository_workdir_path(&mut path, repo, hp) } < 0 {
                return Err(ErrorCode::GENERIC);
            }
            match path.as_c_str() {
                Some(c) => c,
                None => return Err(ErrorCode::GENERIC),
            }
        }
    };
    let st = crate::util::fs_path::lstat(content_path)?;
    let mut odb = OdbRef(ptr::null_mut());
    // SAFETY: a live repository; the reference is held by `odb`.
    ErrorCode::check(unsafe { p::git_repository_odb(&mut odb.0, repo) })?;
    if posix::s_isdir(st.st_mode) {
        let _ = git_error_set!(
            ErrorClass::ODB,
            "cannot create blob from '{}': it is a directory",
            Bytes(content_path.to_bytes())
        );
        return Err(ErrorCode::DIRECTORY);
    }
    if let Some(o) = out_st {
        *o = st;
    }
    let size = st.st_size as u64;
    let mode = if hint_mode != 0 { hint_mode } else { st.st_mode };
    if posix::s_islnk(mode) {
        // SAFETY: a live odb.
        return unsafe { write_symlink(odb.0, content_path, size as usize) };
    }
    let fl = if try_load_filters {
        let hp = hint_path.unwrap_or(c"");
        // SAFETY: a live repository, no blob.
        unsafe { FilterList::load(repo, ptr::null_mut(), hp, cfilter::GIT_FILTER_TO_ODB, cfilter::GIT_FILTER_DEFAULT)? }
    } else {
        None
    };
    match fl {
        // No filters: stream from disk.
        // SAFETY: a live odb.
        None => unsafe { write_file_stream(odb.0, content_path, size) },
        Some(fl) => {
            // `write_file_filtered`.
            let mut tgt = GitStr::new();
            // SAFETY: the repository the list was loaded for.
            unsafe { fl.apply_to_file(&mut tgt, repo, content_path)? };
            // SAFETY: a live odb.
            unsafe { odb_write(odb.0, tgt.as_bytes(), GIT_OBJECT_BLOB) }
        }
    }
}

/// `git_blob_create_from_disk`: a blob of the file at `path`, filtered as
/// its work-tree path when it is in the work tree.
///
/// # Safety
///
/// `repo` is a live repository.
pub unsafe fn create_from_disk(repo: *mut git_repository, path: &CStr) -> Result<Oid> {
    let mut full_path = GitStr::new();
    crate::util::fs_path::prettify(&mut full_path, path, None)?;
    // SAFETY: a live repository; the work dir lives as long as it does.
    let workdir = unsafe { p::git_repository_workdir(repo) };
    let full = full_path.as_bytes();
    let mut hint: Option<&CStr> = None;
    if !workdir.is_null() {
        // SAFETY: a NUL-terminated string of the repository.
        let w = unsafe { CStr::from_ptr(workdir) }.to_bytes();
        if crate::util::util::prefixcmp(full, w) == 0 {
            hint = full_path.as_c_str().map(|c| {
                // A suffix of a NUL-terminated string is one.
                CStr::from_bytes_with_nul(&c.to_bytes_with_nul()[w.len()..]).unwrap_or(c"")
            });
        }
    }
    let content = full_path.as_c_str();
    // SAFETY: a live repository.
    unsafe { create_from_paths(None, repo, content, hint, 0, hint.is_some()) }
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
pub struct BlobWriteStream {
    fbuf: Box<GitFilebuf>,
    repo: *mut git_repository,
    hintpath: Option<GitCStr>,
}

// SAFETY: the state owns its filebuf and path; `repo` is only passed to C,
// which (libgit2's contract for streams) uses it on whichever thread uses
// the stream, one at a time. Every slot takes `&mut`, so `&Self` is never
// shared across threads by the table.
unsafe impl Send for BlobWriteStream {}
// SAFETY: as above.
unsafe impl Sync for BlobWriteStream {}

impl Drop for BlobWriteStream {
    fn drop(&mut self) {
        // `blob_writestream_free`.
        self.fbuf.cleanup();
    }
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
        on busy: -1,
        |s: &mut BlobWriteStream| {
            // SAFETY: C passes `len` readable bytes at `buffer`.
            let data = unsafe { crate::util::export::bytes(buffer, len) };
            crate::util::export::status(s.fbuf.write(data))
        },
    }
    /// `blob_writestream_close`.
    fn close(@stream) -> c_int {
        on panic: { errors::set_panic_error(); -1 },
        on busy: -1,
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
/// `repo` is a live repository that outlives the stream.
pub unsafe fn create_from_stream(
    repo: *mut git_repository,
    hintpath: Option<&CStr>,
) -> Result<CTable<WriteStream, BlobWriteStream>> {
    let hintpath = match hintpath {
        Some(h) => {
            // SAFETY: a fresh NUL-terminated copy, or NULL.
            Some(unsafe { GitCStr::from_raw(alloc::strdup(h)) }.ok_or(ErrorCode::GENERIC)?)
        }
        None => None,
    };
    let mut path = GitStr::new();
    // `GIT_REPOSITORY_ITEM_OBJECTS`.
    // SAFETY: a live repository, a valid `git_str`.
    ErrorCode::check(unsafe { p::git_repository__item_path(&mut path, repo, 4) })?;
    path.joinpath_self(b"streamed")?;
    let mut fbuf = Box::new(GitFilebuf::new());
    fbuf.open_withsize(path.as_bytes(), GIT_FILEBUF_TEMPORARY, 0o666, 2 * 1024 * 1024)?;
    Ok(CTable::new(BlobWriteStream {
        fbuf,
        repo,
        hintpath,
    }))
}

/// `git_blob_create_from_stream_commit`: flush the stream and create the
/// blob (the stream is freed).
pub fn stream_commit(mut stream: CTable<WriteStream, BlobWriteStream>) -> Result<Oid> {
    let s = stream.state_mut();
    if s.fbuf.flush() < 0 {
        return Err(ErrorCode::GENERIC);
    }
    let lock = s.fbuf.lock_path().map(|c| c.to_owned());
    let hint = s.hintpath.as_ref().map(|h| h.as_c_str());
    // SAFETY: the stream's repository, which outlives it (the contract of
    // `create_from_stream`).
    unsafe { create_from_paths(None, s.repo, lock.as_deref(), hint, 0, hint.is_some()) }
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
    let fl = unsafe { FilterList::load_ext(b.object.repo, o, path, cfilter::GIT_FILTER_TO_WORKTREE, &mut fo)? };
    // SAFETY: a valid `git_buf`, a live blob; a NULL list copies the
    // contents.
    unsafe { cfilter::apply_to_blob(out, fl.as_ref().map_or(ptr::null_mut(), FilterList::as_ptr), o) }
}

// ---------------------------------------------------------------------------
// The safe API
// ---------------------------------------------------------------------------

/// A blob of a repository.
#[derive(Clone, Debug)]
pub struct Blob<'repo>(Object<'repo>);

impl<'repo> Blob<'repo> {
    pub(crate) fn try_from_object(o: Object<'repo>) -> core::result::Result<Blob<'repo>, Object<'repo>> {
        if o.kind() == ObjectType::Blob { Ok(Blob(o)) } else { Err(o) }
    }

    /// `git_blob_lookup`.
    pub fn lookup(repo: &'repo Repository, id: &Oid) -> core::result::Result<Blob<'repo>, Error> {
        Ok(Blob(Object::lookup(repo, id, ObjectType::Blob)?))
    }

    /// `git_blob_lookup_prefix`.
    pub fn lookup_prefix(repo: &'repo Repository, id: &Oid, len: usize) -> core::result::Result<Blob<'repo>, Error> {
        Ok(Blob(Object::lookup_prefix(repo, id, len, ObjectType::Blob)?))
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
    pub fn filter(&self, path: &[u8], opts: &BlobFilterOptions) -> core::result::Result<Option<Vec<u8>>, Error> {
        let path = std::ffi::CString::new(path)
            .map_err(|_| Error::from(git_error_set!(ErrorClass::INVALID, "path contains a NUL byte")))?;
        let o = opts.raw();
        if o.flags & GIT_BLOB_FILTER_CHECK_FOR_BINARY != 0 && self.is_binary() {
            return Ok(None);
        }
        let mut out = GitBuf::new();
        // SAFETY: a live blob of a repository and a valid `git_buf`.
        unsafe { filter(&mut out, self.0.as_ptr().cast(), &path, &o)? };
        Ok(Some(out.into_vec()))
    }

    /// `git_blob_create_from_buffer`.
    pub fn create_from_buffer(repo: &Repository, data: &[u8]) -> core::result::Result<Oid, Error> {
        // SAFETY: a live repository.
        Ok(unsafe { create_from_buffer(repo.as_ptr(), data)? })
    }

    /// `git_blob_create_from_workdir`: a blob of the work-tree file at
    /// `path` (relative), filtered.
    pub fn create_from_workdir(repo: &Repository, path: &[u8]) -> core::result::Result<Oid, Error> {
        let path = std::ffi::CString::new(path)
            .map_err(|_| Error::from(git_error_set!(ErrorClass::INVALID, "path contains a NUL byte")))?;
        // SAFETY: a live repository.
        Ok(unsafe { create_from_paths(None, repo.as_ptr(), None, Some(&path), 0, true)? })
    }

    /// `git_blob_create_from_disk`.
    pub fn create_from_disk(repo: &Repository, path: &std::path::Path) -> core::result::Result<Oid, Error> {
        use std::os::unix::ffi::OsStrExt as _;
        let path = std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_| Error::from(git_error_set!(ErrorClass::INVALID, "path contains a NUL byte")))?;
        // SAFETY: a live repository.
        Ok(unsafe { create_from_disk(repo.as_ptr(), &path)? })
    }

    /// `git_blob_create_from_stream`: a writer whose
    /// [`commit`](BlobWriter::commit) creates the blob.
    pub fn writer<'r>(repo: &'r Repository, hintpath: Option<&[u8]>) -> core::result::Result<BlobWriter<'r>, Error> {
        let hint = match hintpath {
            Some(h) => Some(
                std::ffi::CString::new(h)
                    .map_err(|_| Error::from(git_error_set!(ErrorClass::INVALID, "path contains a NUL byte")))?,
            ),
            None => None,
        };
        // SAFETY: a live repository, which the writer borrows.
        let t = unsafe { create_from_stream(repo.as_ptr(), hint.as_deref())? };
        Ok(BlobWriter {
            table: t,
            _repo: core::marker::PhantomData,
        })
    }
}

/// `git_blob_data_is_binary`.
pub fn data_is_binary(data: &[u8]) -> bool {
    crate::util::str::is_binary(data)
}

/// A blob being written (`git_blob_create_from_stream`).
pub struct BlobWriter<'repo> {
    table: CTable<WriteStream, BlobWriteStream>,
    _repo: core::marker::PhantomData<&'repo Repository>,
}

impl BlobWriter<'_> {
    /// Append `data`.
    pub fn write(&mut self, data: &[u8]) -> core::result::Result<(), Error> {
        Ok(self.table.state_mut().fbuf.write(data)?)
    }

    /// `git_blob_create_from_stream_commit`.
    pub fn commit(self) -> core::result::Result<Oid, Error> {
        Ok(stream_commit(self.table)?)
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

/// Options of [`Blob::filter`] (`git_blob_filter_options`).
#[derive(Clone, Copy, Debug)]
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

/// The exports of `blob.c`.
pub mod ffi {
    use super::*;
    use crate::object::ffi::check_version;
    use crate::util::export::{barrier, bytes, cstr, status};

    /// `git_blob_rawcontent`.
    ///
    /// # Safety
    ///
    /// `blob` is NULL or a live blob.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_rawcontent(blob: *const GitBlob) -> *const c_void {
        git_assert_arg!(!blob.is_null(), "blob", ptr::null());
        // SAFETY: a live blob, read only.
        unsafe { (*blob).raw_parts() }.0.cast()
    }

    /// `git_blob_rawsize` (`(git_object_size_t)-1` for NULL, as C's
    /// `GIT_ASSERT_ARG`).
    ///
    /// # Safety
    ///
    /// `blob` is NULL or a live blob.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_rawsize(blob: *const GitBlob) -> u64 {
        git_assert_arg!(!blob.is_null(), "blob", u64::MAX);
        // SAFETY: a live blob, read only.
        unsafe { (*blob).raw_parts() }.1 as u64
    }

    /// `git_blob__getbuf`.
    ///
    /// # Safety
    ///
    /// `buffer` is a valid `git_str` and `blob` a live blob.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob__getbuf(buffer: *mut GitStr, blob: *mut GitBlob) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract: distinct, valid objects.
            status(getbuf(unsafe { &mut *buffer }, unsafe { &*blob }))
        })
    }

    /// `git_blob__create_from_paths`.
    ///
    /// # Safety
    ///
    /// `id` is writable; `out_st` NULL or writable; `repo` a live
    /// repository; the paths NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob__create_from_paths(
        id: *mut Oid,
        out_st: *mut posix::Stat,
        repo: *mut git_repository,
        content_path: *const c_char,
        hint_path: *const c_char,
        hint_mode: libc::mode_t,
        try_load_filters: bool,
    ) -> c_int {
        barrier(|| {
            // SAFETY: NULL or writable, used only here.
            let st = unsafe { out_st.as_mut() };
            // SAFETY: NULL or NUL-terminated.
            let (c, h) = unsafe { (cstr(content_path), cstr(hint_path)) };
            // SAFETY: a live repository.
            match unsafe { create_from_paths(st, repo, c, h, hint_mode, try_load_filters) } {
                Ok(oid) => {
                    // SAFETY: writable.
                    unsafe { id.write(oid) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    fn write_oid(id: *mut Oid, r: Result<Oid>) -> c_int {
        match r {
            Ok(oid) => {
                // SAFETY: the exports' callers pass a writable id.
                unsafe { id.write(oid) };
                0
            }
            Err(e) => e.raw(),
        }
    }

    /// `git_blob_create_from_buffer`.
    ///
    /// # Safety
    ///
    /// `id` is NULL or writable; `repo` NULL or live; `buffer` readable for
    /// `len` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_create_from_buffer(
        id: *mut Oid,
        repo: *mut git_repository,
        buffer: *const c_void,
        len: usize,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!id.is_null(), "id", -1);
            git_assert_arg!(!repo.is_null(), "repo", -1);
            // SAFETY: readable for `len` bytes.
            let data = unsafe { bytes(buffer.cast(), len) };
            // SAFETY: a live repository.
            write_oid(id, unsafe { create_from_buffer(repo, data) })
        })
    }

    /// `git_blob_create_from_workdir`.
    ///
    /// # Safety
    ///
    /// `id` is writable; `repo` live; `path` NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_create_from_workdir(
        id: *mut Oid,
        repo: *mut git_repository,
        path: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: NULL or NUL-terminated.
            let h = unsafe { cstr(path) };
            // SAFETY: a live repository.
            write_oid(id, unsafe { create_from_paths(None, repo, None, h, 0, true) })
        })
    }

    /// `git_blob_create_from_disk`.
    ///
    /// # Safety
    ///
    /// As [`git_blob_create_from_workdir`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_create_from_disk(
        id: *mut Oid,
        repo: *mut git_repository,
        path: *const c_char,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!path.is_null(), "path", -1);
            // SAFETY: NUL-terminated.
            let path = unsafe { CStr::from_ptr(path) };
            // SAFETY: a live repository.
            write_oid(id, unsafe { create_from_disk(repo, path) })
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
        out: *mut *mut GitWriteStream,
        repo: *mut git_repository,
        hintpath: *const c_char,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null(), "out", -1);
            git_assert_arg!(!repo.is_null(), "repo", -1);
            // SAFETY: NULL or NUL-terminated.
            let h = unsafe { cstr(hintpath) };
            // SAFETY: the caller's contract.
            match unsafe { create_from_stream(repo, h) } {
                Ok(t) => {
                    // SAFETY: writable; the table goes to C.
                    unsafe { out.write(t.into_c().as_ptr()) };
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
    /// `out` is writable; `stream` came from
    /// [`git_blob_create_from_stream`], has not been freed, and is not used
    /// again (no slot of it runs now).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_create_from_stream_commit(
        out: *mut Oid,
        stream: *mut GitWriteStream,
    ) -> c_int {
        barrier(|| {
            let Some(s) = NonNull::new(stream) else {
                return errors::assert_arg_failed("stream").raw();
            };
            // SAFETY: the caller's contract: a stream of
            // `git_blob_create_from_stream`, surrendered with `into_c`, which
            // C hands back for good.
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
    pub unsafe extern "C" fn git_blob_is_binary(blob: *const GitBlob) -> c_int {
        barrier(|| {
            git_assert_arg!(!blob.is_null(), "blob", -1);
            // SAFETY: a live blob, read only.
            c_int::from(content_is_binary(unsafe { &*blob }.content()))
        })
    }

    /// `git_blob_data_is_binary`.
    ///
    /// # Safety
    ///
    /// `str` is readable for `len` bytes (or `len` is 0).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_data_is_binary(str: *const c_char, len: usize) -> c_int {
        barrier(|| {
            // SAFETY: readable for `len` bytes.
            c_int::from(data_is_binary(unsafe { bytes(str, len) }))
        })
    }

    /// `git_blob_filter_options_init`.
    ///
    /// # Safety
    ///
    /// `opts` is writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_filter_options_init(
        opts: *mut GitBlobFilterOptions,
        version: c_uint,
    ) -> c_int {
        barrier(|| {
            // SAFETY: a readable `unsigned int`.
            if let Err(e) = unsafe { check_version(&version, GIT_BLOB_FILTER_OPTIONS_VERSION as c_uint, "git_blob_filter_options") } {
                return e.raw();
            }
            // SAFETY: writable.
            unsafe { opts.write(GitBlobFilterOptions::INIT) };
            0
        })
    }

    /// `git_blob_filter`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or a valid `git_buf`; `blob` NULL or a live blob of a
    /// repository; `path` NULL or NUL-terminated; `given_opts` NULL or
    /// valid.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_blob_filter(
        out: *mut GitBuf,
        blob: *mut GitBlob,
        path: *const c_char,
        given_opts: *const GitBlobFilterOptions,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!blob.is_null(), "blob", -1);
            git_assert_arg!(!path.is_null(), "path", -1);
            git_assert_arg!(!out.is_null(), "out", -1);
            // SAFETY: NULL or valid (`version` is its first member).
            if let Err(e) = unsafe {
                check_version(
                    given_opts.cast::<c_uint>(),
                    GIT_BLOB_FILTER_OPTIONS_VERSION as c_uint,
                    "git_blob_filter_options",
                )
            } {
                return e.raw();
            }
            // SAFETY: NULL or valid.
            let opts = unsafe { given_opts.as_ref() }.copied().unwrap_or(GitBlobFilterOptions::INIT);
            // SAFETY: NUL-terminated; a live blob; a valid `git_buf`.
            unsafe { status(filter(out, NonNull::new_unchecked(blob), CStr::from_ptr(path), &opts)) }
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
