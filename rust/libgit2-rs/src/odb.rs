//! Port of `src/libgit2/odb.c`: the object database (`git_odb`), its
//! objects (`git_odb_object`) and its backends' C interface.
//!
//! # Types
//!
//! - [`GitOdb`] is `git_odb`, a shared prefix (kind 2): C that stays reads
//!   `odb->options.oid_type` (repository.c) and the refcount macros touch
//!   `odb->rc`; the rest (the backends and their mutex, the commit-graph,
//!   the odb's own cache, `do_fsync`) is Rust state after the prefix. It is
//!   allocated by [`GitOdb::new`] (`git__allocator`) and freed here only.
//!   It is `Sync`: the backends are reached under a `Mutex`, as C's
//!   `git_mutex`, the caches have their rwlocks, the rest is atomic or
//!   immutable after creation.
//! - [`GitOdbObject`] is `git_odb_object`, mirrored (kind 3): the tests
//!   build one on the stack, object.c reads `cached.*`, commit.c and others
//!   read `buffer` through `git_odb_object_data`.
//! - [`GitOdbBackend`], [`GitOdbStream`], [`GitOdbWritepack`],
//!   [`GitOdbOptions`], [`GitOdbExpandId`], and the backends' option
//!   structs are mirrored public types (`include/git2/**`).
//!
//! Backends are C tables. Those written in C (the tests', the pack backend
//! of `odb_pack.c`) are called through their slots, with the odb's mutex
//! held, as C did; the loose and mempack backends and the streams this
//! crate makes are `ffibox::CTable`s whose slots C calls, so the odb calls
//! them the same way.

use core::ffi::{CStr, c_char, c_int, c_uint, c_ushort, c_void};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::cache::{self, GIT_CACHE_STORE_RAW, GitCache, GitCachedObj};
use crate::oid::{GIT_OID_MINPREFIXLEN, GIT_OID_SHA1, Oid, OidType, git_oid_t, oid_hexsize};
use crate::sys::commit_graph::CommitGraph;
use crate::util::alloc::{self, GIT_ALLOC};
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::hash::{self, HashCtx};
use crate::util::thread::{Atomic32, GitRefcount};
use crate::{git_assert, git_assert_arg, git_error_set};

pub use crate::util::errors::Error;

/// `git_object_t`.
#[allow(non_camel_case_types)]
pub type git_object_t = c_int;
/// `git_object_size_t`.
#[allow(non_camel_case_types)]
pub type git_object_size_t = u64;

/// `GIT_OBJECT_ANY`.
pub const GIT_OBJECT_ANY: git_object_t = -2;
/// `GIT_OBJECT_INVALID`.
pub const GIT_OBJECT_INVALID: git_object_t = -1;
/// `GIT_OBJECT_COMMIT`.
pub const GIT_OBJECT_COMMIT: git_object_t = 1;
/// `GIT_OBJECT_TREE`.
pub const GIT_OBJECT_TREE: git_object_t = 2;
/// `GIT_OBJECT_BLOB`.
pub const GIT_OBJECT_BLOB: git_object_t = 3;
/// `GIT_OBJECT_TAG`.
pub const GIT_OBJECT_TAG: git_object_t = 4;
/// `GIT_OBJECT_OFS_DELTA`.
pub const GIT_OBJECT_OFS_DELTA: git_object_t = 6;
/// `GIT_OBJECT_REF_DELTA`.
pub const GIT_OBJECT_REF_DELTA: git_object_t = 7;

/// `GIT_OBJECT_DIR_MODE`.
pub const GIT_OBJECT_DIR_MODE: c_uint = 0o777;
/// `GIT_OBJECT_FILE_MODE`.
pub const GIT_OBJECT_FILE_MODE: c_uint = 0o444;
/// `GIT_ODB_DEFAULT_LOOSE_PRIORITY`.
pub const GIT_ODB_DEFAULT_LOOSE_PRIORITY: c_int = 1;
/// `GIT_ODB_DEFAULT_PACKED_PRIORITY`.
pub const GIT_ODB_DEFAULT_PACKED_PRIORITY: c_int = 2;
/// `GIT_ODB_BACKEND_VERSION`.
pub const GIT_ODB_BACKEND_VERSION: c_uint = 1;
/// `GIT_ODB_OPTIONS_VERSION`.
pub const GIT_ODB_OPTIONS_VERSION: c_uint = 1;
/// `GIT_ODB_LOOKUP_NO_REFRESH`.
pub const GIT_ODB_LOOKUP_NO_REFRESH: c_uint = 1 << 0;
/// `GIT_STREAM_RDONLY`.
pub const GIT_STREAM_RDONLY: c_uint = 1 << 1;
/// `GIT_STREAM_WRONLY`.
pub const GIT_STREAM_WRONLY: c_uint = 1 << 2;
/// `GIT_ODB_CAP_FROM_OWNER`.
pub const GIT_ODB_CAP_FROM_OWNER: c_int = -1;
/// `GIT_ALTERNATES_MAX_DEPTH`.
const GIT_ALTERNATES_MAX_DEPTH: c_int = 5;

// ---------------------------------------------------------------------------
// Object types
// ---------------------------------------------------------------------------

/// An object's type (`git_object_t`), for the safe API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectType {
    /// `GIT_OBJECT_ANY`: any type (a lookup's wildcard).
    Any,
    /// `GIT_OBJECT_COMMIT`.
    Commit,
    /// `GIT_OBJECT_TREE`.
    Tree,
    /// `GIT_OBJECT_BLOB`.
    Blob,
    /// `GIT_OBJECT_TAG`.
    Tag,
    /// `GIT_OBJECT_OFS_DELTA`.
    OfsDelta,
    /// `GIT_OBJECT_REF_DELTA`.
    RefDelta,
}

impl ObjectType {
    /// From C's value; `None` for `GIT_OBJECT_INVALID` and unknown values.
    pub const fn from_raw(raw: git_object_t) -> Option<ObjectType> {
        Some(match raw {
            GIT_OBJECT_ANY => ObjectType::Any,
            GIT_OBJECT_COMMIT => ObjectType::Commit,
            GIT_OBJECT_TREE => ObjectType::Tree,
            GIT_OBJECT_BLOB => ObjectType::Blob,
            GIT_OBJECT_TAG => ObjectType::Tag,
            GIT_OBJECT_OFS_DELTA => ObjectType::OfsDelta,
            GIT_OBJECT_REF_DELTA => ObjectType::RefDelta,
            _ => return None,
        })
    }

    /// C's value.
    pub const fn raw(self) -> git_object_t {
        match self {
            ObjectType::Any => GIT_OBJECT_ANY,
            ObjectType::Commit => GIT_OBJECT_COMMIT,
            ObjectType::Tree => GIT_OBJECT_TREE,
            ObjectType::Blob => GIT_OBJECT_BLOB,
            ObjectType::Tag => GIT_OBJECT_TAG,
            ObjectType::OfsDelta => GIT_OBJECT_OFS_DELTA,
            ObjectType::RefDelta => GIT_OBJECT_REF_DELTA,
        }
    }

    /// `git_object_type2string`: the loose header name (`""` for the types
    /// that have none).
    pub fn name(self) -> &'static str {
        // The table holds ASCII names.
        core::str::from_utf8(type2string(self.raw())).unwrap_or("")
    }

    /// `git_object_stringn2type`: the first type whose name prefixes `s`
    /// (C's `git__prefixncmp`, so `"commitx"` is a commit).
    pub fn from_name(s: &[u8]) -> Option<ObjectType> {
        Self::from_raw(stringn2type(s))
    }

    /// `git_object_type_is_valid`: commit, tree, blob or tag.
    pub const fn is_loose(self) -> bool {
        type_is_valid(self.raw())
    }
}

impl core::fmt::Display for ObjectType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// `git_object_type2string` on C's value (the object module's table).
pub fn type2string(t: git_object_t) -> &'static [u8] {
    crate::object::type2string(t).to_bytes()
}

/// `git_object_stringn2type` on bytes (the object module's; C stops at a
/// NUL).
pub fn stringn2type(s: &[u8]) -> git_object_t {
    crate::object::stringn2type(s)
}

/// `git_object_type_is_valid` on C's value.
pub const fn type_is_valid(t: git_object_t) -> bool {
    matches!(
        t,
        GIT_OBJECT_COMMIT | GIT_OBJECT_TREE | GIT_OBJECT_BLOB | GIT_OBJECT_TAG
    )
}

// ---------------------------------------------------------------------------
// Public C types
// ---------------------------------------------------------------------------

/// `git_odb_options`, mirrored.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GitOdbOptions {
    /// `version`
    pub version: c_uint,
    /// `oid_type` (0: the default)
    pub oid_type: git_oid_t,
}

impl Default for GitOdbOptions {
    /// `GIT_ODB_OPTIONS_INIT`.
    fn default() -> Self {
        GitOdbOptions {
            version: GIT_ODB_OPTIONS_VERSION,
            oid_type: 0,
        }
    }
}

/// `git_odb_backend_pack_options`, mirrored.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GitOdbBackendPackOptions {
    /// `version`
    pub version: c_uint,
    /// `oid_type`
    pub oid_type: git_oid_t,
}

/// `git_odb_backend_loose_options`, mirrored.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GitOdbBackendLooseOptions {
    /// `version`
    pub version: c_uint,
    /// `GIT_ODB_BACKEND_LOOSE_*` flags
    pub flags: u32,
    /// zlib level, -1 for the default (`Z_BEST_SPEED`)
    pub compression_level: c_int,
    /// directory mode, 0 for `GIT_OBJECT_DIR_MODE`
    pub dir_mode: c_uint,
    /// file mode, 0 for `GIT_OBJECT_FILE_MODE`
    pub file_mode: c_uint,
    /// `oid_type`
    pub oid_type: git_oid_t,
}

