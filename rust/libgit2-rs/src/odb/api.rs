//! The safe Rust API of the object database: [`Odb`], [`OdbObject`],
//! [`ObjectType`], backends ([`Backend`], [`BackendRef`], the
//! [`OdbBackend`] trait, [`Mempack`]) and streams ([`OdbWriter`]: `Write`,
//! [`OdbReader`]: `Read`, [`OdbPackWriter`]: `Write`).
//!
//! ```
//! # #[cfg(not(miri))] {
//! use libgit2_rs::{Backend, ObjectType, Odb};
//! let odb = Odb::new()?;
//! let (backend, mempack) = Backend::mempack();
//! odb.add_backend(backend, 1)?;
//! let id = odb.write(b"hello\n", ObjectType::Blob)?;
//! assert_eq!(id.to_string(), "ce013625030ba8dba906f756967f9e9ca394464a");
//! assert_eq!(odb.read(&id)?.data(), b"hello\n");
//! assert_eq!(mempack.object_count(), 1);
//! # }
//! # Ok::<(), libgit2_rs::Error>(())
//! ```

use core::ffi::{c_char, c_int, c_void};
use core::marker::PhantomData;
use core::ptr::NonNull;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use super::*;
use crate::odb_mempack::MempackState;
use crate::util::errors::Error;

type ApiResult<T> = core::result::Result<T, Error>;

fn err(code: ErrorCode) -> Error {
    Error::from(code)
}

/// `path` as a C string; a NUL inside it is refused (C would see a
/// shorter path).
fn c_path(path: &Path) -> ApiResult<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    c_bytes(path.as_os_str().as_bytes())
}

/// Bytes as a C string, a NUL inside refused.
fn c_bytes(path: &[u8]) -> ApiResult<std::ffi::CString> {
    std::ffi::CString::new(path).map_err(|_| {
        Error::with(
            ErrorCode::INVALID,
            ErrorClass::INVALID,
            "path contains a NUL byte",
        )
    })
}

// ---------------------------------------------------------------------------
// Odb
// ---------------------------------------------------------------------------

/// An object database (`git_odb`): refcounted (`Clone` shares it), usable
/// from several threads at once (`Send + Sync` as `CShared<GitOdb>`).
#[derive(Clone)]
pub struct Odb<'r>(
    CShared<GitOdb>,
    /// The repository the odb came from (`'static` for one made alone):
    /// an odb of a repository uses its owner's object cache, so it cannot
    /// outlive it.
    PhantomData<&'r ()>,
);

impl core::fmt::Debug for Odb<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Odb")
            .field("oid_type", &self.oid_type())
            .field("backends", &self.num_backends())
            .finish()
    }
}

impl<'r> Odb<'r> {
    /// The odb (crate-internal: the repository calls it directly).
    pub(crate) fn inner(&self) -> &GitOdb {
        &self.0
    }

