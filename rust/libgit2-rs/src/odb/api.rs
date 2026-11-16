//! The safe Rust API of the object database: [`Odb`], [`OdbObject`],
//! [`ObjectType`], backends ([`Backend`], the [`OdbBackend`] trait,
//! [`Mempack`]) and streams ([`OdbWriter`]: `Write`, [`OdbReader`]:
//! `Read`).
//!
//! ```
//! # #[cfg(not(miri))] {
//! use libgit2_rs::{Backend, ObjectType, Odb};
//! let _g = libgit2_rs::util::test_support::init();
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
use std::sync::Arc;

use super::*;
use crate::odb_mempack::MempackState;
use crate::util::errors::Error;

type ApiResult<T> = core::result::Result<T, Error>;

fn err(code: ErrorCode) -> Error {
    Error::from(code)
}

/// `path` as a C string; a NUL inside it is refused (C would see a
/// shorter path).
fn c_path(path: &[u8]) -> ApiResult<std::ffi::CString> {
    std::ffi::CString::new(path).map_err(|_| {
        Error::new(
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
/// from several threads at once.
pub struct Odb<'r> {
    raw: NonNull<GitOdb>,
    /// The repository the odb came from (`'static` for one made alone):
    /// an odb of a repository uses its owner's object cache, so it cannot
    /// outlive it.
    _parent: PhantomData<&'r ()>,
}

// SAFETY: `GitOdb` is `Sync` (its backends are reached under a mutex, its
// caches under rwlocks, the rest is atomic or immutable), and its count is
// atomic: a reference may move to, and be dropped on, any thread. The
// repository an odb borrows (`'r`), whose object cache it uses, is `Sync`
// too.
unsafe impl Send for Odb<'_> {}
// SAFETY: as above.
unsafe impl Sync for Odb<'_> {}

impl core::fmt::Debug for Odb<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Odb")
            .field("oid_type", &self.oid_type())
            .field("backends", &self.num_backends())
            .finish()
    }
}

impl<'r> Clone for Odb<'r> {
    fn clone(&self) -> Odb<'r> {
        self.inner().rc.inc();
        Odb {
            raw: self.raw,
            _parent: PhantomData,
        }
    }
}

impl Drop for Odb<'_> {
    fn drop(&mut self) {
        // SAFETY: this handle's reference.
        unsafe { odb_decref(self.raw) }
    }
}

impl<'r> Odb<'r> {
    /// The odb (crate-internal: the repository calls it directly).
    pub(crate) fn inner(&self) -> &GitOdb {
        // SAFETY: the handle holds a reference: the odb is live.
        unsafe { self.raw.as_ref() }
    }