impl Default for GitOdbBackendLooseOptions {
    /// `GIT_ODB_BACKEND_LOOSE_OPTIONS_INIT`.
    fn default() -> Self {
        GitOdbBackendLooseOptions {
            version: 1,
            flags: 0,
            compression_level: -1,
            dir_mode: 0,
            file_mode: 0,
            oid_type: 0,
        }
    }
}

/// `GIT_ODB_BACKEND_LOOSE_FSYNC`.
pub const GIT_ODB_BACKEND_LOOSE_FSYNC: u32 = 1 << 0;

/// `git_odb_foreach_cb`.
pub type ForeachCb = Option<unsafe extern "C" fn(*const Oid, *mut c_void) -> c_int>;
/// `git_indexer_progress_cb`.
pub type IndexerProgressCb = Option<unsafe extern "C" fn(*const c_void, *mut c_void) -> c_int>;

/// `git_odb_backend`, mirrored: a C table each slot of which is passed
/// the table.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
#[allow(clippy::type_complexity)]
pub struct GitOdbBackend {
    /// `version`
    pub version: c_uint,
    /// The owning odb (set when added).
    pub odb: *mut GitOdb,
    /// `read`
    pub read: Option<
        unsafe extern "C" fn(
            *mut *mut c_void,
            *mut usize,
            *mut git_object_t,
            *mut GitOdbBackend,
            *const Oid,
        ) -> c_int,
    >,
    /// `read_prefix`
    pub read_prefix: Option<
        unsafe extern "C" fn(
            *mut Oid,
            *mut *mut c_void,
            *mut usize,
            *mut git_object_t,
            *mut GitOdbBackend,
            *const Oid,
            usize,
        ) -> c_int,
    >,
    /// `read_header`
    pub read_header: Option<
        unsafe extern "C" fn(
            *mut usize,
            *mut git_object_t,
            *mut GitOdbBackend,
            *const Oid,
        ) -> c_int,
    >,
    /// `write`
    pub write: Option<
        unsafe extern "C" fn(
            *mut GitOdbBackend,
            *const Oid,
            *const c_void,
            usize,
            git_object_t,
        ) -> c_int,
    >,
    /// `writestream`
    pub writestream: Option<
        unsafe extern "C" fn(
            *mut *mut GitOdbStream,
            *mut GitOdbBackend,
            git_object_size_t,
            git_object_t,
        ) -> c_int,
    >,
    /// `readstream`
    pub readstream: Option<
        unsafe extern "C" fn(
            *mut *mut GitOdbStream,
            *mut usize,
            *mut git_object_t,
            *mut GitOdbBackend,
            *const Oid,
        ) -> c_int,
    >,
    /// `exists`
    pub exists: Option<unsafe extern "C" fn(*mut GitOdbBackend, *const Oid) -> c_int>,
    /// `exists_prefix`
    pub exists_prefix:
        Option<unsafe extern "C" fn(*mut Oid, *mut GitOdbBackend, *const Oid, usize) -> c_int>,
    /// `refresh`
    pub refresh: Option<unsafe extern "C" fn(*mut GitOdbBackend) -> c_int>,
    /// `foreach`
    pub foreach: Option<unsafe extern "C" fn(*mut GitOdbBackend, ForeachCb, *mut c_void) -> c_int>,
    /// `writepack`
    pub writepack: Option<
        unsafe extern "C" fn(
            *mut *mut GitOdbWritepack,
            *mut GitOdbBackend,
            *mut GitOdb,
            IndexerProgressCb,
            *mut c_void,
        ) -> c_int,
    >,
    /// `writemidx`
    pub writemidx: Option<unsafe extern "C" fn(*mut GitOdbBackend) -> c_int>,
    /// `freshen`
    pub freshen: Option<unsafe extern "C" fn(*mut GitOdbBackend, *const Oid) -> c_int>,
    /// `free`
    pub free: Option<unsafe extern "C" fn(*mut GitOdbBackend)>,
}

impl Default for GitOdbBackend {
    /// All zero (`git_odb_init_backend` then sets the version).
    fn default() -> Self {
        GitOdbBackend {
            version: 0,
            odb: core::ptr::null_mut(),
            read: None,
            read_prefix: None,
            read_header: None,
            write: None,
            writestream: None,
            readstream: None,
            exists: None,
            exists_prefix: None,
            refresh: None,
            foreach: None,
            writepack: None,
            writemidx: None,
            freshen: None,
            free: None,
        }
    }
}

/// `git_odb_stream`, mirrored.
#[repr(C)]
#[derive(Debug)]
#[allow(clippy::type_complexity)]
pub struct GitOdbStream {
    /// The backend that made it.
    pub backend: *mut GitOdbBackend,
    /// `GIT_STREAM_*`
    pub mode: c_uint,
    /// The `git_hash_ctx *` the odb hashes the written data with.
    pub hash_ctx: *mut c_void,
    /// The odb's oid type.
    pub oid_type: git_oid_t,
    /// The size given to `git_odb_open_wstream`.
    pub declared_size: git_object_size_t,
    /// The bytes written so far.
    pub received_bytes: git_object_size_t,
    /// `read`
    pub read: Option<unsafe extern "C" fn(*mut GitOdbStream, *mut c_char, usize) -> c_int>,
    /// `write`
    pub write: Option<unsafe extern "C" fn(*mut GitOdbStream, *const c_char, usize) -> c_int>,
    /// `finalize_write`
    pub finalize_write: Option<unsafe extern "C" fn(*mut GitOdbStream, *const Oid) -> c_int>,
    /// `free`
    pub free: Option<unsafe extern "C" fn(*mut GitOdbStream)>,
}

impl Default for GitOdbStream {
    fn default() -> Self {
        GitOdbStream {
            backend: core::ptr::null_mut(),
            mode: 0,
            hash_ctx: core::ptr::null_mut(),
            oid_type: 0,
            declared_size: 0,
            received_bytes: 0,
            read: None,
            write: None,
            finalize_write: None,
            free: None,
        }
    }
}

/// `git_odb_writepack`, mirrored.
#[repr(C)]
#[derive(Debug)]
#[allow(clippy::type_complexity)]
pub struct GitOdbWritepack {
    /// The backend that made it.
    pub backend: *mut GitOdbBackend,
    /// `append`
    pub append: Option<
        unsafe extern "C" fn(*mut GitOdbWritepack, *const c_void, usize, *mut c_void) -> c_int,
    >,
    /// `commit`
    pub commit: Option<unsafe extern "C" fn(*mut GitOdbWritepack, *mut c_void) -> c_int>,
    /// `free`
    pub free: Option<unsafe extern "C" fn(*mut GitOdbWritepack)>,
}

/// `git_odb_expand_id`, mirrored.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GitOdbExpandId {
    /// The (short) id.
    pub id: Oid,
    /// Its length in hex digits.
    pub length: c_ushort,
    /// The type wanted, then found.
    pub type_: git_object_t,
}

ffibox::define_ctype!(
    BackendTable,
    BackendTableRef,
    BackendTableMut,
    GitOdbBackend
);
ffibox::define_ctype!(StreamTable, StreamTableRef, StreamTableMut, GitOdbStream);

/// The base table of a Rust backend: the version set, every slot NULL.
pub(crate) fn backend_base() -> GitOdbBackend {
    GitOdbBackend {
        version: GIT_ODB_BACKEND_VERSION,
        ..GitOdbBackend::default()
    }
}

/// The result of a slot that cannot enter its table (a release deferred).
pub(crate) fn busy() -> c_int {
    errors::set_str(
        ErrorClass::INTERNAL,
        "the object database backend is being freed",
    )
    .raw()
}

// ---------------------------------------------------------------------------
// Objects
// ---------------------------------------------------------------------------

/// `git_odb_object`, mirrored.
#[repr(C)]
#[derive(Debug)]
pub struct GitOdbObject {
    /// The cache header.
    pub cached: GitCachedObj,
    /// The data (`cached.size` bytes and a NUL, from `git__allocator`).
    pub buffer: *mut c_void,
}

/// `git_rawobj`: what a backend read.
#[derive(Debug)]
pub(crate) struct RawObj {
    pub data: *mut c_void,
    pub len: usize,
    pub type_: git_object_t,
}

impl RawObj {
    fn empty() -> RawObj {
        RawObj {
            data: core::ptr::null_mut(),
            len: 0,
            type_: GIT_OBJECT_INVALID,
        }
    }

    /// Free the data (`git__free(raw.data)`).
    fn free(&mut self) {
        // SAFETY: the data a backend returned is `git__allocator` memory
        // (`git_odb_backend_data_alloc`), owned here, or NULL.
        unsafe { alloc::free(self.data) };
        self.data = core::ptr::null_mut();
    }

    /// The data as bytes.
    ///
    /// # Safety
    ///
    /// `data` is readable for `len` bytes (a backend's contract), or `len`
    /// is 0.
    unsafe fn bytes(&self) -> Option<&[u8]> {
        if self.data.is_null() {
            return (self.len == 0).then_some(&[][..]);
        }
        // SAFETY: the caller's contract; `len` fits a buffer that exists.
        Some(unsafe { core::slice::from_raw_parts(self.data.cast::<u8>(), self.len) })
    }
}

/// `odb_object__alloc`: an object of `raw`'s data (which it takes on
/// success), with one reference.
fn odb_object_alloc(oid: &Oid, raw: &RawObj) -> Result<NonNull<GitOdbObject>> {
    let obj = GitOdbObject {
        cached: GitCachedObj {
            oid: *oid,
            type_: raw.type_ as i16,
            flags: 0,
            size: raw.len,
            // `git__calloc`: 0; the cache takes the first references.
            refcount: Atomic32::new(0),
        },
        buffer: raw.data,
    };
    let b = GIT_ALLOC.try_box(obj).map_err(|_| errors::oom())?;
    let (p, _) = Box::into_raw_with_allocator(b);
    // SAFETY: from a box.
    Ok(unsafe { NonNull::new_unchecked(p) })
}