    /// The odb's reference, as a handle.
    pub(crate) fn from_shared(db: CShared<GitOdb>) -> Odb<'r> {
        Odb(db, PhantomData)
    }

    /// The handle's reference.
    pub(crate) fn into_shared(self) -> CShared<GitOdb> {
        self.0
    }

    /// A new reference to `raw` (`GIT_REFCOUNT_INC`).
    ///
    /// # Safety
    ///
    /// `raw` is a live odb.
    pub(crate) unsafe fn new_ref(raw: NonNull<GitOdb>) -> Odb<'r> {
        // SAFETY: `raw` is a live odb made by `CShared` (this fn's
        // contract); `from_borrowed` takes a new reference to it.
        let db = unsafe { CShared::from_borrowed(raw.as_ptr()) }.expect("non-null");
        Odb(db, PhantomData)
    }

    /// The `git_odb *` (the reference stays with this handle).
    pub(crate) fn as_raw(&self) -> NonNull<GitOdb> {
        // SAFETY: a `CShared`'s pointer is not NULL.
        unsafe { NonNull::new_unchecked(CShared::as_ptr(&self.0)) }
    }

    /// An odb without backends, for SHA-1 ids (`git_odb_new`).
    pub fn new() -> ApiResult<Odb<'static>> {
        Self::with_oid_type(OidType::Sha1)
    }

    /// An odb without backends for ids of `oid_type` (`git_odb_new_ext`).
    pub fn with_oid_type(oid_type: OidType) -> ApiResult<Odb<'static>> {
        crate::libgit2::ensure_init()?;
        let opts = GitOdbOptions {
            version: GIT_ODB_OPTIONS_VERSION,
            oid_type: oid_type.raw(),
        };
        Ok(Odb::from_shared(GitOdb::new(Some(&opts)).map_err(err)?))
    }

    /// The odb of an `objects` directory, with its loose and pack backends
    /// and its alternates (`git_odb_open_ext`).
    pub fn open(objects_dir: impl AsRef<Path>, oid_type: OidType) -> ApiResult<Odb<'static>> {
        let dir = c_path(objects_dir.as_ref())?;
        let odb = Self::with_oid_type(oid_type)?;
        odb.inner()
            .add_default_backends(&dir, false, 0)
            .map_err(err)?;
        Ok(odb)
    }

    /// The type of the ids.
    pub fn oid_type(&self) -> OidType {
        OidType::from_raw(self.inner().oid_type()).unwrap_or(OidType::Sha1)
    }

    /// Add `backend` at `priority` (higher first; `git_odb_add_backend`).
    pub fn add_backend(&self, backend: Backend, priority: i32) -> ApiResult<()> {
        self.add(backend, priority, false)
    }

    /// Add `backend` as an alternate: it is read, never written
    /// (`git_odb_add_alternate`).
    pub fn add_alternate(&self, backend: Backend, priority: i32) -> ApiResult<()> {
        self.add(backend, priority, true)
    }

    fn add(&self, backend: Backend, priority: i32, alternate: bool) -> ApiResult<()> {
        let b = backend.0;
        // SAFETY: `b` is the live table this handle owns, which passes to
        // the odb on success (and is forgotten below).
        match unsafe { self.inner().add_backend(b, priority, alternate) } {
            Ok(()) => {
                // The odb owns it now.
                core::mem::forget(backend);
                Ok(())
            }
            Err(e) => Err(err(e)),
        }
    }

    /// Add the loose and pack backends of another `objects` directory as
    /// alternates (`git_odb_add_disk_alternate`).
    pub fn add_disk_alternate(&self, path: impl AsRef<Path>) -> ApiResult<()> {
        let dir = c_path(path.as_ref())?;
        self.inner()
            .add_default_backends(&dir, true, 0)
            .map_err(err)
    }

    /// The number of backends (`git_odb_num_backends`).
    pub fn num_backends(&self) -> usize {
        self.inner().num_backends()
    }

    /// The backend at `pos` (`git_odb_get_backend`), in priority order
    /// ([`ErrorCode::NOT_FOUND`] past the end). The odb keeps owning it:
    /// the handle borrows the odb.
    pub fn get_backend(&self, pos: usize) -> ApiResult<BackendRef<'_>> {
        let raw = self.inner().get_backend(pos).map_err(err)?;
        Ok(BackendRef {
            raw,
            _odb: PhantomData,
        })
    }

    /// `git_odb_write_pack`: a stream that receives a pack (the bytes of a
    /// `.pack` file) and indexes it into the first backend that can (the
    /// pack backend of an [`open`](Self::open)ed odb), on
    /// [`commit`](OdbPackWriter::commit).
    pub fn write_pack(&self) -> ApiResult<OdbPackWriter<'_>> {
        // SAFETY: `write_pack`'s one condition is on the progress callback,
        // and there is none. (The writepack's indexer keeps the odb's
        // address: the `OdbPackWriter` borrows `self`, whose reference keeps
        // the odb alive until the writepack is freed.)
        let wp = unsafe { self.inner().write_pack(None, core::ptr::null_mut()) }.map_err(err)?;
        let wp = NonNull::new(wp).ok_or_else(|| err(ErrorCode::GENERIC))?;
        Ok(OdbPackWriter {
            wp,
            stats: GitIndexerProgress::default(),
            _odb: PhantomData,
        })
    }

    /// `git_odb_write_multi_pack_index`: write a `multi-pack-index` of the
    /// packs of the first backend that can (the pack backend).
    pub fn write_multi_pack_index(&self) -> ApiResult<()> {
        self.inner().write_multi_pack_index().map_err(err)
    }

    /// `git_commit_graph_open` + `git_odb_set_commit_graph`: use the
    /// commit-graph file of `objects_dir` (its `info/commit-graph`, read
    /// when first needed) to speed up commit walks; `None` drops the
    /// odb's commit-graph.
    pub fn set_commit_graph(&self, objects_dir: Option<&Path>) -> ApiResult<()> {
        let graph = match objects_dir {
            None => None,
            Some(d) => {
                let d = c_path(d)?;
                Some(
                    crate::sys::commit_graph::CommitGraph::new(&d, self.oid_type().raw())
                        .map_err(err)?,
                )
            }
        };
        self.inner().set_commit_graph(graph);
        Ok(())
    }

    /// Read an object (`git_odb_read`).
    pub fn read(&self, id: &Oid) -> ApiResult<OdbObject> {
        OdbObject::checked(self.inner().read(id).map_err(err)?)
    }

    /// Read the object whose id starts with the first `len` hex digits of
    /// `short_id` (`git_odb_read_prefix`).
    pub fn read_prefix(&self, short_id: &Oid, len: usize) -> ApiResult<OdbObject> {
        OdbObject::checked(self.inner().read_prefix(short_id, len).map_err(err)?)
    }

    /// An object's size and type, without reading it if a backend can tell
    /// (`git_odb_read_header`).
    pub fn read_header(&self, id: &Oid) -> ApiResult<(usize, ObjectType)> {
        let (len, t) = self.inner().read_header(id).map_err(err)?;
        Ok((len, valid_type(t)?))
    }

    /// Whether the object exists (`git_odb_exists`), refreshing the
    /// backends if it is not found.
    pub fn exists(&self, id: &Oid) -> bool {
        self.inner().exists_ext(id, 0)
    }

    /// [`exists`](Self::exists) without the refresh
    /// (`GIT_ODB_LOOKUP_NO_REFRESH`).
    pub fn exists_no_refresh(&self, id: &Oid) -> bool {
        self.inner().exists_ext(id, GIT_ODB_LOOKUP_NO_REFRESH)
    }

    /// The full id of the one object whose id starts with the first `len`
    /// digits of `short_id` (`git_odb_exists_prefix`).
    pub fn exists_prefix(&self, short_id: &Oid, len: usize) -> ApiResult<Oid> {
        self.inner().exists_prefix(short_id, len).map_err(err)
    }

    /// Expand short ids and check types (`git_odb_expand_ids`): each query
    /// is completed, or cleared (length 0, type `None`) if not found or
    /// ambiguous.
    pub fn expand_ids(&self, ids: &mut [ExpandId]) -> ApiResult<()> {
        let mut raw = Vec::new();
        raw.try_reserve_exact(ids.len())
            .map_err(|_| err(errors::oom()))?;
        raw.extend(ids.iter().map(|q| GitOdbExpandId {
            id: q.id,
            length: q.length,
            type_: q.kind.map_or(0, ObjectType::raw),
        }));
        self.inner().expand_ids(&mut raw).map_err(err)?;
        for (q, r) in ids.iter_mut().zip(&raw) {
            q.id = r.id;
            q.length = r.length;
            q.kind = ObjectType::from_raw(r.type_);
        }
        Ok(())
    }

    /// Refresh the backends (`git_odb_refresh`): see new packs.
    pub fn refresh(&self) -> ApiResult<()> {
        self.inner().refresh().map_err(err)
    }

    /// Call `f` with each object's id (`git_odb_foreach`); `false` stops
    /// the walk (with `Ok`).
    pub fn foreach(&self, mut f: impl FnMut(&Oid) -> bool) -> ApiResult<()> {
        /// The payload: the closure, and whether it stopped the walk.
        struct Payload<'a> {
            f: &'a mut dyn FnMut(&Oid) -> bool,
            stopped: bool,
            panic: Option<Box<dyn core::any::Any + Send>>,
        }
        unsafe extern "C" fn trampoline(id: *const Oid, payload: *mut c_void) -> c_int {
            // SAFETY: `payload` is `&raw mut p` below, which `foreach_raw`
            // hands only to this trampoline, one call at a time (the odb
            // walks its backends in turn, and each calls back
            // sequentially); `p` is a live local of `foreach`, which does
            // not touch it until the walk returns, so this `&mut` is the
            // only reference to it while the call lasts.
            let p = unsafe { &mut *payload.cast::<Payload<'_>>() };
            // SAFETY: `id` is a valid id for the call: every caller of a
            // `git_odb_foreach_cb` passes one (the Rust backends pass a
            // `&Oid`; C's midx and packs, their local `git_oid`).
            let id = unsafe { &*id };
            if p.panic.is_some() {
                return 1;
            }
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (p.f)(id))) {
                Ok(true) => 0,
                Ok(false) => {
                    p.stopped = true;
                    1
                }
                Err(e) => {
                    p.panic = Some(e);
                    1
                }
            }
        }
        let mut p = Payload {
            f: &mut f,
            stopped: false,
            panic: None,
        };
        // SAFETY: `foreach_raw` requires that `cb` may be called with
        // `payload`: the trampoline reads `payload` as a `Payload`, which it
        // is, and `p` outlives the call.
        let r = unsafe {
            self.inner()
                .foreach_raw(Some(trampoline), (&raw mut p).cast::<c_void>())
        };
        if let Some(e) = p.panic.take() {
            std::panic::resume_unwind(e);
        }
        if r == 0 || p.stopped {
            if p.stopped {
                errors::clear();
            }
            Ok(())
        } else {
            Err(err(ErrorCode::from_raw(r).unwrap_or(ErrorCode::GENERIC)))
        }
    }

    /// Write an object (`git_odb_write`); its id.
    pub fn write(&self, data: &[u8], kind: ObjectType) -> ApiResult<Oid> {
        let mut id = None;
        self.inner().write(data, kind.raw(), &mut id).map_err(err)?;
        id.ok_or_else(|| err(ErrorCode::GENERIC))
    }

    /// A stream to write an object of `size` bytes (`git_odb_open_wstream`).
    pub fn writer(&self, size: u64, kind: ObjectType) -> ApiResult<OdbWriter<'_>> {
        let stream = self.inner().open_wstream(size, kind.raw()).map_err(err)?;
        Ok(OdbWriter {
            stream,
            _odb: PhantomData,
        })
    }

    /// A stream to read an object, its size and type
    /// (`git_odb_open_rstream`).
    pub fn reader(&self, id: &Oid) -> ApiResult<(OdbReader<'_>, usize, ObjectType)> {
        let Some((stream, len, t)) = self.inner().open_rstream(id).map_err(err)? else {
            return Err(Error::with(
                ErrorCode::GENERIC,
                ErrorClass::ODB,
                "backend returned no stream",
            ));
        };
        let reader = OdbReader {
            stream,
            _odb: PhantomData,
        };
        Ok((reader, len, valid_type(t)?))
    }

    /// The id `data` would have as an object of `kind` (`git_odb_hash`).
    pub fn hash(data: &[u8], kind: ObjectType, oid_type: OidType) -> ApiResult<Oid> {
        crate::libgit2::ensure_init()?;
        hash_object(Some(data), data.len(), kind.raw(), oid_type.raw()).map_err(err)
    }
}