    /// Adopt a reference to `raw` that the caller gives up.
    ///
    /// # Safety
    ///
    /// `raw` is a live odb and the caller owns one of its references.
    pub(crate) unsafe fn from_raw(raw: NonNull<GitOdb>) -> Odb<'r> {
        Odb {
            raw,
            _parent: PhantomData,
        }
    }

    /// A new reference to `raw` (`GIT_REFCOUNT_INC`).
    ///
    /// # Safety
    ///
    /// `raw` is a live odb.
    pub(crate) unsafe fn new_ref(raw: NonNull<GitOdb>) -> Odb<'r> {
        // SAFETY: the caller's contract.
        unsafe { raw.as_ref() }.rc.inc();
        Odb {
            raw,
            _parent: PhantomData,
        }
    }

    /// The `git_odb *` (the reference stays with this handle).
    pub(crate) fn as_raw(&self) -> NonNull<GitOdb> {
        self.raw
    }

    /// Give this handle's reference up, as a `git_odb *`.
    pub(crate) fn into_raw(self) -> NonNull<GitOdb> {
        let raw = self.raw;
        core::mem::forget(self);
        raw
    }

    /// An odb without backends, for SHA-1 ids (`git_odb_new`).
    pub fn new() -> ApiResult<Odb<'static>> {
        Self::with_oid_type(OidType::Sha1)
    }

    /// An odb without backends for ids of `oid_type` (`git_odb_new_ext`).
    pub fn with_oid_type(oid_type: OidType) -> ApiResult<Odb<'static>> {
        let opts = GitOdbOptions {
            version: GIT_ODB_OPTIONS_VERSION,
            oid_type: oid_type.raw(),
        };
        let raw = GitOdb::new(Some(&opts)).map_err(err)?;
        Ok(Odb {
            raw,
            _parent: PhantomData,
        })
    }

    /// The odb of an `objects` directory, with its loose and pack backends
    /// and its alternates (`git_odb_open_ext`).
    pub fn open(objects_dir: &[u8], oid_type: OidType) -> ApiResult<Odb<'static>> {
        let dir = c_path(objects_dir)?;
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
    pub fn add_disk_alternate(&self, path: &[u8]) -> ApiResult<()> {
        let dir = c_path(path)?;
        self.inner()
            .add_default_backends(&dir, true, 0)
            .map_err(err)
    }

    /// The number of backends (`git_odb_num_backends`).
    pub fn num_backends(&self) -> usize {
        self.inner().num_backends()
    }

    /// Read an object (`git_odb_read`).
    pub fn read(&self, id: &Oid) -> ApiResult<OdbObject> {
        self.inner().read(id).map(OdbObject).map_err(err)
    }

    /// Read the object whose id starts with the first `len` hex digits of
    /// `short_id` (`git_odb_read_prefix`).
    pub fn read_prefix(&self, short_id: &Oid, len: usize) -> ApiResult<OdbObject> {
        self.inner()
            .read_prefix(short_id, len)
            .map(OdbObject)
            .map_err(err)
    }

    /// An object's size and type, without reading it if a backend can tell
    /// (`git_odb_read_header`).
    pub fn read_header(&self, id: &Oid) -> ApiResult<(usize, Option<ObjectType>)> {
        let (len, t) = self.inner().read_header(id).map_err(err)?;
        Ok((len, ObjectType::from_raw(t)))
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
            // SAFETY: the payload below, alive for the walk; the id is
            // the backend's, readable for the call.
            let p = unsafe { &mut *payload.cast::<Payload<'_>>() };
            // SAFETY: as above.
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
        // SAFETY: the trampoline matches the payload, which outlives the
        // call.
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
    pub fn reader(&self, id: &Oid) -> ApiResult<(OdbReader<'_>, usize, Option<ObjectType>)> {
        let Some((stream, len, t)) = self.inner().open_rstream(id).map_err(err)? else {
            return Err(Error::new(
                ErrorCode::GENERIC,
                ErrorClass::ODB,
                "backend returned no stream",
            ));
        };
        Ok((
            OdbReader {
                stream,
                _odb: PhantomData,
            },
            len,
            ObjectType::from_raw(t),
        ))
    }

    /// The id `data` would have as an object of `kind` (`git_odb_hash`).
    pub fn hash(data: &[u8], kind: ObjectType, oid_type: OidType) -> ApiResult<Oid> {
        hash_object(Some(data), data.len(), kind.raw(), oid_type.raw()).map_err(err)
    }
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

// SAFETY: an odb object is immutable once read, but for its atomic count;
// the last reference frees it on whichever thread drops it.
unsafe impl Send for OdbObject {}
// SAFETY: as above.
unsafe impl Sync for OdbObject {}

impl OdbObject {
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
        // SAFETY: the buffer holds `size` bytes (the backends' contract),
        // never written once the object is made, and lives as long as the
        // object, which `self` keeps.
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

    /// The type (`git_odb_object_type`); `None` for a type a backend made
    /// up (with hash verification off).
    pub fn kind(&self) -> Option<ObjectType> {
        ObjectType::from_raw(git_object_t::from(self.obj().cached.type_))
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
        // SAFETY: this handle's reference.
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
        // SAFETY: our live stream, whose backend belongs to the borrowed
        // odb; `id` a local.
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
        // SAFETY: our live stream; `buf` readable for its length.
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
        // SAFETY: our stream, freed once.
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
        // SAFETY: our live stream; `buf` writable for its length.
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
        // SAFETY: our stream, freed once.
        unsafe { stream_free(self.stream.as_ptr()) }
    }
}

impl core::fmt::Debug for OdbReader<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OdbReader").finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Backends
// ---------------------------------------------------------------------------

/// A backend not yet added to an odb (`git_odb_backend *`); dropped, it is
/// freed. [`Odb::add_backend`] hands it to the odb.
pub struct Backend(NonNull<GitOdbBackend>);

// SAFETY: every backend this API makes (loose, pack, mempack, Rust) is
// usable from any thread under the odb's lock, as libgit2 requires of
// backends.
unsafe impl Send for Backend {}

impl core::fmt::Debug for Backend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Backend").finish_non_exhaustive()
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        // SAFETY: ours, not added to an odb: freed once.
        unsafe { free_backend(self.0) }
    }
}