/// `git_odb_object__free`: free the data and the object.
///
/// # Safety
///
/// `obj` was made by `odb_object_alloc` (or with `git__malloc` by C, with
/// a `git__allocator` buffer), and nothing uses it again.
pub unsafe fn odb_object_free(obj: NonNull<GitOdbObject>) {
    // SAFETY: the caller's contract.
    let buffer = unsafe { (*obj.as_ptr()).buffer };
    // SAFETY: the caller's contract: the object's own buffer.
    unsafe { alloc::free(buffer) };
    // SAFETY: the caller's contract.
    unsafe { alloc::free(obj.as_ptr().cast()) };
}

/// `git_cache_store_raw` of a fresh object: the reference the caller now
/// holds (to it, or to the object already cached).
fn store_raw(cache: &GitCache, obj: NonNull<GitOdbObject>) -> NonNull<GitOdbObject> {
    // SAFETY: fresh, private to the caller.
    unsafe { (*obj.as_ptr()).cached.flags = GIT_CACHE_STORE_RAW };
    // SAFETY: a live raw object, private, whose (zero) reference passes:
    // `store` takes one for the caller and one for the map.
    unsafe { cache.store(obj.cast()) }.cast()
}

// ---------------------------------------------------------------------------
// Headers, hashing, errors
// ---------------------------------------------------------------------------

/// The longest object header: `"commit -9223372036854775808"` and a NUL.
const MAX_HEADER: usize = 32;

/// The header of an object (`"<type> <len>"`, no NUL) and its length.
pub(crate) fn header_bytes(
    obj_len: git_object_size_t,
    obj_type: git_object_t,
) -> ([u8; MAX_HEADER], usize) {
    let mut tmp = [0u8; MAX_HEADER];
    let name = type2string(obj_type);
    let mut n = name.len();
    tmp[..n].copy_from_slice(name);
    tmp[n] = b' ';
    n += 1;
    n += write_i64(&mut tmp[n..], obj_len as i64);
    (tmp, n)
}

/// The error of a header that does not fit (`len >= hdr_max`).
fn header_error() -> ErrorCode {
    git_error_set!(ErrorClass::OS, "object header creation failed")
}

/// `git_odb__format_object_header` into `hdr`: the header with its NUL;
/// `Ok(len + 1)`. With too little room, the truncated header (as
/// `snprintf`) and the error.
pub fn format_object_header(
    hdr: &mut [u8],
    obj_len: git_object_size_t,
    obj_type: git_object_t,
) -> Result<usize> {
    let (tmp, n) = header_bytes(obj_len, obj_type);
    let hdr_max = hdr.len().min(c_int::MAX as usize - 2);
    if hdr_max > 0 {
        let k = n.min(hdr_max - 1);
        hdr[..k].copy_from_slice(&tmp[..k]);
        hdr[k] = 0;
    }
    if n >= hdr_max {
        return Err(header_error());
    }
    Ok(n + 1)
}

/// `"%" PRId64` of `v` into `out`; the length.
fn write_i64(out: &mut [u8], v: i64) -> usize {
    let mut digits = [0u8; 20];
    let mut u = v.unsigned_abs();
    let mut i = digits.len();
    loop {
        i -= 1;
        digits[i] = b'0' + (u % 10) as u8;
        u /= 10;
        if u == 0 {
            break;
        }
    }
    let mut n = 0;
    if v < 0 {
        out[0] = b'-';
        n = 1;
    }
    let d = &digits[i..];
    out[n..n + d.len()].copy_from_slice(d);
    n + d.len()
}

/// `git_oid_algorithm`.
pub(crate) fn oid_algorithm(t: git_oid_t) -> c_int {
    match OidType::from_raw(t) {
        Some(OidType::Sha1) => hash::raw_algorithm::SHA1,
        Some(OidType::Sha256) => hash::raw_algorithm::SHA256,
        None => 0,
    }
}

/// `git_object_id_from_buffer` without filters (what the odb uses): the id
/// of `data` (`None` with a non-zero `len` is C's NULL data) as an object
/// of `object_type` (0: blob) in a repository of `oid_type` (0: SHA-1).
pub fn hash_object(
    data: Option<&[u8]>,
    len: usize,
    object_type: git_object_t,
    oid_type: git_oid_t,
) -> Result<Oid> {
    let object_type = if object_type == 0 {
        GIT_OBJECT_BLOB
    } else {
        object_type
    };
    let oid_type = if oid_type == 0 {
        GIT_OID_SHA1
    } else {
        oid_type
    };
    if !type_is_valid(object_type) {
        return Err(git_error_set!(ErrorClass::INVALID, "invalid object type"));
    }
    let Some(kind) = OidType::from_raw(oid_type) else {
        return Err(git_error_set!(ErrorClass::INVALID, "unknown oid type"));
    };
    let data = match data {
        Some(d) => d,
        None if len != 0 => return Err(git_error_set!(ErrorClass::INVALID, "invalid object")),
        None => &[],
    };
    let mut header = [0u8; 64];
    let hdrlen = format_object_header(&mut header, data.len() as u64, object_type)?;
    let alg = match kind {
        OidType::Sha1 => hash::HashAlgorithm::Sha1,
        OidType::Sha256 => hash::HashAlgorithm::Sha256,
    };
    let digest = hash::hash_vec(&[&header[..hdrlen], data], alg)?;
    let mut id = [0u8; crate::oid::GIT_OID_MAX_SIZE];
    id[..digest.as_bytes().len()].copy_from_slice(digest.as_bytes());
    Ok(Oid::from_raw_parts(oid_type as u8, id))
}

/// `git_odb__error_mismatch`.
pub fn error_mismatch(expected: &Oid, actual: &Oid) -> ErrorCode {
    let e = expected.hex_prefix(expected.hexsize() + 1);
    let a = actual.hex_prefix(actual.hexsize() + 1);
    let _ = git_error_set!(
        ErrorClass::ODB,
        "object hash mismatch - expected {} but got {}",
        Bytes(e.as_bytes()),
        Bytes(a.as_bytes())
    );
    ErrorCode::MISMATCH
}

/// `git_odb__error_notfound`.
pub fn error_notfound(message: &[u8], oid: Option<&Oid>, oid_len: usize) -> ErrorCode {
    match oid {
        Some(oid) => {
            let s = oid.hex_prefix(oid_len.saturating_add(1));
            let s = s.as_bytes();
            let shown = &s[..s.len().min(oid_len)];
            let _ = git_error_set!(
                ErrorClass::ODB,
                "object not found - {} ({})",
                Bytes(message),
                Bytes(shown)
            );
        }
        None => {
            let _ = git_error_set!(ErrorClass::ODB, "object not found - {}", Bytes(message));
        }
    }
    ErrorCode::NOT_FOUND
}

/// `error_null_oid`.
fn error_null_oid(error: ErrorCode, message: &str) -> ErrorCode {
    let _ = git_error_set!(ErrorClass::ODB, "odb: {message}: null OID cannot exist");
    error
}

/// `git_odb__error_ambiguous`.
pub fn error_ambiguous(message: &[u8]) -> ErrorCode {
    let _ = git_error_set!(ErrorClass::ODB, "ambiguous OID prefix - {}", Bytes(message));
    ErrorCode::AMBIGUOUS
}

/// `git_odb__error_unsupported_in_backend`.
fn error_unsupported_in_backend(action: &str) -> ErrorCode {
    git_error_set!(
        ErrorClass::ODB,
        "cannot {action} - unsupported in the loaded odb backends"
    )
}

/// `git_error__check_version` of a versioned struct whose version is
/// `actual` (`None`: no struct).
pub(crate) fn check_version(
    actual: Option<c_uint>,
    expected_max: c_uint,
    name: &str,
) -> Result<()> {
    match actual {
        None => Ok(()),
        Some(v) if v > 0 && v <= expected_max => Ok(()),
        Some(v) => Err(git_error_set!(
            ErrorClass::INVALID,
            "invalid version {} on {name}",
            v as c_int
        )),
    }
}

/// `odb_hardcoded_type`.
fn hardcoded_type(id: &Oid) -> git_object_t {
    if *id == Oid::EMPTY_TREE_SHA1 {
        GIT_OBJECT_TREE
    } else {
        GIT_OBJECT_INVALID
    }
}

/// `git_oid__cpy_prefix`: the first `len` hex digits of `id`, the rest
/// zero.
fn cpy_prefix(id: &Oid, len: usize) -> Oid {
    id.prefix(len)
}

// ---------------------------------------------------------------------------
// The odb
// ---------------------------------------------------------------------------

/// `git_odb__loose_priority` (settings.c writes it).
#[unsafe(no_mangle)]
pub static git_odb__loose_priority: AtomicI32 = AtomicI32::new(GIT_ODB_DEFAULT_LOOSE_PRIORITY);
/// `git_odb__packed_priority` (settings.c writes it).
#[unsafe(no_mangle)]
pub static git_odb__packed_priority: AtomicI32 = AtomicI32::new(GIT_ODB_DEFAULT_PACKED_PRIORITY);
/// `git_odb__strict_hash_verification` (settings.c writes it).
#[unsafe(no_mangle)]
pub static git_odb__strict_hash_verification: AtomicBool = AtomicBool::new(true);

fn strict_hash_verification() -> bool {
    git_odb__strict_hash_verification.load(Ordering::Relaxed)
}