/// The type `t` a backend gave, or the error of a type that is not one of
/// the four (a backend's garbage, which C would hand on).
fn valid_type(t: git_object_t) -> ApiResult<ObjectType> {
    ObjectType::from_raw(t)
        .ok_or_else(|| Error::with(ErrorCode::GENERIC, ErrorClass::ODB, "invalid object type"))
}

/// A query of [`Odb::expand_ids`] (`git_odb_expand_id`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpandId {
    /// The (short) id.
    pub id: Oid,
    /// Its length in hex digits.
    pub length: u16,
    /// The type wanted (`None`: any), then found.
    pub kind: Option<ObjectType>,
}

// ---------------------------------------------------------------------------
// Objects
// ---------------------------------------------------------------------------

/// An object read from an odb (`git_odb_object`): immutable, refcounted
/// (`Clone` shares it), usable from any thread.
pub struct OdbObject(NonNull<GitOdbObject>);

// SAFETY: field by field, `GitOdbObject` is `cached` (id, type, flags and
// size, written only while the object is made, and `refcount`, an atomic)
// and `buffer` (a `git__allocator` block the object owns, never written
// once made). So shared use from several threads only reads, but for the
// atomic count; the last reference frees the object and its buffer
// (`git__free`, usable from any thread) on whichever thread drops it.
unsafe impl Send for OdbObject {}
// SAFETY: as for `Send` just above.
unsafe impl Sync for OdbObject {}

impl OdbObject {
    /// The handle of the object at `p` (our reference), if its type is
    /// one of the four (a backend may make one up with hash verification
    /// off).
    fn checked(p: NonNull<GitOdbObject>) -> ApiResult<OdbObject> {
        let o = OdbObject(p);
        valid_type(git_object_t::from(o.obj().cached.type_))?;
        Ok(o)
    }

    fn obj(&self) -> &GitOdbObject {
        // SAFETY: the handle holds a reference: the object is live, and
        // only its count (atomic) changes.
        unsafe { self.0.as_ref() }
    }

    /// The id (`git_odb_object_id`).
    pub fn id(&self) -> &Oid {
        &self.obj().cached.oid
    }

    /// The data (`git_odb_object_data`, `git_odb_object_size` bytes).
    pub fn data(&self) -> &[u8] {
        let o = self.obj();
        if o.buffer.is_null() || o.cached.size == 0 {
            return &[];
        }
        // SAFETY: `buffer` holds `size` initialized bytes and a NUL
        // (`GitOdbObject`'s invariant: `odb_object_alloc` takes a backend's
        // `RawObj`, at least `len` readable bytes), never written once the
        // object is made, and freed only with the object, which `self`
        // keeps alive for the returned borrow. `bytes` panics past
        // `isize::MAX`.
        unsafe { crate::util::export::bytes(o.buffer.cast::<c_char>(), o.cached.size) }
    }

    /// The size (`git_odb_object_size`).
    pub fn len(&self) -> usize {
        self.obj().cached.size
    }

    /// Whether the object is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The type (`git_odb_object_type`).
    pub fn kind(&self) -> ObjectType {
        // `checked` made sure it is one of the four.
        ObjectType::from_raw(git_object_t::from(self.obj().cached.type_))
            .unwrap_or(ObjectType::Blob)
    }
}

impl Clone for OdbObject {
    fn clone(&self) -> OdbObject {
        self.obj().cached.refcount.inc();
        OdbObject(self.0)
    }
}

impl Drop for OdbObject {
    fn drop(&mut self) {
        // SAFETY: the object is live (this handle holds a reference, which
        // it gives up here, once); it was made by `odb_object_alloc`, whose
        // `flags` make `decref` free it as an odb object.
        unsafe { cache::decref(self.0.cast()) }
    }
}