/// Options of the loose backend (`git_odb_backend_loose_options`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LooseOptions {
    /// zlib's level (0–9); `None` for the default (fastest).
    pub compression_level: Option<u32>,
    /// `fsync` each object written.
    pub fsync: bool,
    /// Mode of the directories made (`None`: 0777, less the umask).
    pub dir_mode: Option<u32>,
    /// Mode of the files written (`None`: 0444).
    pub file_mode: Option<u32>,
    /// The type of the ids.
    pub oid_type: OidType,
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

impl Backend {
    /// The loose backend of `objects_dir` (`git_odb_backend_loose`).
    pub fn loose(objects_dir: &[u8], opts: &LooseOptions) -> ApiResult<Backend> {
        c_path(objects_dir)?;
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
        crate::odb_loose::new_backend(objects_dir, Some(&o))
            .map(Backend)
            .map_err(err)
    }

    /// The pack backend of `objects_dir` (`git_odb_backend_pack`).
    pub fn pack(objects_dir: &[u8], oid_type: OidType) -> ApiResult<Backend> {
        let dir = c_path(objects_dir)?;
        let opts = GitOdbBackendPackOptions {
            version: 1,
            oid_type: oid_type.raw(),
        };
        crate::odb_pack::backend_pack_raw(&dir, &opts)
            .map(Backend)
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
        Backend(
            ffibox::CTable::<BackendTable, RustBackend>::new(RustBackend(Box::new(backend)))
                .into_c(),
        )
    }
}

/// The objects of a mempack backend ([`Backend::mempack`]).
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
}

/// An odb backend implemented in Rust ([`Backend::custom`]). Errors with
/// the code [`ErrorCode::NOT_FOUND`] mean "not here"; with
/// [`ErrorCode::PASSTHROUGH`], "ask the other backends". The odb calls a
/// backend with its lock held, from any thread: a backend must not call
/// back into the odb it belongs to (that deadlocks, as in C).
pub trait OdbBackend: Send + Sync + 'static {
    /// The data and type of `id`.
    fn read(&self, id: &Oid) -> ApiResult<(Vec<u8>, ObjectType)>;

    /// Store an object (whose id the odb computed).
    fn write(&self, id: &Oid, data: &[u8], kind: ObjectType) -> ApiResult<()>;

    /// Whether `id` is here.
    fn exists(&self, id: &Oid) -> bool;

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
            Err(Error::new(
                ErrorCode::NOT_FOUND,
                ErrorClass::ODB,
                "object not found",
            ))
        }
    }
}

fn passthrough() -> Error {
    Error::new(ErrorCode::PASSTHROUGH, ErrorClass::NONE, "")
}

/// A Rust backend's state behind its table.
struct RustBackend(Box<dyn OdbBackend>);

/// The C status of a Rust backend's error: the message set as C does.
fn raise(e: &Error) -> c_int {
    if e.code() != ErrorCode::PASSTHROUGH && e.code() != ErrorCode::NOT_FOUND
        || !e.message_bytes().is_empty()
    {
        let _ = errors::set_str(e.class(), e.message_bytes());
    }
    e.code().raw()
}