/// `backend_internal`.
#[derive(Clone, Copy, Debug)]
struct BackendInternal {
    backend: NonNull<GitOdbBackend>,
    priority: c_int,
    is_alternate: bool,
    disk_inode: u64,
}

/// What the odb's mutex protects: the backends (C's `git_vector`, sorted
/// with `backend_sort_cmp`) and the commit-graph.
#[derive(Debug, Default)]
struct Locked {
    backends: Vec<BackendInternal>,
    cgraph: Option<CommitGraph>,
}

// SAFETY: the backends are C tables libgit2 calls from any thread under
// this mutex (their contract); the commit-graph likewise.
unsafe impl Send for Locked {}

/// `backend_sort_cmp`: higher priority first; at equal priority, C's
/// comparison puts an alternate first (and is not antisymmetric when both
/// are: kept, as the order is timsort's).
fn backend_sort_cmp(a: BackendInternal, b: BackendInternal) -> c_int {
    if b.priority == a.priority {
        if a.is_alternate {
            return -1;
        }
        if b.is_alternate {
            return 1;
        }
        return 0;
    }
    b.priority.wrapping_sub(a.priority)
}

/// `git_odb`: the C-visible prefix (`rc`, `options`), then Rust state.
#[repr(C)]
pub struct GitOdb {
    /// The count and owner (a repository).
    pub(crate) rc: GitRefcount,
    /// The options, normalized (immutable after creation).
    pub(crate) options: GitOdbOptions,
    locked: Mutex<Locked>,
    own_cache: GitCache,
    do_fsync: AtomicBool,
}

impl core::fmt::Debug for GitOdb {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GitOdb")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl GitOdb {
    /// `git_odb_new_ext`: an odb with no backends and one reference.
    pub(crate) fn new(opts: Option<&GitOdbOptions>) -> Result<NonNull<GitOdb>> {
        check_version(
            opts.map(|o| o.version),
            GIT_ODB_OPTIONS_VERSION,
            "git_odb_options",
        )?;
        let mut options = opts.copied().unwrap_or_default();
        if options.oid_type == 0 {
            options.oid_type = OidType::DEFAULT.raw();
        }
        let mut backends = Vec::new();
        backends.try_reserve(4).map_err(|_| errors::oom())?;
        let db = GitOdb {
            rc: GitRefcount::new(1),
            options,
            locked: Mutex::new(Locked {
                backends,
                cgraph: None,
            }),
            own_cache: GitCache::new(),
            do_fsync: AtomicBool::new(false),
        };
        let b = GIT_ALLOC.try_box(db).map_err(|_| errors::oom())?;
        let (p, _) = Box::into_raw_with_allocator(b);
        // SAFETY: from a box.
        Ok(unsafe { NonNull::new_unchecked(p) })
    }

    /// `odb_free`: free the backends (with the lock held, as C), the
    /// commit-graph, the cache and the odb.
    ///
    /// # Safety
    ///
    /// `db` came from [`new`](Self::new); its last reference is gone and
    /// nothing uses it again.
    unsafe fn free(db: NonNull<GitOdb>) {
        // SAFETY: the caller's contract: a box of `GIT_ALLOC`.
        let mut b = unsafe { Box::from_raw_in(db.as_ptr(), GIT_ALLOC) };
        {
            let mut l = b.lock();
            for bi in core::mem::take(&mut l.backends) {
                // SAFETY: each backend was handed to this odb, which frees
                // it once (its `free` slot is required).
                if let Some(free) = unsafe { (*bi.backend.as_ptr()).free } {
                    // SAFETY: as above.
                    unsafe { free(bi.backend.as_ptr()) };
                }
            }
            l.cgraph = None;
        }
        b.own_cache.dispose();
        drop(b);
    }

    /// The backends' lock (C's `git_mutex_lock`, which never fails).
    fn lock(&self) -> MutexGuard<'_, Locked> {
        self.locked.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The oid type.
    pub(crate) fn oid_type(&self) -> git_oid_t {
        self.options.oid_type
    }

    pub(crate) fn hexsize(&self) -> usize {
        oid_hexsize(self.options.oid_type)
    }

    /// `odb_cache`: the owning repository's cache, or the odb's own.
    pub(crate) fn cache(&self) -> &GitCache {
        let owner = self.rc.owner();
        if owner.is_null() {
            &self.own_cache
        } else {
            // SAFETY: the owner of an odb is a live repository (it clears
            // the owner before it goes); `GitRepository` is `Sync`.
            let repo = unsafe { &*owner.cast::<crate::repository::GitRepository>() };
            repo.objects()
        }
    }

    /// Whether objects are fsync'd (`do_fsync`).
    pub(crate) fn do_fsync(&self) -> bool {
        self.do_fsync.load(Ordering::Relaxed)
    }

    /// `add_backend_internal`.
    ///
    /// # Safety
    ///
    /// `backend` is a live backend table (as `git_odb_add_backend`
    /// requires), whose ownership passes to the odb on success.
    unsafe fn add_backend_internal(
        &self,
        backend: NonNull<GitOdbBackend>,
        priority: c_int,
        is_alternate: bool,
        disk_inode: u64,
    ) -> Result<()> {
        // SAFETY: the caller's backend is a live table (the exports'
        // contract); fields are read in place.
        let t = unsafe { slots(backend.as_ptr()) };
        let (version, owner) = (t.version, t.odb);
        check_version(Some(version), GIT_ODB_BACKEND_VERSION, "git_odb_backend")?;
        let me = core::ptr::from_ref(self).cast_mut();
        git_assert!(
            owner.is_null() || owner == me,
            "!backend->odb || backend->odb == odb"
        );
        let mut l = self.lock();
        l.backends.try_reserve(1).map_err(|_| errors::oom())?;
        l.backends.push(BackendInternal {
            backend,
            priority,
            is_alternate,
            disk_inode,
        });
        if l.backends.len() > 1 {
            crate::util::tsort::tsort(&mut l.backends, backend_sort_cmp);
        }
        // SAFETY: a field of the live table, written in place while the
        // lock is held (C writes it the same way).
        unsafe { (*backend.as_ptr()).odb = me };
        Ok(())
    }

    /// `git_odb_add_backend` / `git_odb_add_alternate`.
    ///
    /// # Safety
    ///
    /// As [`add_backend_internal`](Self::add_backend_internal).
    pub(crate) unsafe fn add_backend(
        &self,
        backend: NonNull<GitOdbBackend>,
        priority: c_int,
        is_alternate: bool,
    ) -> Result<()> {
        // SAFETY: the caller's contract.
        unsafe { self.add_backend_internal(backend, priority, is_alternate, 0) }
    }

    /// `git_odb_num_backends`.
    pub(crate) fn num_backends(&self) -> usize {
        self.lock().backends.len()
    }

    /// `git_odb_get_backend`.
    pub(crate) fn get_backend(&self, pos: usize) -> Result<NonNull<GitOdbBackend>> {
        match self.lock().backends.get(pos) {
            Some(b) => Ok(b.backend),
            None => {
                let _ = git_error_set!(ErrorClass::ODB, "no ODB backend loaded at index {pos}");
                Err(ErrorCode::NOT_FOUND)
            }
        }
    }

    /// `git_odb__add_default_backends`.
    pub(crate) fn add_default_backends(
        &self,
        objects_dir: &CStr,
        as_alternates: bool,
        alternate_depth: c_int,
    ) -> Result<()> {
        let st = match crate::util::posix::stat(objects_dir) {
            Ok(st) => st,
            Err(_) => {
                if as_alternates {
                    return Ok(());
                }
                return Err(git_error_set!(
                    ErrorClass::ODB,
                    "failed to load object database in '{}'",
                    Bytes(objects_dir.to_bytes())
                ));
            }
        };
        let inode = st.st_ino;
        if self.lock().backends.iter().any(|b| b.disk_inode == inode) {
            return Ok(());
        }

        let mut loose_opts = GitOdbBackendLooseOptions::default();
        if self.do_fsync() {
            loose_opts.flags |= GIT_ODB_BACKEND_LOOSE_FSYNC;
        }
        loose_opts.oid_type = self.options.oid_type;
        let pack_opts = GitOdbBackendPackOptions {
            version: 1,
            oid_type: self.options.oid_type,
        };

        // C returns -1 for each failure below, whatever the callee's code.
        let loose = crate::odb_loose::new_backend(objects_dir.to_bytes(), Some(&loose_opts))
            .map_err(|_| ErrorCode::GENERIC)?;
        // SAFETY: a backend table this function just made.
        if unsafe {
            self.add_backend_internal(
                loose,
                git_odb__loose_priority.load(Ordering::Relaxed),
                as_alternates,
                inode,
            )
        }
        .is_err()
        {
            // C leaked the backend here (rust/DEFECTS.md).
            // SAFETY: ours, not added: freed once.
            unsafe { free_backend(loose) };
            return Err(ErrorCode::GENERIC);
        }

        let packed = crate::odb_pack::backend_pack_raw(objects_dir, &pack_opts)
            .map_err(|_| ErrorCode::GENERIC)?;
        // SAFETY: a backend table the pack backend just made.
        if unsafe {
            self.add_backend_internal(
                packed,
                git_odb__packed_priority.load(Ordering::Relaxed),
                as_alternates,
                inode,
            )
        }
        .is_err()
        {
            // SAFETY: as above.
            unsafe { free_backend(packed) };
            return Err(ErrorCode::GENERIC);
        }

        {
            let mut l = self.lock();
            if l.cgraph.is_none() {
                l.cgraph = Some(
                    CommitGraph::new(objects_dir, self.options.oid_type)
                        .map_err(|_| ErrorCode::GENERIC)?,
                );
            }
        }

        self.load_alternates(objects_dir, alternate_depth)
    }