impl core::fmt::Debug for OdbObject {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OdbObject")
            .field("id", self.id())
            .field("kind", &self.kind())
            .field("len", &self.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Streams
// ---------------------------------------------------------------------------

fn io_error(code: c_int) -> std::io::Error {
    std::io::Error::other(Error::from(
        ErrorCode::from_raw(code).unwrap_or(ErrorCode::GENERIC),
    ))
}

/// A stream that writes an object (`git_odb_open_wstream`): write exactly
/// the declared size, then [`finalize`](Self::finalize).
pub struct OdbWriter<'odb> {
    stream: NonNull<GitOdbStream>,
    _odb: PhantomData<&'odb ()>,
}

impl OdbWriter<'_> {
    /// Store the object (`git_odb_stream_finalize_write`); its id.
    pub fn finalize(self) -> ApiResult<Oid> {
        let mut id = Oid::NONE;
        // SAFETY: `git_odb_stream_finalize_write`'s conditions: `id` is a
        // local, readable and writable; `self.stream` is the live stream
        // `open_wstream` gave, used by this handle alone (it is neither
        // `Send` nor `Sync`, and holds no reference into the stream); its
        // backend belongs to the odb that `'odb` keeps alive.
        let r =
            unsafe { super::ffi::git_odb_stream_finalize_write(&raw mut id, self.stream.as_ptr()) };
        if r < 0 {
            return Err(err(ErrorCode::from_raw(r).unwrap_or(ErrorCode::GENERIC)));
        }
        Ok(id)
    }
}

impl std::io::Write for OdbWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // SAFETY: `git_odb_stream_write`'s conditions: the live stream from
        // `open_wstream`, used by this handle alone (`&mut self`, not
        // `Send`); `buf` is readable for `buf.len()` bytes, which nothing
        // writes while it is borrowed.
        let r = unsafe {
            super::ffi::git_odb_stream_write(self.stream.as_ptr(), buf.as_ptr().cast(), buf.len())
        };
        if r < 0 {
            return Err(io_error(r));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for OdbWriter<'_> {
    fn drop(&mut self) {
        // SAFETY: the live stream from `open_wstream`, not used after this;
        // its `hash_ctx` is the odb's, held by the stream alone.
        unsafe { stream_free(self.stream.as_ptr()) }
    }
}

impl core::fmt::Debug for OdbWriter<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OdbWriter").finish_non_exhaustive()
    }
}

/// A stream that reads an object (`git_odb_open_rstream`).
pub struct OdbReader<'odb> {
    stream: NonNull<GitOdbStream>,
    _odb: PhantomData<&'odb ()>,
}

impl std::io::Read for OdbReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // SAFETY: `git_odb_stream_read`'s conditions: the live stream from
        // `open_rstream`, used by this handle alone (`&mut self`, not
        // `Send`); `buf` is writable for `buf.len()` bytes.
        let r = unsafe {
            super::ffi::git_odb_stream_read(
                self.stream.as_ptr(),
                buf.as_mut_ptr().cast(),
                buf.len(),
            )
        };
        if r < 0 {
            return Err(io_error(r));
        }
        Ok(r as usize)
    }
}

impl Drop for OdbReader<'_> {
    fn drop(&mut self) {
        // SAFETY: the live stream from `open_rstream`, not used after this;
        // its `hash_ctx` is NULL or the backend's, held by the stream alone.
        unsafe { stream_free(self.stream.as_ptr()) }
    }
}

impl core::fmt::Debug for OdbReader<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OdbReader").finish_non_exhaustive()
    }
}

/// `git_indexer_progress`, mirrored (`git2/indexer.h`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct GitIndexerProgress {
    total_objects: core::ffi::c_uint,
    indexed_objects: core::ffi::c_uint,
    received_objects: core::ffi::c_uint,
    local_objects: core::ffi::c_uint,
    total_deltas: core::ffi::c_uint,
    indexed_deltas: core::ffi::c_uint,
    received_bytes: usize,
}

/// The progress of indexing a pack (`git_indexer_progress`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct IndexerProgress {
    /// The objects in the pack.
    pub total_objects: u32,
    /// The objects indexed.
    pub indexed_objects: u32,
    /// The objects received.
    pub received_objects: u32,
    /// The objects that a thin pack's deltas needed from the odb.
    pub local_objects: u32,
    /// The deltas in the pack.
    pub total_deltas: u32,
    /// The deltas resolved.
    pub indexed_deltas: u32,
    /// The bytes received.
    pub received_bytes: usize,
}

impl From<GitIndexerProgress> for IndexerProgress {
    fn from(p: GitIndexerProgress) -> IndexerProgress {
        IndexerProgress {
            total_objects: p.total_objects,
            indexed_objects: p.indexed_objects,
            received_objects: p.received_objects,
            local_objects: p.local_objects,
            total_deltas: p.total_deltas,
            indexed_deltas: p.indexed_deltas,
            received_bytes: p.received_bytes,
        }
    }
}

/// A stream that writes a pack into an odb (`git_odb_write_pack`'s
/// `git_odb_writepack`): write the pack's bytes, then
/// [`commit`](Self::commit); dropped before, the pack is discarded.
pub struct OdbPackWriter<'odb> {
    wp: NonNull<GitOdbWritepack>,
    stats: GitIndexerProgress,
    _odb: PhantomData<&'odb ()>,
}

impl OdbPackWriter<'_> {
    /// The progress so far.
    pub fn progress(&self) -> IndexerProgress {
        self.stats.into()
    }

    /// Finish the pack: resolve its deltas, write its index and make it
    /// visible to the odb (`git_odb_writepack.commit`); the final
    /// progress.
    pub fn commit(mut self) -> ApiResult<IndexerProgress> {
        let wp = self.wp.as_ptr();
        // SAFETY: `wp` is the live writepack `write_pack` gave, used by this
        // handle alone (not `Send`, `self` owned): a read of one field in
        // place, no reference formed.
        let Some(commit) = (unsafe { (*wp).commit }) else {
            return Err(err(ErrorCode::GENERIC));
        };
        // SAFETY: the writepack's own `commit` slot, called as
        // `git2/sys/odb_backend.h` declares it: with the live writepack and
        // a writable `git_indexer_progress` (our field, mirrored, which
        // nothing else references during the call).
        let r = unsafe { commit(wp, (&raw mut self.stats).cast()) };
        if r < 0 {
            return Err(err(ErrorCode::from_raw(r).unwrap_or(ErrorCode::GENERIC)));
        }
        Ok(self.stats.into())
    }
}