/// Copy `data` into `git__allocator` memory for C (with a NUL).
fn to_c_buffer(data: &[u8]) -> Result<*mut c_void> {
    let p = alloc::malloc(data.len().checked_add(1).ok_or_else(errors::oom)?).cast::<u8>();
    if p.is_null() {
        return Err(ErrorCode::GENERIC);
    }
    // SAFETY: a fresh allocation of `len + 1` bytes.
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len()) };
    // SAFETY: as above.
    unsafe { p.wrapping_add(data.len()).write(0) };
    Ok(p.cast())
}

ffibox::impl_cvtable! {
    // SAFETY: libgit2 passes each of a backend's slots the backend in the
    // place marked `@`, and calls `free` once; the slots left NULL are
    // optional in C's contract (streams are then the odb's own).
    unsafe impl CVtable<RustBackend> for BackendTable {}
    base = backend_base();
    fn read(buffer_p: *mut *mut c_void, len_p: *mut usize, type_p: *mut git_object_t, @backend, oid: *const Oid) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &RustBackend| {
            // SAFETY: C's contract: a readable id.
            match s.0.read(unsafe { &*oid }) {
                Ok((data, kind)) => match to_c_buffer(&data) {
                    Ok(p) => {
                        // SAFETY: C's contract: writable out-parameters.
                        unsafe { buffer_p.write(p) };
                        // SAFETY: as above.
                        unsafe { len_p.write(data.len()) };
                        // SAFETY: as above.
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
        on panic: -1,
        on busy: busy(),
        |s: &RustBackend| {
            // SAFETY: C's contract: a readable id.
            match s.0.read_prefix(unsafe { &*short }, len) {
                Ok((id, data, kind)) => match to_c_buffer(&data) {
                    Ok(p) => {
                        // SAFETY: C's contract: writable out-parameters.
                        unsafe { out.write(id) };
                        // SAFETY: as above.
                        unsafe { buffer_p.write(p) };
                        // SAFETY: as above.
                        unsafe { len_p.write(data.len()) };
                        // SAFETY: as above.
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
        on panic: -1,
        on busy: busy(),
        |s: &RustBackend| {
            // SAFETY: C's contract: a readable id.
            match s.0.read_header(unsafe { &*oid }) {
                Ok((len, kind)) => {
                    // SAFETY: C's contract: writable out-parameters.
                    unsafe { len_p.write(len) };
                    // SAFETY: as above.
                    unsafe { type_p.write(kind.raw()) };
                    0
                }
                Err(e) => raise(&e),
            }
        },
    }
    fn write(@backend, oid: *const Oid, data: *const c_void, len: usize, type_: git_object_t) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &RustBackend| {
            let Some(kind) = ObjectType::from_raw(type_) else {
                return git_error_set!(ErrorClass::ODB, "invalid object type").raw();
            };
            if data.is_null() && len != 0 {
                return errors::assert_arg_failed("data").raw();
            }
            // SAFETY: C's contract: a readable id.
            let oid = unsafe { &*oid };
            // SAFETY: C's contract: `len` readable bytes.
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
        // SAFETY: C's contract: a readable id.
        |s: &RustBackend| c_int::from(s.0.exists(unsafe { &*oid })),
    }
    fn exists_prefix(out: *mut Oid, @backend, short: *const Oid, len: usize) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &RustBackend| {
            // SAFETY: C's contract: a readable id.
            match s.0.exists_prefix(unsafe { &*short }, len) {
                Ok(id) => {
                    // SAFETY: C's contract: writable.
                    unsafe { crate::util::export::write_opt(out, id) };
                    0
                }
                Err(e) => raise(&e),
            }
        },
    }
    fn refresh(@backend) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &RustBackend| match s.0.refresh() {
            Ok(()) => 0,
            Err(e) => raise(&e),
        },
    }
    fn foreach(@backend, cb: ForeachCb, payload: *mut c_void) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &RustBackend| {
            let Some(cb) = cb else {
                return errors::assert_arg_failed("cb").raw();
            };
            let mut stop = 0;
            let r = s.0.foreach(&mut |id| {
                // SAFETY: C's callback, with its payload.
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
        on panic: -1,
        on busy: busy(),
        // SAFETY: C's contract: a readable id.
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