    /// `load_alternates`.
    fn load_alternates(&self, objects_dir: &CStr, alternate_depth: c_int) -> Result<()> {
        use crate::util::str::GitStr;
        if alternate_depth > GIT_ALTERNATES_MAX_DEPTH {
            return Ok(());
        }
        let mut path = GitStr::new();
        path.joinpath(objects_dir.to_bytes(), b"info/alternates")?;
        let Some(p) = path.as_c_str() else {
            return Err(ErrorCode::GENERIC);
        };
        if !crate::util::fs_path::exists(p) {
            return Ok(());
        }
        let mut buf = GitStr::new();
        crate::util::futils::readbuffer(&mut buf, p)?;
        // `git__strtok(&buffer, "\r\n")` stops at the first NUL.
        let content = buf.as_bytes();
        let content = &content[..memchr::memchr(0, content).unwrap_or(content.len())];
        for alternate in content.split(|&c| c == b'\r' || c == b'\n') {
            if alternate.is_empty() || alternate[0] == b'#' {
                continue;
            }
            let alt: std::ffi::CString;
            let target: &CStr = if alternate[0] == b'.' {
                path.joinpath(objects_dir.to_bytes(), alternate)?;
                match path.as_c_str() {
                    Some(p) => p,
                    None => return Err(ErrorCode::GENERIC),
                }
            } else {
                alt = std::ffi::CString::new(alternate).map_err(|_| ErrorCode::GENERIC)?;
                &alt
            };
            let target = target.to_owned();
            self.add_default_backends(&target, true, alternate_depth + 1)?;
        }
        Ok(())
    }

    /// `git_odb_set_commit_graph`: take `cgraph` (or none), freeing the
    /// previous one.
    pub(crate) fn set_commit_graph(&self, cgraph: Option<CommitGraph>) {
        let old = core::mem::replace(&mut self.lock().cgraph, cgraph);
        drop(old);
    }

    /// `git_odb__set_caps`.
    pub(crate) fn set_caps(&self, caps: c_int) -> Result<()> {
        if caps == GIT_ODB_CAP_FROM_OWNER {
            let repo = self.rc.owner();
            if repo.is_null() {
                return Err(git_error_set!(
                    ErrorClass::ODB,
                    "cannot access repository to set odb caps"
                ));
            }
            // SAFETY: the owner of an odb is a live repository (it clears
            // the owner before it goes); `GitRepository` is `Sync`.
            let repo = unsafe { &*repo.cast::<crate::repository::GitRepository>() };
            if let Ok(val) =
                repo.configmap_lookup(crate::repository::GIT_CONFIGMAP_FSYNCOBJECTFILES)
            {
                self.do_fsync.store(val != 0, Ordering::Relaxed);
            }
        }
        Ok(())
    }

    /// `git_odb__get_commit_graph_file`.
    pub(crate) fn get_commit_graph_file(&self) -> Result<*mut c_void> {
        let mut l = self.lock();
        match l.cgraph.as_mut() {
            None => Err(ErrorCode::NOT_FOUND),
            Some(g) => g.get_file(),
        }
    }

    // -- lookups -----------------------------------------------------------

    /// `odb_exists_1`.
    fn exists_1(&self, id: &Oid, only_refreshed: bool) -> bool {
        let l = self.lock();
        for bi in &l.backends {
            let b = bi.backend.as_ptr();
            // SAFETY: a live backend table (the odb's), read in place.
            let t = unsafe { slots(b) };
            let (refresh, exists) = (t.refresh, t.exists);
            if only_refreshed && refresh.is_none() {
                continue;
            }
            if let Some(exists) = exists {
                // SAFETY: the slot of the table, given the table and a live
                // oid, with the lock held (the backends' contract).
                if unsafe { exists(b, id) } != 0 {
                    return true;
                }
            }
        }
        false
    }

    /// `odb_freshen_1`.
    fn freshen_1(&self, id: &Oid, only_refreshed: bool) -> bool {
        let l = self.lock();
        for bi in &l.backends {
            let b = bi.backend.as_ptr();
            // SAFETY: as in `exists_1`.
            let t = unsafe { slots(b) };
            let (refresh, freshen, exists) = (t.refresh, t.freshen, t.exists);
            if only_refreshed && refresh.is_none() {
                continue;
            }
            let found = if let Some(freshen) = freshen {
                // SAFETY: as in `exists_1`.
                unsafe { freshen(b, id) == 0 }
            } else if let Some(exists) = exists {
                // SAFETY: as in `exists_1`.
                unsafe { exists(b, id) != 0 }
            } else {
                false
            };
            if found {
                return true;
            }
        }
        false
    }

    /// `git_odb__freshen`.
    pub(crate) fn freshen(&self, id: &Oid) -> bool {
        if self.freshen_1(id, false) {
            return true;
        }
        if self.refresh().is_ok() {
            return self.freshen_1(id, true);
        }
        false
    }

    /// `git_odb_exists_ext`.
    pub(crate) fn exists_ext(&self, id: &Oid, flags: c_uint) -> bool {
        if id.is_zero() {
            return false;
        }
        if let Some(obj) = self.cache().get(id, GIT_CACHE_STORE_RAW) {
            // SAFETY: the reference `get` gave.
            unsafe { cache::decref(obj) };
            return true;
        }
        if self.exists_1(id, false) {
            return true;
        }
        if flags & GIT_ODB_LOOKUP_NO_REFRESH == 0 && self.refresh().is_ok() {
            return self.exists_1(id, true);
        }
        false
    }

    /// `odb_exists_prefix_1`.
    fn exists_prefix_1(&self, key: &Oid, len: usize, only_refreshed: bool) -> Result<Oid> {
        let l = self.lock();
        let mut last_found: Option<Oid> = None;
        for bi in &l.backends {
            let b = bi.backend.as_ptr();
            // SAFETY: as in `exists_1`.
            let t = unsafe { slots(b) };
            let (refresh, exists_prefix) = (t.refresh, t.exists_prefix);
            if only_refreshed && refresh.is_none() {
                continue;
            }
            let Some(exists_prefix) = exists_prefix else {
                continue;
            };
            let mut found = Oid::NONE;
            // SAFETY: as in `exists_1`; `found` is writable.
            let error = unsafe { exists_prefix(&raw mut found, b, key, len) };
            if error == ErrorCode::NOT_FOUND.raw() || error == ErrorCode::PASSTHROUGH.raw() {
                continue;
            }
            if let Some(e) = ErrorCode::from_raw(error) {
                return Err(e);
            }
            match last_found {
                Some(last) => {
                    if last.cmp_raw(&found) != 0 {
                        return Err(error_ambiguous(b"multiple matches for prefix"));
                    }
                }
                None => last_found = Some(found),
            }
        }
        last_found.ok_or(ErrorCode::NOT_FOUND)
    }

    /// `git_odb_exists_prefix`.
    pub(crate) fn exists_prefix(&self, short_id: &Oid, len: usize) -> Result<Oid> {
        if len < GIT_OID_MINPREFIXLEN {
            return Err(error_ambiguous(b"prefix length too short"));
        }
        if len >= self.hexsize() {
            if self.exists_ext(short_id, 0) {
                return Ok(*short_id);
            }
            return Err(error_notfound(
                b"no match for id prefix",
                Some(short_id),
                len,
            ));
        }
        let key = cpy_prefix(short_id, len);
        let mut r = self.exists_prefix_1(&key, len, false);
        if r == Err(ErrorCode::NOT_FOUND) && self.refresh().is_ok() {
            r = self.exists_prefix_1(&key, len, true);
        }
        if r == Err(ErrorCode::NOT_FOUND) {
            return Err(error_notfound(b"no match for id prefix", Some(&key), len));
        }
        r
    }

    /// `git_odb_expand_ids`.
    pub(crate) fn expand_ids(&self, ids: &mut [GitOdbExpandId]) -> Result<()> {
        let hex_size = self.hexsize();
        for query in ids.iter_mut() {
            let mut error: Result<()> = Err(ErrorCode::AMBIGUOUS);
            if query.type_ == 0 {
                query.type_ = GIT_OBJECT_ANY;
            }
            let qlen = usize::from(query.length);
            if qlen >= GIT_OID_MINPREFIXLEN && qlen < hex_size {
                match self.exists_prefix_1(&query.id, qlen, false) {
                    Ok(actual) => {
                        query.id = actual;
                        query.length = hex_size as c_ushort;
                        error = Ok(());
                    }
                    Err(e) => error = Err(e),
                }
            }
            if usize::from(query.length) >= hex_size {
                error = match self.otype_fast(&query.id) {
                    Ok(actual) => {
                        if query.type_ != GIT_OBJECT_ANY && query.type_ != actual {
                            Err(ErrorCode::NOT_FOUND)
                        } else {
                            query.type_ = actual;
                            Ok(())
                        }
                    }
                    Err(e) => Err(e),
                };
            }
            match error {
                Ok(()) => continue,
                Err(ErrorCode::NOT_FOUND) | Err(ErrorCode::AMBIGUOUS) => {
                    // `git_oid_clear`: zero, with the odb's type.
                    query.id = Oid::from_raw_parts(self.options.oid_type as u8, [0; 32]);
                    query.length = 0;
                    query.type_ = 0;
                }
                Err(e) => return Err(e),
            }
        }
        errors::clear();
        Ok(())
    }