impl std::io::Write for OdbPackWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let wp = self.wp.as_ptr();
        // SAFETY: as in `commit`: a field read of the live writepack.
        let Some(append) = (unsafe { (*wp).append }) else {
            return Err(io_error(ErrorCode::GENERIC.raw()));
        };
        // SAFETY: the writepack's own `append` slot, with the live
        // writepack, `buf` readable for `buf.len()` bytes (unwritten while
        // borrowed) and our writable progress field, as in `commit`.
        let r = unsafe {
            append(
                wp,
                buf.as_ptr().cast(),
                buf.len(),
                (&raw mut self.stats).cast(),
            )
        };
        if r < 0 {
            return Err(io_error(r));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for OdbPackWriter<'_> {
    fn drop(&mut self) {
        let wp = self.wp.as_ptr();
        // SAFETY: as in `commit`: a field read of the live writepack.
        if let Some(free) = unsafe { (*wp).free } {
            // SAFETY: the writepack's own `free` slot, called once, with the
            // live writepack, which nothing uses after.
            unsafe { free(wp) };
        }
    }
}

impl core::fmt::Debug for OdbPackWriter<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OdbPackWriter")
            .field("progress", &self.progress())
            .finish()
    }
}

crate::c_layout_test! {
    headers: ["git2/indexer.h"],
    types: [
        GitIndexerProgress => "git_indexer_progress" {
            total_objects, indexed_objects, received_objects, local_objects,
            total_deltas, indexed_deltas, received_bytes
        },
    ],
    asserts: [],
}

// ---------------------------------------------------------------------------
// Backends
// ---------------------------------------------------------------------------

/// A backend not yet added to an odb (`git_odb_backend *`); dropped, it is
/// freed. [`Odb::add_backend`] hands it to the odb.
pub struct Backend(NonNull<GitOdbBackend>);

// SAFETY: the one field is a table that only this handle reaches (not yet
// added to an odb), freed on drop through its `free` slot. Every
// constructor makes a `CTable<_, S>` whose state `S` is `Send + Sync`
// (asserted below): `LooseBackend`, `PackBackend` (`one_pack`, `pack`),
// `MempackBackend`, `RustBackend`. So moving the table to another thread,
// and freeing it there, moves only `Send` state.
unsafe impl Send for Backend {}
const _: () = {
    fn send_sync<T: Send + Sync>() {}
    let _ = send_sync::<crate::odb_loose::LooseBackend>;
    let _ = send_sync::<crate::odb_pack::PackBackend>;
    let _ = send_sync::<crate::odb_mempack::MempackBackend>;
    let _ = send_sync::<RustBackend>;
};

impl core::fmt::Debug for Backend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Backend").finish_non_exhaustive()
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        // SAFETY: a live table only this handle has (never added to an
        // odb: `Odb::add` forgets the handle when it does), not used again.
        unsafe { free_backend(self.0) }
    }
}

/// Options of the loose backend (`git_odb_backend_loose_options`): a
/// by-value builder.
///
/// ```
/// use libgit2_rs::{LooseOptions, OidType};
/// let o = LooseOptions::new().compression_level(9).fsync(true).oid_type(OidType::Sha1);
/// # let _ = o;
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LooseOptions {
    compression_level: Option<u32>,
    fsync: bool,
    dir_mode: Option<u32>,
    file_mode: Option<u32>,
    oid_type: OidType,
}

impl Default for LooseOptions {
    fn default() -> Self {
        LooseOptions {
            compression_level: None,
            fsync: false,
            dir_mode: None,
            file_mode: None,
            oid_type: OidType::Sha1,
        }
    }
}

impl LooseOptions {
    /// The defaults (`GIT_ODB_BACKEND_LOOSE_OPTIONS_INIT`): zlib's fastest
    /// level, no `fsync`, modes 0777 / 0444 (less the umask), SHA-1 ids.
    pub fn new() -> LooseOptions {
        LooseOptions::default()
    }

    /// zlib's level, 0 to 9 (more is 9).
    pub fn compression_level(mut self, level: u32) -> LooseOptions {
        self.compression_level = Some(level.min(9));
        self
    }

    /// `fsync` each object written (`GIT_ODB_BACKEND_LOOSE_FSYNC`).
    pub fn fsync(mut self, on: bool) -> LooseOptions {
        self.fsync = on;
        self
    }

    /// The mode of the directories made.
    pub fn dir_mode(mut self, mode: u32) -> LooseOptions {
        self.dir_mode = Some(mode);
        self
    }

    /// The mode of the files written.
    pub fn file_mode(mut self, mode: u32) -> LooseOptions {
        self.file_mode = Some(mode);
        self
    }

    /// The type of the ids.
    pub fn oid_type(mut self, t: OidType) -> LooseOptions {
        self.oid_type = t;
        self
    }
}

impl Backend {
    /// The loose backend of `objects_dir` (`git_odb_backend_loose`).
    pub fn loose(objects_dir: impl AsRef<Path>, opts: LooseOptions) -> ApiResult<Backend> {
        crate::libgit2::ensure_init()?;
        let dir = c_path(objects_dir.as_ref())?;
        let o = GitOdbBackendLooseOptions {
            version: 1,
            flags: if opts.fsync {
                GIT_ODB_BACKEND_LOOSE_FSYNC
            } else {
                0
            },
            compression_level: opts
                .compression_level
                .map_or(-1, |l| c_int::try_from(l.min(9)).unwrap_or(9)),
            dir_mode: opts.dir_mode.unwrap_or(0),
            file_mode: opts.file_mode.unwrap_or(0),
            oid_type: opts.oid_type.raw(),
        };
        crate::odb_loose::new_backend(dir.to_bytes(), Some(&o))
            .map(Backend)
            .map_err(err)
    }

    /// The pack backend of `objects_dir`: its `pack/` directory's packs
    /// and `multi-pack-index` (`git_odb_backend_pack`).
    pub fn pack(objects_dir: impl AsRef<Path>, oid_type: OidType) -> ApiResult<Backend> {
        crate::libgit2::ensure_init()?;
        let dir = c_path(objects_dir.as_ref())?;
        let opts = GitOdbBackendPackOptions {
            version: 1,
            oid_type: oid_type.raw(),
        };
        crate::odb_pack::backend_pack_raw(&dir, &opts)
            .map(Backend)
            .map_err(err)
    }

    /// The backend of the one pack whose index is `index_path` (a
    /// `.idx`, beside its `.pack`; `git_odb_backend_one_pack`).
    pub fn one_pack(index_path: impl AsRef<Path>, oid_type: OidType) -> ApiResult<Backend> {
        crate::libgit2::ensure_init()?;
        let idx = c_path(index_path.as_ref())?;
        let opts = GitOdbBackendPackOptions {
            version: 1,
            oid_type: oid_type.raw(),
        };
        crate::odb_pack::backend_one_pack(&idx, Some(&opts))
            .map(|t| Backend(t.into_c()))
            .map_err(err)
    }