    /// `odb_read_header_1`: `Ok(Some((len, type)))`, or the passthrough
    /// (`Ok(None)`) / not-found code.
    fn read_header_1(&self, id: &Oid, only_refreshed: bool) -> Result<(usize, git_object_t)> {
        if !only_refreshed {
            let ht = hardcoded_type(id);
            if ht != GIT_OBJECT_INVALID {
                return Ok((0, ht));
            }
        }
        let mut passthrough = false;
        let l = self.lock();
        for bi in &l.backends {
            let b = bi.backend.as_ptr();
            // SAFETY: as in `exists_1`.
            let t = unsafe { slots(b) };
            let (refresh, read_header) = (t.refresh, t.read_header);
            if only_refreshed && refresh.is_none() {
                continue;
            }
            let Some(read_header) = read_header else {
                passthrough = true;
                continue;
            };
            let mut len = 0usize;
            let mut t = GIT_OBJECT_INVALID;
            // SAFETY: as in `exists_1`; the out-parameters are writable.
            let error = unsafe { read_header(&raw mut len, &raw mut t, b, id) };
            match error {
                0 => return Ok((len, t)),
                e if e == ErrorCode::PASSTHROUGH.raw() => passthrough = true,
                e if e == ErrorCode::NOT_FOUND.raw() => {}
                e => return Err(ErrorCode::from_raw(e).unwrap_or(ErrorCode::GENERIC)),
            }
        }
        Err(if passthrough {
            ErrorCode::PASSTHROUGH
        } else {
            ErrorCode::NOT_FOUND
        })
    }

    /// `git_odb__read_header_or_object`: the size and type, and the object
    /// when it had to be read (or was cached).
    pub(crate) fn read_header_or_object(
        &self,
        id: &Oid,
    ) -> Result<(usize, git_object_t, Option<NonNull<GitOdbObject>>)> {
        if id.is_zero() {
            return Err(error_null_oid(ErrorCode::NOT_FOUND, "cannot read object"));
        }
        if let Some(obj) = self.cache().get(id, GIT_CACHE_STORE_RAW) {
            let obj = obj.cast::<GitOdbObject>();
            // SAFETY: a live object we hold a reference to.
            let o = unsafe { obj.as_ref() };
            let (size, t) = (o.cached.size, o.cached.type_);
            return Ok((size, git_object_t::from(t), Some(obj)));
        }
        let mut r = self.read_header_1(id, false);
        if r == Err(ErrorCode::NOT_FOUND) && self.refresh().is_ok() {
            r = self.read_header_1(id, true);
        }
        match r {
            Err(ErrorCode::NOT_FOUND) => Err(error_notfound(
                b"cannot read header for",
                Some(id),
                self.hexsize(),
            )),
            Ok((len, t)) => Ok((len, t, None)),
            Err(ErrorCode::PASSTHROUGH) => {
                let obj = self.read(id)?;
                // SAFETY: a live object we hold a reference to.
                let o = unsafe { obj.as_ref() };
                let (size, t) = (o.cached.size, o.cached.type_);
                Ok((size, git_object_t::from(t), Some(obj)))
            }
            Err(e) => Err(e),
        }
    }

    /// `git_odb_read_header`.
    pub(crate) fn read_header(&self, id: &Oid) -> Result<(usize, git_object_t)> {
        let (len, t, obj) = self.read_header_or_object(id)?;
        if let Some(obj) = obj {
            // SAFETY: our reference.
            unsafe { cache::decref(obj.cast()) };
        }
        Ok((len, t))
    }

    /// Verify `raw` hashes to `id` (`git_odb__strict_hash_verification`).
    fn verify(&self, id: &Oid, raw: &RawObj) -> Result<()> {
        if !strict_hash_verification() {
            return Ok(());
        }
        // SAFETY: a backend's buffer is readable for its length.
        let data = unsafe { raw.bytes() };
        let hashed = hash_object(data, raw.len, raw.type_, self.options.oid_type)?;
        if *id != hashed {
            return Err(error_mismatch(id, &hashed));
        }
        Ok(())
    }

    /// `odb_read_1`.
    fn read_1(&self, id: &Oid, only_refreshed: bool) -> Result<NonNull<GitOdbObject>> {
        let mut raw = RawObj::empty();
        let mut found = false;
        if !only_refreshed && hardcoded_type(id) != GIT_OBJECT_INVALID {
            raw.type_ = hardcoded_type(id);
            raw.len = 0;
            raw.data = alloc::calloc(1, 1);
            if raw.data.is_null() {
                return Err(ErrorCode::GENERIC);
            }
            found = true;
        }
        if !found {
            let l = self.lock();
            for bi in &l.backends {
                let b = bi.backend.as_ptr();
                // SAFETY: as in `exists_1`.
                let t = unsafe { slots(b) };
                let (refresh, read) = (t.refresh, t.read);
                if only_refreshed && refresh.is_none() {
                    continue;
                }
                let Some(read) = read else {
                    continue;
                };
                // SAFETY: as in `exists_1`; the out-parameters are writable.
                let error = unsafe {
                    read(
                        &raw mut raw.data,
                        &raw mut raw.len,
                        &raw mut raw.type_,
                        b,
                        id,
                    )
                };
                if error == ErrorCode::PASSTHROUGH.raw() || error == ErrorCode::NOT_FOUND.raw() {
                    continue;
                }
                if error < 0 {
                    return Err(ErrorCode::from_raw(error).unwrap_or(ErrorCode::GENERIC));
                }
                found = true;
                break;
            }
        }
        if !found {
            return Err(ErrorCode::NOT_FOUND);
        }
        if let Err(e) = self.verify(id, &raw) {
            raw.free();
            return Err(e);
        }
        errors::clear();
        let obj = match odb_object_alloc(id, &raw) {
            Ok(o) => o,
            Err(e) => {
                raw.free();
                return Err(e);
            }
        };
        Ok(store_raw(self.cache(), obj))
    }

    /// `git_odb_read`: a new reference to the object.
    pub(crate) fn read(&self, id: &Oid) -> Result<NonNull<GitOdbObject>> {
        if id.is_zero() {
            return Err(error_null_oid(ErrorCode::NOT_FOUND, "cannot read object"));
        }
        if let Some(obj) = self.cache().get(id, GIT_CACHE_STORE_RAW) {
            return Ok(obj.cast());
        }
        let mut r = self.read_1(id, false);
        if r == Err(ErrorCode::NOT_FOUND) && self.refresh().is_ok() {
            r = self.read_1(id, true);
        }
        if r == Err(ErrorCode::NOT_FOUND) {
            return Err(error_notfound(b"no match for id", Some(id), id.hexsize()));
        }
        r
    }

    /// `odb_otype_fast`.
    fn otype_fast(&self, id: &Oid) -> Result<git_object_t> {
        if id.is_zero() {
            return Err(error_null_oid(
                ErrorCode::NOT_FOUND,
                "cannot get object type",
            ));
        }
        if let Some(obj) = self.cache().get(id, GIT_CACHE_STORE_RAW) {
            // SAFETY: our reference, given up after the read.
            let t = unsafe { (*obj.cast::<GitOdbObject>().as_ptr()).cached.type_ };
            // SAFETY: as above.
            unsafe { cache::decref(obj) };
            return Ok(git_object_t::from(t));
        }
        match self.read_header_1(id, false) {
            Ok((_, t)) => Ok(t),
            Err(ErrorCode::PASSTHROUGH) => {
                let obj = self.read_1(id, false)?;
                // SAFETY: as above.
                let t = unsafe { (*obj.as_ptr()).cached.type_ };
                // SAFETY: as above.
                unsafe { cache::decref(obj.cast()) };
                Ok(git_object_t::from(t))
            }
            Err(e) => Err(e),
        }
    }

    /// `read_prefix_1`.
    fn read_prefix_1(
        &self,
        key: &Oid,
        len: usize,
        only_refreshed: bool,
    ) -> Result<NonNull<GitOdbObject>> {
        let mut raw = RawObj::empty();
        let mut data: *mut c_void = core::ptr::null_mut();
        let mut found: Option<Oid> = None;
        let r = (|| {
            let l = self.lock();
            for bi in &l.backends {
                let b = bi.backend.as_ptr();
                // SAFETY: as in `exists_1`.
                let t = unsafe { slots(b) };
                let (refresh, read_prefix) = (t.refresh, t.read_prefix);
                if only_refreshed && refresh.is_none() {
                    continue;
                }
                let Some(read_prefix) = read_prefix else {
                    continue;
                };
                let mut full_oid = Oid::NONE;
                // SAFETY: as in `exists_1`; the out-parameters are writable.
                let error = unsafe {
                    read_prefix(
                        &raw mut full_oid,
                        &raw mut raw.data,
                        &raw mut raw.len,
                        &raw mut raw.type_,
                        b,
                        key,
                        len,
                    )
                };
                if error == ErrorCode::NOT_FOUND.raw() || error == ErrorCode::PASSTHROUGH.raw() {
                    continue;
                }
                if let Some(e) = ErrorCode::from_raw(error) {
                    return Err(e);
                }
                // C frees the previous match's data (and, on an ambiguity,
                // `raw.data` too, on the way out).
                // SAFETY: the previous backend's buffer, owned here.
                unsafe { alloc::free(data) };
                data = raw.data;
                if let Some(prev) = found
                    && full_oid.cmp_raw(&prev) != 0
                {
                    let mut msg = Vec::new();
                    msg.extend_from_slice(b"multiple matches for prefix: ");
                    msg.extend_from_slice(full_oid.hex().as_bytes());
                    msg.push(b' ');
                    msg.extend_from_slice(prev.hex().as_bytes());
                    return Err(error_ambiguous(&msg));
                }
                found = Some(full_oid);
            }
            Ok(())
        })();
        if let Err(e) = r {
            // C frees `raw.data` (also `data`, when it is the same buffer).
            raw.free();
            return Err(e);
        }
        let Some(found_full_oid) = found else {
            return Err(ErrorCode::NOT_FOUND);
        };
        if let Err(e) = self.verify(&found_full_oid, &raw) {
            raw.free();
            return Err(e);
        }
        let obj = match odb_object_alloc(&found_full_oid, &raw) {
            Ok(o) => o,
            Err(e) => {
                raw.free();
                return Err(e);
            }
        };
        Ok(store_raw(self.cache(), obj))
    }