    /// An in-memory backend (`git_mempack_new`), and a handle to its
    /// objects that stays valid once the backend is added to an odb.
    pub fn mempack() -> (Backend, Mempack) {
        let state = Arc::new(MempackState::default());
        let b = crate::odb_mempack::new_backend(state.clone());
        (Backend(b), Mempack(state))
    }

    /// A backend implemented in Rust.
    pub fn custom(backend: impl OdbBackend) -> Backend {
        let boxed: Box<dyn OdbBackend> = Box::new(backend);
        let state: *const dyn OdbBackend = &raw const *boxed;
        let table = ffibox::CTable::<BackendTable, RustBackend>::new(RustBackend(
            crate::api::NoUnwindDrop::new(boxed),
        ))
        .into_c();
        register(table, Known::Rust(RustState(state)));
        Backend(table)
    }
}

// ---------------------------------------------------------------------------
// Backends of an odb
// ---------------------------------------------------------------------------

/// A backend the safe API (or `git_mempack_new`) made, by its table.
enum Known {
    /// A mempack: its objects.
    Mempack(Weak<MempackState>),
    /// A Rust backend: its value, in its table's `Box`.
    Rust(RustState),
}

/// The address of a Rust backend's value.
struct RustState(*const dyn OdbBackend);

// SAFETY: the one field is an address that `KNOWN` holds; any thread
// compares it, and only `BackendRef::downcast_ref` dereferences it, as a
// `&` to a `dyn OdbBackend`, which is `Sync`, while the table it belongs to
// is live (its SAFETY comment). No `&mut` is ever made through it.
unsafe impl Send for RustState {}

/// The tables of the backends that are alive, with what they are; an entry
/// goes when its backend's state is dropped (before the table is freed),
/// so a table found here is that backend.
static KNOWN: Mutex<Vec<(usize, Known)>> = Mutex::new(Vec::new());

fn known() -> std::sync::MutexGuard<'static, Vec<(usize, Known)>> {
    KNOWN.lock().unwrap_or_else(PoisonError::into_inner)
}

fn register(table: NonNull<GitOdbBackend>, what: Known) {
    known().push((table.as_ptr() as usize, what));
}

/// Record that `table` is the mempack of `state` (`git_mempack_new`).
pub(crate) fn register_mempack(table: NonNull<GitOdbBackend>, state: &Arc<MempackState>) {
    register(table, Known::Mempack(Arc::downgrade(state)));
}

/// Forget the mempack of `state` (its table is being freed).
pub(crate) fn unregister_mempack(state: &Arc<MempackState>) {
    known().retain(
        |(_, k)| !matches!(k, Known::Mempack(w) if core::ptr::eq(w.as_ptr(), Arc::as_ptr(state))),
    );
}

/// A backend of an odb ([`Odb::get_backend`]), which the odb owns: it
/// borrows the odb. What the safe API made can be reached back through it
/// ([`as_mempack`](Self::as_mempack), [`downcast_ref`](Self::downcast_ref)).
pub struct BackendRef<'odb> {
    raw: NonNull<GitOdbBackend>,
    _odb: PhantomData<&'odb ()>,
}

impl core::fmt::Debug for BackendRef<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BackendRef").finish_non_exhaustive()
    }
}

impl<'odb> BackendRef<'odb> {
    /// The mempack's handle, if the backend is a mempack
    /// ([`Backend::mempack`] or `git_mempack_new`).
    pub fn as_mempack(&self) -> Option<Mempack> {
        let k = known();
        k.iter().find_map(|(t, what)| match what {
            Known::Mempack(w) if *t == self.raw.as_ptr() as usize => w.upgrade().map(Mempack),
            _ => None,
        })
    }

    /// The Rust backend, if the backend is a [`Backend::custom`] of a `T`.
    pub fn downcast_ref<T: OdbBackend>(&self) -> Option<&'odb T> {
        let p = {
            let k = known();
            k.iter().find_map(|(t, what)| match what {
                Known::Rust(RustState(p)) if *t == self.raw.as_ptr() as usize => Some(*p),
                _ => None,
            })?
        };
        // SAFETY: the table is a live Rust backend's (its entry goes before
        // its state is dropped), owned by the odb this handle borrows: the
        // odb frees its backends only when its last reference goes, and
        // the borrowed `Odb` holds one, so the value (in its `Box`, which
        // does not move) lives for `'odb`. It is only read (`&`), and it is
        // `Sync`.
        let b: &'odb dyn OdbBackend = unsafe { &*p };
        let any: &'odb dyn core::any::Any = b;
        any.downcast_ref::<T>()
    }
}

/// The objects of a mempack backend ([`Backend::mempack`],
/// [`BackendRef::as_mempack`]).
#[derive(Clone, Debug)]
pub struct Mempack(Arc<MempackState>);

impl Mempack {
    /// The number of objects (`git_mempack_object_count`).
    pub fn object_count(&self) -> usize {
        self.0.object_count()
    }

    /// Forget every object (`git_mempack_reset`).
    pub fn reset(&self) {
        self.0.reset();
    }

    /// The ids of the objects, in the order `git_mempack_write_thin_pack`
    /// inserts them.
    pub fn ids(&self) -> ApiResult<Vec<Oid>> {
        self.0.ids().map_err(err)
    }

    /// Whether the backend holds `id`.
    pub fn contains(&self, id: &Oid) -> bool {
        self.0.exists(id)
    }

    /// `git_mempack_dump`: a pack (the bytes of a `.pack` file) of the
    /// commits written to the mempack, with every object they reach,
    /// read from `repo`'s odb (which should have the mempack as a
    /// backend).
    pub fn dump(&self, repo: &crate::repository::Repository) -> ApiResult<Vec<u8>> {
        let mut pack = crate::util::str::GitStr::new();
        // SAFETY: `repo` is borrowed for the call, so its `git_repository`
        // stays live (as `dump` requires).
        unsafe { crate::odb_mempack::dump(&mut pack, repo.raw_ptr().cast(), &self.0) }
            .map_err(err)?;
        Ok(pack.as_bytes().to_vec())
    }
}

/// An odb backend implemented in Rust ([`Backend::custom`]).
///
/// Errors with the code [`ErrorCode::NOT_FOUND`] mean "not here"; with
/// [`ErrorCode::PASSTHROUGH`], "ask the other backends". A panic in a
/// method is caught and becomes an error of class
/// [`ErrorClass::INTERNAL`]; a success code or [`ErrorCode::ITER_OVER`]
/// in an error becomes a plain failure. Every method has a default (by
/// default the backend is empty and passes writes on), so that the trait
/// can grow. Its `Drop` may panic: the panic is caught (and its payload
/// leaked), as C frees a backend where it cannot unwind.
///
/// The odb calls a backend with its lock held, from any thread. A backend
/// must not call back into the odb it belongs to: that is detected and
/// fails (C deadlocks). A backend that holds a clone of its own odb keeps
/// it alive forever (a cycle).
pub trait OdbBackend: core::any::Any + Send + Sync + 'static {
    /// The data and type of `id`; by default, not here.
    fn read(&self, id: &Oid) -> ApiResult<(Vec<u8>, ObjectType)> {
        let _ = id;
        Err(Error::with(ErrorCode::NOT_FOUND, ErrorClass::NONE, ""))
    }

    /// Store an object (whose id the odb computed); by default, passed
    /// on to the other backends.
    fn write(&self, id: &Oid, data: &[u8], kind: ObjectType) -> ApiResult<()> {
        let _ = (id, data, kind);
        Err(passthrough())
    }

    /// Whether `id` is here; by default, `false`.
    fn exists(&self, id: &Oid) -> bool {
        let _ = id;
        false
    }

    /// The size and type of `id`; by default, the odb reads the object.
    fn read_header(&self, id: &Oid) -> ApiResult<(usize, ObjectType)> {
        let _ = id;
        Err(passthrough())
    }

    /// The full id, data and type of the object whose id starts with the
    /// first `len` hex digits of `short_id`; by default, not supported.
    fn read_prefix(&self, short_id: &Oid, len: usize) -> ApiResult<(Oid, Vec<u8>, ObjectType)> {
        let _ = (short_id, len);
        Err(passthrough())
    }

    /// The full id of the object whose id starts with the first `len` hex
    /// digits of `short_id`; by default, not supported.
    fn exists_prefix(&self, short_id: &Oid, len: usize) -> ApiResult<Oid> {
        let _ = (short_id, len);
        Err(passthrough())
    }

    /// See objects added behind the backend's back.
    fn refresh(&self) -> ApiResult<()> {
        Ok(())
    }

    /// Call `f` with each id; `f` returns `false` to stop. By default,
    /// lists nothing.
    fn foreach(&self, f: &mut dyn FnMut(&Oid) -> bool) -> ApiResult<()> {
        let _ = f;
        Ok(())
    }

    /// Mark `id` as recently used (it exists); by default, `exists`.
    fn freshen(&self, id: &Oid) -> ApiResult<()> {
        if self.exists(id) {
            Ok(())
        } else {
            Err(Error::with(
                ErrorCode::NOT_FOUND,
                ErrorClass::ODB,
                "object not found",
            ))
        }
    }
}
fn passthrough() -> Error {
    Error::with(ErrorCode::PASSTHROUGH, ErrorClass::NONE, "")
}

/// A Rust backend's state behind its table.
struct RustBackend(crate::api::NoUnwindDrop<Box<dyn OdbBackend>>);

impl Drop for RustBackend {
    fn drop(&mut self) {
        // Before the value goes (the field is dropped after this).
        let p: *const dyn OdbBackend = &raw const **self.0;
        known()
            .retain(|(_, k)| !matches!(k, Known::Rust(RustState(q)) if core::ptr::addr_eq(*q, p)));
    }
}

/// The C status of a Rust backend's error: the message set as C does.
fn raise(e: &Error) -> c_int {
    if e.raw_code() != ErrorCode::PASSTHROUGH && e.raw_code() != ErrorCode::NOT_FOUND
        || !e.message_bytes().is_empty()
    {
        let _ = errors::set_str(e.raw_class(), e.message_bytes());
    }
    e.user_code().raw()
}

/// Copy `data` into `git__allocator` memory for C (with a NUL).
fn to_c_buffer(data: &[u8]) -> Result<*mut c_void> {
    let p = alloc::malloc(data.len().checked_add(1).ok_or_else(errors::oom)?).cast::<u8>();
    if p.is_null() {
        return Err(ErrorCode::GENERIC);
    }
    // SAFETY: `p` is a fresh allocation of `len + 1` bytes (not NULL,
    // checked), so writable for `len` and disjoint from `data`; `u8` needs
    // no alignment.
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len()) };
    // SAFETY: as the line above: byte `len` is the last of the allocation.
    unsafe { p.wrapping_add(data.len()).write(0) };
    Ok(p.cast())
}