    /// `git_odb_read_prefix`.
    pub(crate) fn read_prefix(&self, short_id: &Oid, len: usize) -> Result<NonNull<GitOdbObject>> {
        let hex_size = self.hexsize();
        if len < GIT_OID_MINPREFIXLEN {
            return Err(error_ambiguous(b"prefix length too short"));
        }
        let len = len.min(hex_size);
        if len == hex_size
            && let Some(obj) = self.cache().get(short_id, GIT_CACHE_STORE_RAW)
        {
            return Ok(obj.cast());
        }
        let key = cpy_prefix(short_id, len);
        let mut r = self.read_prefix_1(&key, len, false);
        if r == Err(ErrorCode::NOT_FOUND) && self.refresh().is_ok() {
            r = self.read_prefix_1(&key, len, true);
        }
        if r == Err(ErrorCode::NOT_FOUND) {
            return Err(error_notfound(b"no match for prefix", Some(&key), len));
        }
        r
    }

    /// A copy of the backends (`git_vector_dup` under the lock), to call
    /// without the lock.
    fn backends_snapshot(&self) -> Result<Vec<BackendInternal>> {
        let l = self.lock();
        let mut v = Vec::new();
        v.try_reserve_exact(l.backends.len())
            .map_err(|_| errors::oom())?;
        v.extend_from_slice(&l.backends);
        Ok(v)
    }

    /// `git_odb_foreach` with C's callback.
    ///
    /// # Safety
    ///
    /// `cb` may be called with `payload` (the caller's contract with it).
    pub(crate) unsafe fn foreach_raw(&self, cb: ForeachCb, payload: *mut c_void) -> c_int {
        let backends = match self.backends_snapshot() {
            Ok(b) => b,
            Err(e) => return e.raw(),
        };
        for bi in backends {
            let b = bi.backend.as_ptr();
            // SAFETY: as in `exists_1` (C calls it without the lock, and
            // without checking the slot: a backend without `foreach`, such
            // as mempack, is skipped here; rust/DEFECTS.md).
            let Some(foreach) = (unsafe { (*b).foreach }) else {
                continue;
            };
            // SAFETY: as above; the caller's callback and payload.
            let error = unsafe { foreach(b, cb, payload) };
            if error != 0 {
                return error;
            }
        }
        0
    }

    /// `git_odb_write`: the id is written to `out` once computed (C wrote
    /// `*oid` before trying the backends, so it is set on their failures
    /// too).
    pub(crate) fn write(
        &self,
        data: &[u8],
        type_: git_object_t,
        out: &mut Option<Oid>,
    ) -> Result<()> {
        let len = data.len();
        let oid = hash_object(Some(data), len, type_, self.options.oid_type)?;
        *out = Some(oid);
        if oid.is_zero() {
            return Err(error_null_oid(ErrorCode::INVALID, "cannot write object"));
        }
        if self.freshen(&oid) {
            return Ok(());
        }
        let data_ptr = data.as_ptr().cast::<c_void>();
        let mut error = ErrorCode::GENERIC.raw();
        {
            let l = self.lock();
            for bi in &l.backends {
                if error >= 0 {
                    break;
                }
                if bi.is_alternate {
                    continue;
                }
                let b = bi.backend.as_ptr();
                // SAFETY: as in `exists_1`.
                if let Some(write) = unsafe { (*b).write } {
                    // SAFETY: as in `exists_1`; `data` is readable for
                    // `len` bytes (a slice).
                    error = unsafe { write(b, &raw const oid, data_ptr, len, type_) };
                }
            }
        }
        if error == 0 || error == ErrorCode::PASSTHROUGH.raw() {
            return Ok(());
        }
        let stream = self.open_wstream(len as git_object_size_t, type_)?;
        let stream = stream.as_ptr();
        // SAFETY: a live stream (ours until freed below); `data` as above.
        let mut r = unsafe { stream_write_slot(stream, data_ptr.cast(), len) };
        if r == 0 {
            // SAFETY: as above.
            r = match unsafe { (*stream).finalize_write } {
                // SAFETY: as above.
                Some(f) => unsafe { f(stream, &raw const oid) },
                None => git_error_set!(ErrorClass::ODB, "stream does not support writing").raw(),
            };
        }
        // SAFETY: ours, freed once.
        unsafe { stream_free(stream) };
        ErrorCode::check(r).map(drop)
    }

    /// `git_odb_open_wstream`.
    pub(crate) fn open_wstream(
        &self,
        size: git_object_size_t,
        type_: git_object_t,
    ) -> Result<NonNull<GitOdbStream>> {
        let mut writes = 0usize;
        let mut error = ErrorCode::GENERIC.raw();
        let mut stream: *mut GitOdbStream = core::ptr::null_mut();
        {
            let l = self.lock();
            for bi in &l.backends {
                if error >= 0 {
                    break;
                }
                if bi.is_alternate {
                    continue;
                }
                let b = bi.backend.as_ptr();
                // SAFETY: as in `exists_1`.
                let t = unsafe { slots(b) };
                let (writestream, write) = (t.writestream, t.write);
                if let Some(writestream) = writestream {
                    writes += 1;
                    // SAFETY: as in `exists_1`; `stream` is writable.
                    error = unsafe { writestream(&raw mut stream, b, size, type_) };
                } else if write.is_some() {
                    writes += 1;
                    error = match fake_wstream_new(b, size, type_) {
                        Ok(s) => {
                            stream = s.as_ptr();
                            0
                        }
                        Err(e) => e.raw(),
                    };
                }
            }
        }
        if error < 0 {
            if error == ErrorCode::PASSTHROUGH.raw() {
                // C returns 0 with the stream the backend left (if any).
                return NonNull::new(stream).ok_or(ErrorCode::PASSTHROUGH);
            }
            if writes == 0 {
                return Err(error_unsupported_in_backend("write object"));
            }
            return Err(ErrorCode::from_raw(error).unwrap_or(ErrorCode::GENERIC));
        }
        let Some(stream) = NonNull::new(stream) else {
            return Err(git_error_set!(
                ErrorClass::ODB,
                "backend returned no stream"
            ));
        };
        // `git__malloc`: `init_raw` writes the context in place, so it is
        // not zeroed first.
        let ctx = alloc::malloc(size_of::<HashCtx>()).cast::<HashCtx>();
        if ctx.is_null() {
            // C leaked the stream here (rust/DEFECTS.md).
            // SAFETY: the backend's stream, not handed out.
            unsafe { stream_free(stream.as_ptr()) };
            return Err(ErrorCode::GENERIC);
        }
        let r = (|| {
            // SAFETY: a fresh context of ours, written in place.
            unsafe { HashCtx::init_raw(ctx, oid_algorithm(self.options.oid_type)) }?;
            let mut header = [0u8; 64];
            let hdrlen = format_object_header(&mut header, size, type_)?;
            // SAFETY: a fresh context, ours.
            let c = unsafe { &mut *ctx };
            // SAFETY: initialized above.
            unsafe { c.hasher() }?.update(&header[..hdrlen])
        })();
        if let Err(e) = r {
            // SAFETY: ours, not handed out.
            unsafe { alloc::free(ctx.cast()) };
            // C leaked the stream here (rust/DEFECTS.md).
            // SAFETY: the backend's stream, not handed out; its context
            // is still NULL.
            unsafe { stream_free(stream.as_ptr()) };
            return Err(e);
        }
        let s = stream.as_ptr();
        // SAFETY: the stream's C-visible fields, written in place; the
        // stream is the caller's alone.
        let s = unsafe { &mut *s };
        s.oid_type = self.options.oid_type;
        s.hash_ctx = ctx.cast();
        s.declared_size = size;
        s.received_bytes = 0;
        Ok(stream)
    }

    /// `git_odb_open_rstream`: the stream, the object's size and type.
    /// `Ok(None)`: a backend passed through without making a stream (C
    /// returns 0 and writes nothing then).
    pub(crate) fn open_rstream(
        &self,
        oid: &Oid,
    ) -> Result<Option<(NonNull<GitOdbStream>, usize, git_object_t)>> {
        let mut reads = 0usize;
        let mut error = ErrorCode::GENERIC.raw();
        let mut stream: *mut GitOdbStream = core::ptr::null_mut();
        let mut len = 0usize;
        let mut t = GIT_OBJECT_INVALID;
        {
            let l = self.lock();
            for bi in &l.backends {
                if error >= 0 {
                    break;
                }
                let b = bi.backend.as_ptr();
                // SAFETY: as in `exists_1`.
                if let Some(readstream) = unsafe { (*b).readstream } {
                    reads += 1;
                    // SAFETY: as in `exists_1`; the out-parameters are
                    // writable.
                    error =
                        unsafe { readstream(&raw mut stream, &raw mut len, &raw mut t, b, oid) };
                }
            }
        }
        if error == ErrorCode::PASSTHROUGH.raw() {
            error = 0;
        }
        if error < 0 && reads == 0 {
            return Err(error_unsupported_in_backend("read object streamed"));
        }
        ErrorCode::check(error)?;
        Ok(NonNull::new(stream).map(|s| (s, len, t)))
    }