ffibox::impl_cvtable! {
    // SAFETY: libgit2 passes each of a backend's slots the backend in the
    // place marked `@`, and calls `free` once; the slots left NULL are
    // optional in C's contract (streams are then the odb's own).
    //
    // Slot arguments (cited as such below): the odb (`odb.rs`: `read_1`,
    // `read_prefix_1`, `read_header`, `exists_1`, `exists_prefix_1`,
    // `write`, `foreach`, `freshen_1`) passes its `&Oid` ids, a slice's
    // pointer and length as `data, len`, and its own locals as
    // out-parameters, all valid and aligned for the call and reached by
    // nothing else meanwhile; a C caller of the slots of
    // `git2/sys/odb_backend.h` must pass the same. The ids are `const`:
    // nothing writes them during the call. An `&Oid` made from one lives
    // only inside the slot.
    unsafe impl CVtable<RustBackend> for BackendTable {}
    base = backend_base();
    fn read(buffer_p: *mut *mut c_void, len_p: *mut usize, type_p: *mut git_object_t, @backend, oid: *const Oid) -> c_int {
        on panic: { errors::set_panic_error(); -1 },
        on busy: busy(),
        |s: &RustBackend| {
            // SAFETY: `oid` is a valid id, unwritten for the call (slot
            // arguments, above).
            match s.0.read(unsafe { &*oid }) {
                Ok((data, kind)) => match to_c_buffer(&data) {
                    Ok(p) => {
                        // SAFETY: `buffer_p` is a valid, aligned `void **`
                        // for the call (slot arguments).
                        unsafe { buffer_p.write(p) };
                        // SAFETY: `len_p` likewise, a `size_t *`.
                        unsafe { len_p.write(data.len()) };
                        // SAFETY: `type_p` likewise, a `git_object_t *`.
                        unsafe { type_p.write(kind.raw()) };
                        0
                    }
                    Err(e) => e.raw(),
                },
                Err(e) => raise(&e),
            }
        },
    }
    fn read_prefix(out: *mut Oid, buffer_p: *mut *mut c_void, len_p: *mut usize, type_p: *mut git_object_t, @backend, short: *const Oid, len: usize) -> c_int {
        on panic: { errors::set_panic_error(); -1 },
        on busy: busy(),
        |s: &RustBackend| {
            // SAFETY: `short` is a valid id, unwritten for the call (slot
            // arguments, above).
            match s.0.read_prefix(unsafe { &*short }, len) {
                Ok((id, data, kind)) => match to_c_buffer(&data) {
                    Ok(p) => {
                        // SAFETY: `out` is a valid, aligned `git_oid *` for
                        // the call (slot arguments).
                        unsafe { out.write(id) };
                        // SAFETY: `buffer_p` likewise, a `void **`.
                        unsafe { buffer_p.write(p) };
                        // SAFETY: `len_p` likewise, a `size_t *`.
                        unsafe { len_p.write(data.len()) };
                        // SAFETY: `type_p` likewise, a `git_object_t *`.
                        unsafe { type_p.write(kind.raw()) };
                        0
                    }
                    Err(e) => e.raw(),
                },
                Err(e) => raise(&e),
            }
        },
    }
    fn read_header(len_p: *mut usize, type_p: *mut git_object_t, @backend, oid: *const Oid) -> c_int {
        on panic: { errors::set_panic_error(); -1 },
        on busy: busy(),
        |s: &RustBackend| {
            // SAFETY: `oid` is a valid id, unwritten for the call (slot
            // arguments, above).
            match s.0.read_header(unsafe { &*oid }) {
                Ok((len, kind)) => {
                    // SAFETY: `len_p` is a valid, aligned `size_t *` for the
                    // call (slot arguments).
                    unsafe { len_p.write(len) };
                    // SAFETY: `type_p` likewise, a `git_object_t *`.
                    unsafe { type_p.write(kind.raw()) };
                    0
                }
                Err(e) => raise(&e),
            }
        },
    }
    fn write(@backend, oid: *const Oid, data: *const c_void, len: usize, type_: git_object_t) -> c_int {
        on panic: { errors::set_panic_error(); -1 },
        on busy: busy(),
        |s: &RustBackend| {
            let Some(kind) = ObjectType::from_raw(type_) else {
                return git_error_set!(ErrorClass::ODB, "invalid object type").raw();
            };
            if data.is_null() && len != 0 {
                return errors::assert_arg_failed("data").raw();
            }
            // SAFETY: `oid` is a valid id, unwritten for the call (slot
            // arguments, above).
            let oid = unsafe { &*oid };
            // SAFETY: `data` is readable for `len` initialized bytes that
            // nothing writes during the call (slot arguments: the odb
            // passes a slice); NULL with 0 is the empty slice (NULL with
            // more is refused above), and `bytes` panics (into `on panic`)
            // for `len > isize::MAX`.
            let data = unsafe { crate::util::export::bytes(data.cast(), len) };
            match s.0.write(oid, data, kind) {
                Ok(()) => 0,
                Err(e) => raise(&e),
            }
        },
    }
    fn exists(@backend, oid: *const Oid) -> c_int {
        on panic: 0,
        on busy: 0,
        // SAFETY: `oid` is a valid id, unwritten for the call (slot
        // arguments, above).
        |s: &RustBackend| c_int::from(s.0.exists(unsafe { &*oid })),
    }
    fn exists_prefix(out: *mut Oid, @backend, short: *const Oid, len: usize) -> c_int {
        on panic: { errors::set_panic_error(); -1 },
        on busy: busy(),
        |s: &RustBackend| {
            // SAFETY: `short` is a valid id, unwritten for the call (slot
            // arguments, above).
            match s.0.exists_prefix(unsafe { &*short }, len) {
                Ok(id) => {
                    // SAFETY: `out` is NULL (skipped) or a valid, aligned
                    // `git_oid *` for the call (slot arguments).
                    unsafe { crate::util::export::write_opt(out, id) };
                    0
                }
                Err(e) => raise(&e),
            }
        },
    }
    fn refresh(@backend) -> c_int {
        on panic: { errors::set_panic_error(); -1 },
        on busy: busy(),
        |s: &RustBackend| match s.0.refresh() {
            Ok(()) => 0,
            Err(e) => raise(&e),
        },
    }
    fn foreach(@backend, cb: ForeachCb, payload: *mut c_void) -> c_int {
        on panic: { errors::set_panic_error(); -1 },
        on busy: busy(),
        |s: &RustBackend| {
            let Some(cb) = cb else {
                return errors::assert_arg_failed("cb").raw();
            };
            let mut stop = 0;
            let r = s.0.foreach(&mut |id| {
                // A backend that ignores `false` gets `false` again, and C's
                // callback is not called after it asked to stop (S-6b).
                if stop != 0 {
                    return false;
                }
                // SAFETY: `cb` and `payload` are a `git_odb_foreach_cb` and
                // its payload, which the caller of `foreach` (the odb's
                // `foreach_raw`, or C) lets us call during the call; `id` is
                // the backend's `&Oid`, valid during the callback.
                stop = errors::set_after_callback(unsafe { cb(id, payload) }, "git_odb_foreach");
                stop == 0
            });
            if stop != 0 {
                return stop;
            }
            match r {
                Ok(()) => 0,
                Err(e) => raise(&e),
            }
        },
    }
    fn freshen(@backend, oid: *const Oid) -> c_int {
        on panic: { errors::set_panic_error(); -1 },
        on busy: busy(),
        // SAFETY: `oid` is a valid id, unwritten for the call (slot
        // arguments, above).
        |s: &RustBackend| match s.0.freshen(unsafe { &*oid }) {
            Ok(()) => 0,
            Err(e) => raise(&e),
        },
    }
    release free;
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Whether objects read are checked against their ids
/// (`GIT_OPT_ENABLE_STRICT_HASH_VERIFICATION`).
pub fn set_strict_hash_verification(on: bool) {
    git_odb__strict_hash_verification.store(on, Ordering::Relaxed);
}

/// The priorities of the loose and pack backends of directories opened
/// later (`GIT_OPT_SET_ODB_LOOSE_PRIORITY`, `…_PACKED_PRIORITY`).
pub fn set_default_priorities(loose: i32, packed: i32) {
    git_odb__loose_priority.store(loose, Ordering::Relaxed);
    git_odb__packed_priority.store(packed, Ordering::Relaxed);
}