    /// `git_odb_write_pack`.
    ///
    /// # Safety
    ///
    /// `progress_cb` may be called with `progress_payload`.
    pub(crate) unsafe fn write_pack(
        &self,
        progress_cb: IndexerProgressCb,
        progress_payload: *mut c_void,
    ) -> Result<*mut GitOdbWritepack> {
        let mut writes = 0usize;
        let mut error = ErrorCode::GENERIC.raw();
        let mut out: *mut GitOdbWritepack = core::ptr::null_mut();
        let me = core::ptr::from_ref(self).cast_mut();
        {
            let l = self.lock();
            for bi in &l.backends {
                if error >= 0 {
                    break;
                }
                if bi.is_alternate {
                    continue;
                }
                let b = bi.backend.as_ptr();
                // SAFETY: as in `exists_1`.
                if let Some(writepack) = unsafe { (*b).writepack } {
                    writes += 1;
                    // SAFETY: as in `exists_1`; the caller's callback.
                    error =
                        unsafe { writepack(&raw mut out, b, me, progress_cb, progress_payload) };
                }
            }
        }
        if error == ErrorCode::PASSTHROUGH.raw() {
            error = 0;
        }
        if error < 0 && writes == 0 {
            return Err(error_unsupported_in_backend("write pack"));
        }
        ErrorCode::check(error)?;
        Ok(out)
    }

    /// `git_odb_write_multi_pack_index`. C walked the backends without the
    /// lock; this walks a copy taken under it.
    pub(crate) fn write_multi_pack_index(&self) -> Result<()> {
        let mut writes = 0usize;
        let mut error = ErrorCode::GENERIC.raw();
        for bi in self.backends_snapshot()? {
            if error >= 0 {
                break;
            }
            if bi.is_alternate {
                continue;
            }
            let b = bi.backend.as_ptr();
            // SAFETY: as in `exists_1`.
            if let Some(writemidx) = unsafe { (*b).writemidx } {
                writes += 1;
                // SAFETY: as in `exists_1`.
                error = unsafe { writemidx(b) };
            }
        }
        if error == ErrorCode::PASSTHROUGH.raw() {
            error = 0;
        }
        if error < 0 && writes == 0 {
            return Err(error_unsupported_in_backend("write multi-pack-index"));
        }
        ErrorCode::check(error).map(drop)
    }

    /// `git_odb_refresh`.
    pub(crate) fn refresh(&self) -> Result<()> {
        let mut l = self.lock();
        for bi in &l.backends {
            let b = bi.backend.as_ptr();
            // SAFETY: as in `exists_1`.
            if let Some(refresh) = unsafe { (*b).refresh } {
                // SAFETY: as in `exists_1`.
                let error = unsafe { refresh(b) };
                if error < 0 {
                    return Err(ErrorCode::from_raw(error).unwrap_or(ErrorCode::GENERIC));
                }
            }
        }
        if let Some(g) = l.cgraph.as_mut() {
            g.refresh();
        }
        Ok(())
    }
}

/// A copy of the backend table at `b` (its version, owner and slots).
///
/// # Safety
///
/// `b` is a live backend table, whose fields nothing writes meanwhile
/// (libgit2 sets `odb` once, under the odb's lock).
#[inline]
unsafe fn slots(b: *const GitOdbBackend) -> GitOdbBackend {
    // SAFETY: the caller's contract; every field is plain data.
    unsafe { b.read() }
}

/// Free a backend through its `free` slot.
///
/// # Safety
///
/// `b` is a live backend nothing uses again.
pub(crate) unsafe fn free_backend(b: NonNull<GitOdbBackend>) {
    // SAFETY: the caller's contract.
    if let Some(free) = unsafe { (*b.as_ptr()).free } {
        // SAFETY: as above.
        unsafe { free(b.as_ptr()) };
    }
}

/// `stream->write(stream, data, len)`.
///
/// # Safety
///
/// `stream` is a live stream; `data` is readable for `len` bytes.
unsafe fn stream_write_slot(stream: *mut GitOdbStream, data: *const c_char, len: usize) -> c_int {
    // SAFETY: the caller's contract.
    match unsafe { (*stream).write } {
        // SAFETY: as above.
        Some(w) => unsafe { w(stream, data, len) },
        None => git_error_set!(ErrorClass::ODB, "stream does not support writing").raw(),
    }
}

/// `git_odb_stream_free`.
///
/// # Safety
///
/// `stream` is a live stream nothing uses again; its `hash_ctx` is NULL or
/// a `git__allocator` context.
pub unsafe fn stream_free(stream: *mut GitOdbStream) {
    // SAFETY: the caller's contract.
    let ctx = unsafe { (*stream).hash_ctx };
    // SAFETY: the caller's contract; `git_hash_ctx_cleanup` does nothing.
    unsafe { alloc::free(ctx) };
    // SAFETY: as above.
    if let Some(free) = unsafe { (*stream).free } {
        // SAFETY: as above.
        unsafe { free(stream) };
    }
}

/// The odb whose last reference was dropped, freed (`git_odb_free`).
///
/// # Safety
///
/// `db` is a live odb the caller gives a reference to.
pub unsafe fn odb_decref(db: NonNull<GitOdb>) {
    // SAFETY: the caller's contract.
    if unsafe { db.as_ref() }.rc.dec() {
        // SAFETY: the last reference, with no owner.
        unsafe { GitOdb::free(db) };
    }
}

// ---------------------------------------------------------------------------
// The fake write stream
// ---------------------------------------------------------------------------

/// `fake_wstream`: buffers what is written and hands it to the backend's
/// `write` at the end.
struct FakeWstream {
    buffer: Mutex<Vec<u8, crate::util::alloc::GitAlloc>>,
    size: usize,
    type_: git_object_t,
}

ffibox::impl_cvtable! {
    // SAFETY: libgit2 passes each of a stream's slots the stream, first,
    // and calls `free` on every stream (once); `read` is NULL (a write-only
    // stream), which C's contract allows.
    unsafe impl CVtable<FakeWstream> for StreamTable {}
    base = GitOdbStream { mode: GIT_STREAM_WRONLY, ..GitOdbStream::default() };
    /// `fake_wstream__write`.
    fn write(@stream, data: *const c_char, len: usize) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &FakeWstream| {
            let mut buf = s.buffer.lock().unwrap_or_else(PoisonError::into_inner);
            if buf.len().checked_add(len).is_none_or(|n| n > s.size) {
                return errors::assert_failed("stream->written + len <= stream->size").raw();
            }
            // SAFETY: C's contract: `data` is readable for `len` bytes.
            buf.extend_from_slice(unsafe { crate::util::export::bytes(data, len) });
            0
        },
    }
    /// `fake_wstream__fwrite`.
    fn finalize_write(@stream, oid: *const Oid) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &FakeWstream| {
            let mut buf = s.buffer.lock().unwrap_or_else(PoisonError::into_inner);
            // C hands the backend `size` bytes, whatever was written (the
            // rest indeterminate: zeroes here; the capacity is there).
            if buf.len() < s.size {
                buf.resize(s.size, 0);
            }
            // SAFETY: `stream` is this live table (the slot's argument);
            // its backend is a live table with a `write` (checked when the
            // stream was made).
            let b = unsafe { (*stream).backend };
            // SAFETY: as above.
            match unsafe { (*b).write } {
                // SAFETY: as above; the buffer holds `size` bytes.
                Some(w) => unsafe { w(b, oid, buf.as_ptr().cast(), s.size, s.type_) },
                None => -1,
            }
        },
    }
    release free;
}

/// `init_fake_wstream`.
fn fake_wstream_new(
    backend: *mut GitOdbBackend,
    size: git_object_size_t,
    type_: git_object_t,
) -> Result<NonNull<GitOdbStream>> {
    let blobsize = usize::try_from(size).map_err(|_| {
        git_error_set!(
            ErrorClass::NOMEMORY,
            "blob contents too large to fit in memory"
        )
    })?;
    let mut buffer = Vec::new_in(GIT_ALLOC);
    buffer
        .try_reserve_exact(blobsize)
        .map_err(|_| errors::oom())?;
    let t = ffibox::CTable::<StreamTable, FakeWstream>::new(FakeWstream {
        buffer: Mutex::new(buffer),
        size: blobsize,
        type_,
    });
    let p = t.into_c();
    // SAFETY: the fresh table's field, before C sees it.
    unsafe { (*p.as_ptr()).backend = backend };
    Ok(p)
}

/// The C ABI of `odb.c`.
pub mod ffi;

/// The safe Rust API.
mod api;
pub use api::*;

crate::c_layout_test! {
    headers: ["odb.h", "git2/sys/odb_backend.h", "git2/odb_backend.h"],
    types: [
        GitOdbObject => "git_odb_object" { cached, buffer },
        GitOdbOptions => "git_odb_options" { version, oid_type },
        GitOdbBackendPackOptions => "git_odb_backend_pack_options" { version, oid_type },
        GitOdbBackendLooseOptions => "git_odb_backend_loose_options" { version, flags, compression_level, dir_mode, file_mode, oid_type },
        GitOdbBackend => "git_odb_backend" {
            version, odb, read, read_prefix, read_header, write, writestream, readstream,
            exists, exists_prefix, refresh, foreach, writepack, writemidx, freshen, free
        },
        GitOdbStream => "git_odb_stream" {
            backend, mode, hash_ctx, oid_type, declared_size, received_bytes, read, write, finalize_write, free
        },
        GitOdbWritepack => "git_odb_writepack" { backend, append, commit, free },
        GitOdbExpandId => "git_odb_expand_id" { id, length, type_ as "type" },
    ],
    asserts: [
        "offsetof(git_odb, rc) == 0",
        "offsetof(git_odb, options) == 16",
        "sizeof(((git_odb *)0)->options) == 8",
        "GIT_OBJECT_DIR_MODE == 0777",
        "GIT_OBJECT_FILE_MODE == 0444",
        "GIT_ODB_BACKEND_VERSION == 1",
        "GIT_STREAM_WRONLY == 4",
        "GIT_STREAM_RDONLY == 2",
        "GIT_ODB_LOOKUP_NO_REFRESH == 1",
    ],
}
