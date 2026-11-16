//! Port of `src/libgit2/object.c`: the common header of commits, trees,
//! blobs and tags (`git_object`), lookup through the repository's object
//! cache and odb, the per-type parse/free table, peeling, path lookup, short
//! ids, object-id computation, and the object header helpers commit and tag
//! parsing use.
//!
//! # Types
//!
//! - [`GitObject`] is `struct git_object` (kind 3, mirrored): C embeds it by
//!   value as the first member of `git_commit` and `git_tree` (which C
//!   allocates: `commit_list.c`, the tests), and reads `obj->cached` and
//!   `obj->repo`. Its first member is [`GitCachedObj`], the cache's
//!   `git_cached_obj` (the cache module's type).
//! - An object is shared by every holder, on any thread, through the
//!   repository's cache: after it is published (stored in the cache, or
//!   returned by [`from_raw`]) only `cached.refcount` (an atomic) changes;
//!   everything else is read only. So a `&GitObject` is sound while the
//!   holder's reference lives.
//! - [`ObjectType`] is `git_object_t` for the safe API; C's values are the
//!   `GIT_OBJECT_*` constants.
//!
//! # The object table
//!
//! `git_objects_table` becomes [`Kind`]: commit and tree are the commit and
//! tree modules' (`git_commit__parse` & co., through their exports, and
//! their types' sizes), blob and tag are this crate's ([`crate::blob`],
//! [`crate::tag`]), allocated with `git__allocator` and freed through
//! [`free_object`].
//!
//! # Lifetimes
//!
//! A reference is a count in `cached.refcount`: `git_object_free` gives one
//! back through the cache's `git_cached_obj_decref`, which
//! frees the object with [`ffi::git_object__free`] when the count reaches
//! zero. The safe [`Object`] holds one reference and borrows its
//! repository.

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::marker::PhantomData;
use core::mem::size_of;
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::oid::{self, GIT_OID_MAX_HEXSIZE, GIT_OID_MINPREFIXLEN, Oid, OidType};
use crate::sys::blob_filter::{self as cfilter, git_filter_list};
pub(crate) mod deps;

use crate::util::alloc;
use crate::util::errors::{self, Bytes, Error, ErrorClass, ErrorCode, Result};
use crate::util::futils::{self, GIT_BUFSIZE_FILEIO};
use crate::util::hash::{self, HashAlgorithm, Hasher};
use crate::util::posix;
use crate::util::str::GitStr;
use crate::util::thread::Atomic32;
use crate::{git_assert, git_assert_arg, git_error_set};
use deps::{self as p, git_odb, git_odb_object, git_repository};

// ---------------------------------------------------------------------------
// git_object_t
// ---------------------------------------------------------------------------

/// `git_object_t`.
#[allow(non_camel_case_types)]
pub type git_object_t = c_int;

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

/// The safe API's object type (the odb's, `crate::odb::ObjectType`).
pub use crate::odb::ObjectType;

// ---------------------------------------------------------------------------
// git_cached_obj and git_object
// ---------------------------------------------------------------------------

/// `GIT_CACHE_STORE_ANY`.
pub const GIT_CACHE_STORE_ANY: u16 = 0;
/// `GIT_CACHE_STORE_RAW`.
pub const GIT_CACHE_STORE_RAW: u16 = 1;
/// `GIT_CACHE_STORE_PARSED`.
pub const GIT_CACHE_STORE_PARSED: u16 = 2;

/// `git_cached_obj` (`cache.h`): the cache's type, which `git_object`
/// embeds.
pub use crate::cache::GitCachedObj;

/// `struct git_object`, mirrored.
#[repr(C)]
#[derive(Debug)]
pub struct GitObject {
    /// The cache header.
    pub cached: GitCachedObj,
    /// The owning repository (NULL for an object parsed from raw data).
    pub repo: *mut git_repository,
}

/// `git_object`.
#[allow(non_camel_case_types)]
pub type git_object = GitObject;

impl GitObject {
    /// A header with nothing set (C's `git__calloc`).
    pub const fn zeroed() -> GitObject {
        GitObject {
            cached: GitCachedObj {
                oid: Oid::NONE,
                type_: 0,
                flags: 0,
                size: 0,
                refcount: Atomic32::new(0),
            },
            repo: ptr::null_mut(),
        }
    }

    /// The object's id.
    pub fn id(&self) -> &Oid {
        &self.cached.oid
    }

    /// Its `git_object_t`.
    pub fn raw_kind(&self) -> git_object_t {
        git_object_t::from(self.cached.type_)
    }
}

/// `sizeof(git_commit)`: allocated here, parsed and freed by the commit
/// module.
const COMMIT_SIZE: usize = size_of::<crate::commit::GitCommit>();
/// `sizeof(git_tree)`: likewise, by the tree module.
const TREE_SIZE: usize = size_of::<crate::tree::GitTree>();

// ---------------------------------------------------------------------------
// Strict object creation
// ---------------------------------------------------------------------------

/// `git_object__strict_input_validation` (`GIT_OPT_ENABLE_STRICT_OBJECT_CREATION`):
/// C's `bool`, which `settings.c` writes. An `AtomicBool` has `bool`'s
/// layout.
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub static git_object__strict_input_validation: AtomicBool = AtomicBool::new(true);

/// Whether strict object creation is on.
pub fn strict_input_validation() -> bool {
    git_object__strict_input_validation.load(Ordering::Relaxed)
}

/// Turn strict object creation on or off.
pub fn set_strict_input_validation(on: bool) {
    git_object__strict_input_validation.store(on, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// The repository's fields
// ---------------------------------------------------------------------------

/// `repo->oid_type`.
///
/// # Safety
///
/// `repo` is a live repository.
#[inline]
pub unsafe fn repo_oid_type(repo: *mut git_repository) -> crate::oid::git_oid_t {
    // SAFETY: a live repository (`Sync`).
    unsafe { &*repo }.oid_type_raw()
}

/// `&repo->objects`.
///
/// # Safety
///
/// `repo` is a live repository.
#[inline]
pub unsafe fn repo_objects(repo: *mut git_repository) -> *mut p::git_cache {
    // SAFETY: a live repository (`Sync`); the cache is `Sync` and reached
    // through `&` (the pointer is only passed to the cache's functions).
    ptr::from_ref(unsafe { &*repo }.objects()).cast_mut()
}

/// `git_repository_odb__weakptr`: the repository's odb, owned by it.
///
/// # Safety
///
/// `repo` is a live repository.
pub unsafe fn repo_odb_weak(repo: *mut git_repository) -> Result<*mut git_odb> {
    // SAFETY: a live repository (`Sync`).
    unsafe { &*repo }.odb_weak().map(NonNull::as_ptr)
}

// ---------------------------------------------------------------------------
// The object table
// ---------------------------------------------------------------------------

/// `git_object_type2string`.
pub fn type2string(t: git_object_t) -> &'static CStr {
    match t {
        GIT_OBJECT_COMMIT => c"commit",
        GIT_OBJECT_TREE => c"tree",
        GIT_OBJECT_BLOB => c"blob",
        GIT_OBJECT_TAG => c"tag",
        _ => c"",
    }
}

/// `git_object__size`: the size of the object structure (0 for an invalid
/// type).
fn object_size(t: git_object_t) -> usize {
    match t {
        GIT_OBJECT_COMMIT => COMMIT_SIZE,
        GIT_OBJECT_TREE => TREE_SIZE,
        GIT_OBJECT_BLOB => size_of::<crate::blob::GitBlob>(),
        GIT_OBJECT_TAG => size_of::<crate::tag::GitTag>(),
        _ => 0,
    }
}

/// `git_object_type_is_valid`.
pub fn type_is_valid(t: git_object_t) -> bool {
    object_size(t) > 0
}

/// `git_object_stringn2type` (on the first `len` bytes; C's prefix match).
pub fn stringn2type(s: &[u8]) -> git_object_t {
    if s.is_empty() || s[0] == 0 {
        return GIT_OBJECT_INVALID;
    }
    for t in [
        GIT_OBJECT_COMMIT,
        GIT_OBJECT_TREE,
        GIT_OBJECT_BLOB,
        GIT_OBJECT_TAG,
    ] {
        if crate::util::util::prefixncmp(s, s.len(), type2string(t).to_bytes()) == 0 {
            return t;
        }
    }
    GIT_OBJECT_INVALID
}

/// Allocate a zeroed object of type `t` (a valid one), its header's type
/// set.
fn alloc_object(t: git_object_t) -> Result<NonNull<GitObject>> {
    let obj = match t {
        GIT_OBJECT_COMMIT | GIT_OBJECT_TREE => {
            NonNull::new(alloc::calloc(1, object_size(t)).cast::<GitObject>())
                .ok_or(ErrorCode::GENERIC)?
        }
        GIT_OBJECT_BLOB => crate::blob::alloc()?.cast(),
        GIT_OBJECT_TAG => crate::tag::alloc()?.cast(),
        _ => return Err(ErrorCode::GENERIC),
    };
    // SAFETY: a fresh, initialized object header no one else sees.
    unsafe { (*obj.as_ptr()).cached.type_ = t as i16 };
    Ok(obj)
}

/// `def->parse`: parse `odb_obj` into the fresh object `obj`.
///
/// # Safety
///
/// `obj` is a fresh object of its type, not yet shared; `odb_obj` a live
/// odb object of that type.
unsafe fn parse_odb(
    obj: NonNull<GitObject>,
    odb_obj: *mut git_odb_object,
    oid_type: crate::oid::git_oid_t,
) -> c_int {
    // SAFETY: a fresh, unshared object.
    let t = unsafe { obj.as_ref() }.raw_kind();
    let o = obj.as_ptr().cast::<c_void>();
    match t {
        // SAFETY: a zeroed `git_commit` / `git_tree` of C's size, and a
        // live odb object (the caller's contract).
        GIT_OBJECT_COMMIT => unsafe { p::git_commit__parse(o, odb_obj, oid_type) },
        // SAFETY: as above.
        GIT_OBJECT_TREE => unsafe { p::git_tree__parse(o, odb_obj, oid_type) },
        // SAFETY: the caller's contract.
        GIT_OBJECT_BLOB => unsafe { crate::blob::parse(obj.cast(), odb_obj) },
        // SAFETY: as above.
        GIT_OBJECT_TAG => unsafe { crate::tag::parse_odb(obj.cast(), odb_obj, oid_type) },
        _ => errors::assert_failed("def->free && def->parse").raw(),
    }
}

/// `def->parse_raw`.
///
/// # Safety
///
/// `obj` is a fresh object of its type; `data` is readable for `size`
/// bytes and outlives the object (a blob points into it).
unsafe fn parse_raw(
    obj: NonNull<GitObject>,
    data: *const c_char,
    size: usize,
    oid_type: crate::oid::git_oid_t,
) -> c_int {
    // SAFETY: a fresh, unshared object.
    let t = unsafe { obj.as_ref() }.raw_kind();
    let o = obj.as_ptr().cast::<c_void>();
    match t {
        // SAFETY: as in `parse_odb`, and the caller's readable data.
        GIT_OBJECT_COMMIT => unsafe { p::git_commit__parse_raw(o, data, size, oid_type) },
        // SAFETY: as above.
        GIT_OBJECT_TREE => unsafe { p::git_tree__parse_raw(o, data, size, oid_type) },
        GIT_OBJECT_BLOB => {
            // SAFETY: the caller's contract.
            unsafe { crate::blob::parse_raw(obj.cast(), data.cast(), size) };
            0
        }
        GIT_OBJECT_TAG => {
            // SAFETY: the caller's readable data.
            let d = unsafe { crate::util::export::bytes(data, size) };
            // SAFETY: a fresh tag.
            unsafe { crate::tag::parse_into(obj.cast(), d, oid_type) }
        }
        _ => errors::assert_failed("def->free && def->parse_raw").raw(),
    }
}

/// `git_object__free`: free `obj` with its type's `free` (or `git__free`).
///
/// # Safety
///
/// `obj` is an object of this module's making (or C's, of C's size), with
/// no reference left.
pub unsafe fn free_object(obj: *mut GitObject) {
    // SAFETY: a live object no one else holds.
    let t = git_object_t::from(unsafe { (*obj).cached.type_ });
    let o = obj.cast::<c_void>();
    match t {
        // SAFETY: a commit / tree of C's, freed by C.
        GIT_OBJECT_COMMIT => unsafe { p::git_commit__free(o) },
        // SAFETY: as above.
        GIT_OBJECT_TREE => unsafe { p::git_tree__free(o) },
        GIT_OBJECT_BLOB | GIT_OBJECT_TAG => {
            // A live object (the caller's contract) is never NULL.
            let Some(nn) = NonNull::new(obj) else { return };
            if t == GIT_OBJECT_BLOB {
                // SAFETY: a blob of this crate's, freed once.
                unsafe { crate::blob::free(nn.cast()) }
            } else {
                // SAFETY: a tag of this crate's, freed once.
                unsafe { crate::tag::free(nn.cast()) }
            }
        }
        // SAFETY: anything else is a plain `git__allocator` block.
        _ => unsafe { alloc::free(o) },
    }
}

// ---------------------------------------------------------------------------
// Creating objects
// ---------------------------------------------------------------------------

/// `git_object__from_raw`: parse `data` as an object of type `t`, not
/// owned by a repository (and not cached), with one reference. The object
/// may point into `data` (a blob does).
///
/// # Safety
///
/// `data` is readable for `size` bytes and outlives the object.
pub unsafe fn from_raw(
    data: *const c_char,
    size: usize,
    t: git_object_t,
    oid_type: crate::oid::git_oid_t,
) -> Result<NonNull<GitObject>> {
    if !matches!(
        t,
        GIT_OBJECT_BLOB | GIT_OBJECT_TREE | GIT_OBJECT_COMMIT | GIT_OBJECT_TAG
    ) {
        let _ = git_error_set!(ErrorClass::INVALID, "the requested type is invalid");
        return Err(ErrorCode::NOT_FOUND);
    }
    let obj = alloc_object(t)?;
    // SAFETY: a fresh, unshared object.
    unsafe { (*obj.as_ptr()).cached.flags = GIT_CACHE_STORE_PARSED };
    let opts = GitObjectIdOptions {
        version: GIT_OBJECT_ID_OPTIONS_VERSION,
        object_type: t,
        oid_type,
        filters: ptr::null_mut(),
    };
    let d = if data.is_null() {
        None
    } else {
        // SAFETY: the caller's readable data.
        Some(unsafe { crate::util::export::bytes(data, size) })
    };
    // SAFETY: options without filters.
    let id = match unsafe { id_from_buffer_c(d, size, &raw const opts) } {
        Ok(id) => id,
        Err(e) => {
            // (C leaks the object here: rust/DEFECTS.md.)
            // SAFETY: the fresh object, freed once.
            unsafe { free_object(obj.as_ptr()) };
            return Err(e);
        }
    };
    // SAFETY: as above.
    unsafe { (*obj.as_ptr()).cached.oid = id };
    // SAFETY: a fresh object; the caller's data.
    let r = unsafe { parse_raw(obj, data, size, oid_type) };
    if r < 0 {
        // SAFETY: the fresh object, freed once.
        unsafe { free_object(obj.as_ptr()) };
        return Err(ErrorCode::from_raw(r).unwrap_or(ErrorCode::GENERIC));
    }
    // SAFETY: the object is live; `git_cached_obj_incref`.
    unsafe { obj.as_ref() }.cached.refcount.inc();
    Ok(obj)
}

/// The type, size and id of an odb object (its `git_cached_obj` prefix).
///
/// # Safety
///
/// `odb_obj` is a live odb object.
#[inline]
unsafe fn odb_header(odb_obj: *mut git_odb_object) -> (Oid, git_object_t, usize) {
    let c = odb_obj.cast::<GitCachedObj>();
    // SAFETY: a live `git_odb_object` starts with a `git_cached_obj`
    // (`GitOdbObject.cached`) whose id, type and size are written only
    // before it is shared.
    let oid = unsafe { (*c).oid };
    // SAFETY: as above.
    let kind = unsafe { (*c).type_ };
    // SAFETY: as above.
    let size = unsafe { (*c).size };
    (oid, git_object_t::from(kind), size)
}

/// `git_object__init_from_odb_object`: a fresh object for `odb_obj`
/// (unparsed).
///
/// # Safety
///
/// `repo` is a live repository and `odb_obj` a live odb object.
pub unsafe fn init_from_odb_object(
    repo: *mut git_repository,
    odb_obj: *mut git_odb_object,
    t: git_object_t,
) -> Result<NonNull<GitObject>> {
    // SAFETY: a live odb object.
    let (oid, otype, osize) = unsafe { odb_header(odb_obj) };
    if t != GIT_OBJECT_ANY && t != otype {
        let _ = git_error_set!(
            ErrorClass::INVALID,
            "the requested type does not match the type in the ODB"
        );
        return Err(ErrorCode::NOT_FOUND);
    }
    if object_size(otype) == 0 {
        let _ = git_error_set!(ErrorClass::INVALID, "the requested type is invalid");
        return Err(ErrorCode::NOT_FOUND);
    }
    let obj = alloc_object(otype)?;
    // SAFETY: a fresh, unshared object.
    let o = unsafe { &mut *obj.as_ptr() };
    o.cached.oid = oid;
    o.cached.size = osize;
    o.repo = repo;
    Ok(obj)
}

/// `git_object__from_odb_object`: parse `odb_obj` and store the object in
/// the repository's cache; the cached object, with a new reference.
///
/// # Safety
///
/// As [`init_from_odb_object`].
pub unsafe fn from_odb_object(
    repo: *mut git_repository,
    odb_obj: *mut git_odb_object,
    t: git_object_t,
) -> Result<NonNull<GitObject>> {
    // SAFETY: the caller's contract.
    let obj = unsafe { init_from_odb_object(repo, odb_obj, t)? };
    // SAFETY: a live repository.
    let oid_type = unsafe { repo_oid_type(repo) };
    // SAFETY: a fresh object of the odb object's type.
    if unsafe { parse_odb(obj, odb_obj, oid_type) } < 0 {
        // Parse errors (`GIT_EINVALID`) become -1.
        // SAFETY: the fresh object, freed once.
        unsafe { free_object(obj.as_ptr()) };
        return Err(ErrorCode::GENERIC);
    }
    // SAFETY: a live repository.
    let cache = unsafe { repo_objects(repo) };
    // SAFETY: a live repository's cache and a parsed object, whose
    // reference passes to the cache, which returns one (non-NULL).
    let stored = unsafe { p::git_cache_store_parsed(cache, obj.as_ptr().cast()) };
    NonNull::new(stored.cast::<GitObject>()).ok_or(ErrorCode::GENERIC)
}

/// `git_object_free`: give back one reference.
///
/// # Safety
///
/// `obj` is NULL or an object the caller holds a reference to, which this
/// call consumes.
#[inline]
pub unsafe fn release(obj: *mut GitObject) {
    if !obj.is_null() {
        // SAFETY: the caller's reference, given back to the cache, which
        // frees the object with the last one.
        unsafe { p::git_cached_obj_decref(obj.cast()) }
    }
}

/// `git_object_dup`: take one more reference.
///
/// # Safety
///
/// `obj` is a live object.
#[inline]
pub unsafe fn add_ref(obj: NonNull<GitObject>) {
    // SAFETY: a live object; the count is atomic.
    unsafe { obj.as_ref() }.cached.refcount.inc();
}

// ---------------------------------------------------------------------------
// Lookup
// ---------------------------------------------------------------------------

/// A held odb object, freed on drop.
struct OdbObject(*mut git_odb_object);

impl Drop for OdbObject {
    fn drop(&mut self) {
        // SAFETY: the reference this value holds (or NULL), given back
        // once.
        unsafe { p::git_odb_object_free(self.0) }
    }
}

/// `git_object_lookup_prefix`: the object whose id starts with the first
/// `len` hex digits of `id` (all of them: the cache is searched first),
/// of type `t` (or any), with a new reference.
///
/// # Safety
///
/// `repo` is a live repository.
pub unsafe fn lookup_prefix(
    repo: *mut git_repository,
    id: &Oid,
    len: usize,
    t: git_object_t,
) -> Result<NonNull<GitObject>> {
    if len < GIT_OID_MINPREFIXLEN {
        let _ = git_error_set!(
            ErrorClass::OBJECT,
            "ambiguous lookup - OID prefix is too short"
        );
        return Err(ErrorCode::AMBIGUOUS);
    }
    // SAFETY: a live repository.
    let odb = unsafe { repo_odb_weak(repo)? };
    // SAFETY: as above.
    let oid_type = unsafe { repo_oid_type(repo) };
    let hexsize = oid::oid_hexsize(oid_type);
    let len = len.min(hexsize);
    let mut odb_obj = ptr::null_mut();
    let r = if len == hexsize {
        // A full id: the cache first.
        // SAFETY: a live repository.
        let cache = unsafe { repo_objects(repo) };
        // SAFETY: a live repository's cache and a valid id.
        let cached = unsafe { p::git_cache_get_any(cache, id) }.cast::<GitCachedObj>();
        if !cached.is_null() {
            // SAFETY: a cached object, with the reference the cache just
            // gave; its flags are set before it is shared.
            let flags = unsafe { (*cached).flags };
            if flags == GIT_CACHE_STORE_PARSED {
                let obj = cached.cast::<GitObject>();
                // SAFETY: a live object.
                let kind = git_object_t::from(unsafe { (*obj).cached.type_ });
                if t != GIT_OBJECT_ANY && t != kind {
                    // SAFETY: the reference the cache gave.
                    unsafe { release(obj) };
                    let _ = git_error_set!(
                        ErrorClass::INVALID,
                        "the requested type does not match the type in the ODB"
                    );
                    return Err(ErrorCode::NOT_FOUND);
                }
                // SAFETY: non-null.
                return Ok(unsafe { NonNull::new_unchecked(obj) });
            } else if flags == GIT_CACHE_STORE_RAW {
                odb_obj = cached.cast::<git_odb_object>();
                0
            } else {
                // (C leaks the reference here: rust/DEFECTS.md.)
                // SAFETY: the reference the cache gave.
                unsafe { p::git_cached_obj_decref(cached.cast()) };
                return Err(errors::assert_failed(
                    "!\"Wrong caching type in the global object cache\"",
                ));
            }
        } else {
            // SAFETY: a live odb, a valid id, a writable out-parameter.
            unsafe { p::git_odb_read(&raw mut odb_obj, odb, id) }
        }
    } else {
        let short = Oid::from_raw_parts(oid_type as u8, *id.prefix(len).raw_id());
        // SAFETY: as above.
        unsafe { p::git_odb_read_prefix(&raw mut odb_obj, odb, &raw const short, len) }
    };
    ErrorCode::check(r)?;
    git_assert!(!odb_obj.is_null(), "odb_obj");
    let odb_obj = OdbObject(odb_obj);
    // SAFETY: a live repository and odb object.
    unsafe { from_odb_object(repo, odb_obj.0, t) }
}

/// `git_object_lookup`.
///
/// # Safety
///
/// As [`lookup_prefix`].
pub unsafe fn lookup(
    repo: *mut git_repository,
    id: &Oid,
    t: git_object_t,
) -> Result<NonNull<GitObject>> {
    // SAFETY: a live repository.
    let len = oid::oid_hexsize(unsafe { repo_oid_type(repo) });
    // SAFETY: forwarded.
    unsafe { lookup_prefix(repo, id, len, t) }
}

// ---------------------------------------------------------------------------
// Peeling
// ---------------------------------------------------------------------------

/// `dereference_object`: what `obj` points to, one step.
///
/// # Safety
///
/// `obj` is a live object of a repository.
unsafe fn dereference(obj: NonNull<GitObject>) -> core::result::Result<NonNull<GitObject>, c_int> {
    // SAFETY: a live object.
    match unsafe { obj.as_ref() }.raw_kind() {
        GIT_OBJECT_COMMIT => {
            let mut out = ptr::null_mut();
            // SAFETY: a live commit; a writable out-parameter.
            let r = unsafe { p::git_commit_tree(&raw mut out, obj.as_ptr().cast()) };
            if r != 0 {
                return Err(r);
            }
            NonNull::new(out.cast::<GitObject>()).ok_or(-1)
        }
        // SAFETY: a live tag.
        GIT_OBJECT_TAG => unsafe { crate::tag::target(obj.cast()) }.map_err(ErrorCode::raw),
        GIT_OBJECT_BLOB | GIT_OBJECT_TREE => Err(ErrorCode::PEEL.raw()),
        _ => Err(ErrorCode::INVALID_SPEC.raw()),
    }
}

/// `peel_error`.
fn peel_error(error: c_int, oid: &Oid, t: git_object_t) -> ErrorCode {
    let mut hex = [0u8; GIT_OID_MAX_HEXSIZE + 1];
    let _ = oid.nfmt(&mut hex);
    let n = memchr::memchr(0, &hex).unwrap_or(hex.len());
    let _ = git_error_set!(
        ErrorClass::OBJECT,
        "the git_object of id '{}' can not be successfully peeled into a {} (git_object_t={}).",
        Bytes(&hex[..n]),
        type2string(t).to_str().unwrap_or(""),
        t
    );
    ErrorCode::from_raw(error).unwrap_or(ErrorCode::GENERIC)
}

/// `check_type_combination`.
fn check_type_combination(t: git_object_t, target: git_object_t) -> c_int {
    if t == target {
        return 0;
    }
    match t {
        GIT_OBJECT_BLOB | GIT_OBJECT_TREE => ErrorCode::INVALID_SPEC.raw(),
        GIT_OBJECT_COMMIT if target != GIT_OBJECT_TREE && target != GIT_OBJECT_ANY => {
            ErrorCode::INVALID_SPEC.raw()
        }
        GIT_OBJECT_COMMIT | GIT_OBJECT_TAG => 0,
        _ => ErrorCode::INVALID_SPEC.raw(),
    }
}

/// `git_object_peel`: follow tags and commits until an object of type
/// `target` (or, for `GIT_OBJECT_ANY`, of another type than `object`'s),
/// with a new reference.
///
/// # Safety
///
/// `object` is a live object of a repository.
pub unsafe fn peel(object: NonNull<GitObject>, target: git_object_t) -> Result<NonNull<GitObject>> {
    git_assert_arg!(
        matches!(
            target,
            GIT_OBJECT_TAG | GIT_OBJECT_COMMIT | GIT_OBJECT_TREE | GIT_OBJECT_BLOB | GIT_OBJECT_ANY
        ),
        "target_type == GIT_OBJECT_TAG || target_type == GIT_OBJECT_COMMIT || target_type == GIT_OBJECT_TREE || target_type == GIT_OBJECT_BLOB || target_type == GIT_OBJECT_ANY"
    );
    let (otype, oid) = {
        // SAFETY: a live object, its header read only.
        let o = unsafe { object.as_ref() };
        (o.raw_kind(), o.cached.oid)
    };
    let e = check_type_combination(otype, target);
    if e < 0 {
        return Err(peel_error(e, &oid, target));
    }
    if otype == target {
        // SAFETY: a live object.
        unsafe { add_ref(object) };
        return Ok(object);
    }
    let mut source = object;
    let error = loop {
        // SAFETY: `source` is `object` or an object this loop holds.
        match unsafe { dereference(source) } {
            Ok(deref) => {
                if source != object {
                    // SAFETY: the reference this loop took.
                    unsafe { release(source.as_ptr()) };
                }
                // SAFETY: a live object (the reference just taken).
                let dt = unsafe { deref.as_ref() }.raw_kind();
                if dt == target || (target == GIT_OBJECT_ANY && dt != otype) {
                    return Ok(deref);
                }
                source = deref;
            }
            Err(e) => break e,
        }
    };
    if source != object {
        // SAFETY: the reference this loop took.
        unsafe { release(source.as_ptr()) };
    }
    Err(peel_error(error, &oid, target))
}

/// `git_object_lookup_bypath`: the object at `path` in the tree that
/// `treeish` peels to, of type `t` (or any).
///
/// # Safety
///
/// `treeish` is a live object of a repository; `path` is NUL-terminated.
pub unsafe fn lookup_bypath(
    treeish: NonNull<GitObject>,
    path: &CStr,
    t: git_object_t,
) -> Result<NonNull<GitObject>> {
    /// A held tree entry, freed on drop.
    struct Entry(*mut p::git_tree_entry);
    impl Drop for Entry {
        fn drop(&mut self) {
            // SAFETY: the entry this value holds (or NULL), freed once.
            unsafe { p::git_tree_entry_free(self.0) }
        }
    }
    /// A held object reference, given back on drop.
    struct Held(NonNull<GitObject>);
    impl Drop for Held {
        fn drop(&mut self) {
            // SAFETY: the reference this value holds.
            unsafe { release(self.0.as_ptr()) }
        }
    }

    // SAFETY: the caller's contract.
    let tree = Held(unsafe { peel(treeish, GIT_OBJECT_TREE)? });
    let mut entry = Entry(ptr::null_mut());
    // SAFETY: a live tree, a NUL-terminated path, a writable out.
    let r = unsafe {
        p::git_tree_entry_bypath(&raw mut entry.0, tree.0.as_ptr().cast(), path.as_ptr())
    };
    ErrorCode::check(r)?;
    // SAFETY: a live entry.
    if t != GIT_OBJECT_ANY && unsafe { p::git_tree_entry_type(entry.0) } != t {
        let _ = git_error_set!(
            ErrorClass::OBJECT,
            "object at path '{}' is not of the asked-for type {}",
            Bytes(path.to_bytes()),
            t
        );
        return Err(ErrorCode::INVALID_SPEC);
    }
    let mut out = ptr::null_mut();
    // SAFETY: a live object; `repo` is read only.
    let repo = unsafe { treeish.as_ref() }.repo;
    // SAFETY: the treeish's repository (live while it is), a live entry.
    let r = unsafe { p::git_tree_entry_to_object(&raw mut out, repo, entry.0) };
    ErrorCode::check(r)?;
    NonNull::new(out.cast::<GitObject>()).ok_or(ErrorCode::GENERIC)
}

/// `git_object__short_id`: the shortest unambiguous abbreviation of `obj`'s
/// id (at least `core.abbrev` digits), into `out`.
///
/// # Safety
///
/// `obj` is a live object of a repository.
pub unsafe fn short_id(out: &mut GitStr, obj: NonNull<GitObject>) -> Result<()> {
    // SAFETY: a live object.
    let o = unsafe { obj.as_ref() };
    let repo = o.repo;
    // SAFETY: the object's live repository.
    let oid_type = unsafe { repo_oid_type(repo) };
    let mut id = Oid::from_raw_parts(oid_type as u8, [0; oid::GIT_OID_MAX_SIZE]);
    let hexsize = oid::oid_hexsize(oid_type);
    let mut len: c_int = 0;
    // SAFETY: a live repository, a writable out.
    ErrorCode::check(unsafe { p::git_repository__abbrev_length(&raw mut len, repo) })?;
    if len as usize == hexsize {
        // `git_oid_cpy` copies the type too.
        id = o.cached.oid;
    }
    let mut odb = ptr::null_mut();
    // SAFETY: as above; the odb comes with a reference.
    ErrorCode::check(unsafe { p::git_repository_odb(&raw mut odb, repo) })?;
    let mut error = 0;
    while (len as usize) < hexsize {
        let l = len as usize;
        let n = l.div_ceil(2);
        let src = *o.cached.oid.raw_id();
        let dst = id.raw_id_mut();
        dst[..n].copy_from_slice(&src[..n]);
        if l & 1 == 1 {
            dst[l / 2] &= 0xf0;
        }
        // SAFETY: a live odb, a valid id.
        error = unsafe { p::git_odb_exists_prefix(ptr::null_mut(), odb, &raw const id, l) };
        if error != ErrorCode::AMBIGUOUS.raw() {
            break;
        }
        errors::clear();
        len += 1;
    }
    let r = if error != 0 {
        Err(ErrorCode::from_raw(error).unwrap_or(ErrorCode::GENERIC))
    } else {
        let len = len as usize;
        out.grow(len + 1).map(|()| {
            // `git_oid_tostr(out->ptr, len + 1, &id); out->size = len`.
            let hex = id.hex();
            let digits = &hex.as_bytes()[..len.min(hex.as_bytes().len())];
            out.clear();
            let _ = out.put(digits);
        })
    };
    // SAFETY: the reference `git_repository_odb` gave.
    unsafe { p::git_odb_free(odb) };
    r
}

/// `git_object__is_valid`: with strict object creation, whether `id` names
/// an object of type `expected` (or any) in the repository's odb.
///
/// # Safety
///
/// `repo` is a live repository.
pub unsafe fn is_valid(repo: *mut git_repository, id: &Oid, expected: git_object_t) -> bool {
    if !strict_input_validation() {
        return true;
    }
    // SAFETY: a live repository.
    let Ok(odb) = (unsafe { repo_odb_weak(repo) }) else {
        return false;
    };
    let mut len = 0usize;
    let mut actual: c_int = 0;
    // SAFETY: a live odb, valid outputs.
    if unsafe { p::git_odb_read_header(&raw mut len, &raw mut actual, odb, id) } < 0 {
        return false;
    }
    if expected != GIT_OBJECT_ANY && expected != actual {
        let _ = git_error_set!(
            ErrorClass::INVALID,
            "the requested type does not match the type in the ODB"
        );
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// Object ids
// ---------------------------------------------------------------------------

/// `git_object_id_options`, mirrored.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GitObjectIdOptions {
    /// `GIT_OBJECT_ID_OPTIONS_VERSION`.
    pub version: c_uint,
    /// The object type (0: blob).
    pub object_type: git_object_t,
    /// The oid type (0: SHA-1).
    pub oid_type: crate::oid::git_oid_t,
    /// Filters to apply to blob contents first.
    pub filters: *mut git_filter_list,
}

/// `GIT_OBJECT_ID_OPTIONS_VERSION`.
pub const GIT_OBJECT_ID_OPTIONS_VERSION: c_uint = 1;

/// The normalized options.
#[derive(Clone, Copy, Debug)]
struct IdOptions {
    object_type: git_object_t,
    oid_type: crate::oid::git_oid_t,
    filters: *mut git_filter_list,
}

/// `normalize_options`.
///
/// # Safety
///
/// `given` is NULL or valid.
unsafe fn normalize_options(given: *const GitObjectIdOptions) -> Result<IdOptions> {
    // SAFETY: NULL or valid.
    let g = unsafe { given.as_ref() }.copied();
    let object_type = match g {
        Some(g) if g.object_type != 0 => g.object_type,
        _ => GIT_OBJECT_BLOB,
    };
    let oid_type = match g {
        Some(g) if g.oid_type != 0 => g.oid_type,
        _ => oid::GIT_OID_SHA1,
    };
    if !type_is_valid(object_type) {
        return Err(git_error_set!(ErrorClass::INVALID, "invalid object type"));
    }
    if OidType::from_raw(oid_type).is_none() {
        return Err(git_error_set!(ErrorClass::INVALID, "unknown oid type"));
    }
    Ok(IdOptions {
        object_type,
        oid_type,
        filters: g.map_or(ptr::null_mut(), |g| g.filters),
    })
}

/// `needs_filter`.
///
/// # Safety
///
/// `opts.filters` is NULL or a live filter list.
unsafe fn needs_filter(opts: &IdOptions) -> bool {
    // SAFETY: the caller's contract.
    !opts.filters.is_null()
        && unsafe { cfilter::list_length(opts.filters) } > 0
        && (opts.object_type == 0 || opts.object_type == GIT_OBJECT_BLOB)
}

/// `git_odb__format_object_header`: `"<type> <len>\0"`; the length
/// written, the NUL included.
pub fn format_object_header(out: &mut [u8; 64], len: u64, t: git_object_t) -> Result<usize> {
    use std::io::Write as _;
    let mut cur = std::io::Cursor::new(&mut out[..63]);
    // `%s %"PRId64`, which must fit with its NUL in the 64 bytes.
    if write!(
        cur,
        "{} {}",
        type2string(t).to_str().unwrap_or(""),
        len as i64
    )
    .is_err()
    {
        return Err(git_error_set!(
            ErrorClass::OS,
            "object header creation failed"
        ));
    }
    let n = cur.position() as usize;
    out[n] = 0;
    Ok(n + 1)
}

fn algorithm(oid_type: crate::oid::git_oid_t) -> HashAlgorithm {
    if oid_type == oid::GIT_OID_SHA256 {
        HashAlgorithm::Sha256
    } else {
        HashAlgorithm::Sha1
    }
}

fn oid_from_digest(oid_type: crate::oid::git_oid_t, d: &hash::Digest) -> Oid {
    let mut id = [0u8; oid::GIT_OID_MAX_SIZE];
    let b = d.as_bytes();
    id[..b.len()].copy_from_slice(b);
    Oid::from_raw_parts(oid_type as u8, id)
}

/// `id_from_buffer`: the id of `data` as an object (no filters).
fn id_from_buffer_opts(data: &[u8], opts: &IdOptions) -> Result<Oid> {
    let mut header = [0u8; 64];
    let n = format_object_header(&mut header, data.len() as u64, opts.object_type)?;
    let d = hash::hash_vec(&[&header[..n], data], algorithm(opts.oid_type))?;
    Ok(oid_from_digest(opts.oid_type, &d))
}

/// The id of `data` as an object of type `t` (`git_object_id_from_buffer`
/// without filters).
pub fn id_from_buffer(data: &[u8], t: ObjectType, oid_type: OidType) -> Result<Oid> {
    // SAFETY: options on the stack, without filters.
    let opts = unsafe {
        normalize_options(&GitObjectIdOptions {
            version: GIT_OBJECT_ID_OPTIONS_VERSION,
            object_type: t.raw(),
            oid_type: oid_type.raw(),
            filters: ptr::null_mut(),
        })
    }?;
    id_from_buffer_opts(data, &opts)
}

/// `git_object_id_from_buffer` with C's options.
///
/// # Safety
///
/// `given` is NULL or valid, with a live filter list or none.
unsafe fn id_from_buffer_c(
    data: Option<&[u8]>,
    len: usize,
    given: *const GitObjectIdOptions,
) -> Result<Oid> {
    // SAFETY: the caller's contract.
    let opts = unsafe { normalize_options(given)? };
    let mut filtered = GitStr::new();
    let mut data = data;
    let mut len = len;
    // SAFETY: the caller's contract on the filters.
    if unsafe { needs_filter(&opts) } {
        // SAFETY: a live list.
        unsafe { cfilter::apply_to_buffer(&mut filtered, opts.filters, data.unwrap_or(&[]))? };
        data = Some(filtered.as_bytes());
        len = filtered.len();
    }
    match data {
        None if len != 0 => Err(git_error_set!(ErrorClass::INVALID, "invalid object")),
        _ => id_from_buffer_opts(data.unwrap_or(&[]), &opts),
    }
}

/// `id_from_fd`: hash `size` bytes of `fd` as an object (no filters).
fn id_from_fd_raw(fd: c_int, mut size: usize, opts: &IdOptions) -> Result<Oid> {
    let mut h = Hasher::new(algorithm(opts.oid_type))?;
    let mut header = [0u8; 64];
    let n = format_object_header(&mut header, size as u64, opts.object_type)?;
    h.update(&header[..n])?;
    let mut buffer = vec![0u8; GIT_BUFSIZE_FILEIO];
    let mut read_failed = false;
    while size > 0 {
        match posix::read(fd, &mut buffer) {
            Ok(0) => break,
            Ok(r) => {
                if r > size {
                    return Err(git_error_set!(
                        ErrorClass::OS,
                        "error reading file for hashing"
                    ));
                }
                h.update(&buffer[..r])?;
                size -= r;
            }
            Err(posix::Errno(e)) => {
                errors::set_errno(e);
                read_failed = true;
                break;
            }
        }
    }
    // A failed read, or a file truncated since it was stat'd.
    if read_failed || size > 0 {
        return Err(git_error_set!(
            ErrorClass::OS,
            "error reading file for hashing"
        ));
    }
    let d = h.finalize()?;
    Ok(oid_from_digest(opts.oid_type, &d))
}

/// `git_object_id_from_fd`.
///
/// # Safety
///
/// `given` is NULL or valid, with a live filter list or none.
unsafe fn id_from_fd_c(fd: c_int, size: usize, given: *const GitObjectIdOptions) -> Result<Oid> {
    // SAFETY: the caller's contract.
    let opts = unsafe { normalize_options(given)? };
    // SAFETY: as above.
    if !unsafe { needs_filter(&opts) } {
        return id_from_fd_raw(fd, size, &opts);
    }
    // The size goes into the header: read it all, then filter.
    let mut raw = GitStr::new();
    futils::readbuffer_fd(&mut raw, fd, size)?;
    let o = GitObjectIdOptions {
        version: GIT_OBJECT_ID_OPTIONS_VERSION,
        object_type: opts.object_type,
        oid_type: opts.oid_type,
        filters: opts.filters,
    };
    // SAFETY: the normalized options, with the caller's filters.
    unsafe { id_from_buffer_c(Some(raw.as_bytes()), raw.len(), &raw const o) }
}

/// `git_object_id_from_file`.
///
/// # Safety
///
/// As [`id_from_fd_c`].
unsafe fn id_from_file_c(path: &CStr, given: *const GitObjectIdOptions) -> Result<Oid> {
    // SAFETY: the caller's contract.
    let opts = unsafe { normalize_options(given)? };
    let fd = futils::open_ro(path)?;
    let r = (|| {
        let size = futils::filesize(fd)?;
        let Ok(size) = usize::try_from(size) else {
            return Err(git_error_set!(
                ErrorClass::OS,
                "file size overflow for 32-bit systems"
            ));
        };
        let o = GitObjectIdOptions {
            version: GIT_OBJECT_ID_OPTIONS_VERSION,
            object_type: opts.object_type,
            oid_type: opts.oid_type,
            filters: opts.filters,
        };
        // SAFETY: the normalized options.
        unsafe { id_from_fd_c(fd, size, &raw const o) }
    })();
    // SAFETY: the descriptor opened above, closed once.
    let _ = unsafe { posix::close(fd) };
    r
}

/// `git_object_id_from_symlink`.
///
/// # Safety
///
/// As [`id_from_fd_c`].
unsafe fn id_from_symlink_c(path: &CStr, given: *const GitObjectIdOptions) -> Result<Oid> {
    // SAFETY: the caller's contract.
    let mut opts = unsafe { normalize_options(given)? };
    opts.filters = ptr::null_mut();
    if opts.object_type != GIT_OBJECT_BLOB {
        return Err(git_error_set!(
            ErrorClass::INVALID,
            "symbolic links must be blob types"
        ));
    }
    let st = crate::util::fs_path::lstat(path)?;
    let o = GitObjectIdOptions {
        version: GIT_OBJECT_ID_OPTIONS_VERSION,
        object_type: opts.object_type,
        oid_type: opts.oid_type,
        filters: ptr::null_mut(),
    };
    if !posix::s_islnk(st.st_mode) {
        // SAFETY: options without filters.
        return unsafe { id_from_file_c(path, &raw const o) };
    }
    if st.st_size < 0 || st.st_size > i64::from(c_int::MAX) {
        return Err(git_error_set!(
            ErrorClass::FILESYSTEM,
            "file size overflow for 32-bit systems"
        ));
    }
    let link_len = st.st_size as usize;
    let mut link = Vec::new();
    link.try_reserve_exact(link_len + 1)
        .map_err(|_| errors::oom())?;
    link.resize(link_len + 1, 0);
    let read = match posix::readlink(path, &mut link[..link_len]) {
        Ok(n) => n,
        Err(posix::Errno(e)) => {
            errors::set_errno(e);
            return Err(git_error_set!(
                ErrorClass::OS,
                "failed to read symlink data for '{}'",
                Bytes(path.to_bytes())
            ));
        }
    };
    git_assert!(read <= link_len, "read_len <= st.st_size");
    id_from_buffer_opts(&link[..read], &opts)
}

/// `git_object_rawcontent_is_valid` for Rust: whether `buf` parses as an
/// object of type `t` (blobs always do).
pub fn rawcontent_is_valid(buf: &[u8], t: ObjectType, oid_type: OidType) -> Result<bool> {
    if t == ObjectType::Blob {
        return Ok(true);
    }
    // SAFETY: `buf` outlives the object, which is released before return.
    match unsafe { from_raw(buf.as_ptr().cast(), buf.len(), t.raw(), oid_type.raw()) } {
        Ok(o) => {
            // SAFETY: the reference `from_raw` gave.
            unsafe { release(o.as_ptr()) };
            Ok(true)
        }
        Err(e) if e == ErrorCode::INVALID => Ok(false),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Object header helpers (commit and tag parsing)
// ---------------------------------------------------------------------------

/// `git_object__parse_oid_header`: `<header><hex id>\n` at the start of
/// `buf`; the id and the bytes consumed. -1 (with no message, except an
/// invalid id's) when it is not there.
pub fn parse_oid_header(buf: &[u8], header: &[u8], oid_type: OidType) -> Result<(Oid, usize)> {
    let sha_len = oid_type.hexsize();
    let h = header.len();
    if h + sha_len + 1 > buf.len() || &buf[..h] != header || buf[h + sha_len] != b'\n' {
        return Err(ErrorCode::GENERIC);
    }
    let id = Oid::from_prefix(&buf[h..h + sha_len], oid_type).map_err(|_| ErrorCode::GENERIC)?;
    Ok((id, h + sha_len + 1))
}

/// `git_object__write_oid_header`: append `<header><hex id>\n`.
pub fn write_oid_header(buf: &mut GitStr, header: &[u8], id: &Oid) -> Result<()> {
    let hex_size = id.hexsize();
    if hex_size == 0 {
        return Err(git_error_set!(ErrorClass::INVALID, "unknown type"));
    }
    let hex = id.hex();
    let _ = buf.puts(header);
    let _ = buf.put(&hex.as_bytes()[..hex_size]);
    let _ = buf.putc(b'\n');
    if buf.is_oom() {
        Err(ErrorCode::GENERIC)
    } else {
        Ok(())
    }
}

/// `git_str_printf(buf, fmt, …)` of text already formatted into `bytes`
/// (which need not be UTF-8), with printf's growth: room for `2 *
/// format_len` more bytes first, then exactly what is missing.
pub(crate) fn printf_bytes(buf: &mut GitStr, format_len: usize, bytes: &[u8]) -> Result<()> {
    buf.vprintf_begin(format_len)?;
    let Ok(len) = c_int::try_from(bytes.len()) else {
        // `vsnprintf` fails beyond INT_MAX.
        // SAFETY: -1 is never written.
        unsafe { buf.vprintf_written(-1)? };
        return Err(ErrorCode::GENERIC);
    };
    loop {
        let size = buf.len();
        // The room after the contents, the NUL's slot included.
        let (spare, room) = buf.spare_ptr();
        if bytes.len() < room {
            // SAFETY: `room > len` bytes at `spare`, the string's own buffer
            // (borrowed through `&mut buf`), disjoint from `bytes`.
            unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), spare, bytes.len()) };
            // SAFETY: the byte after them, within `room`.
            unsafe { spare.wrapping_add(bytes.len()).write(0) };
            // SAFETY: the string owns its buffer (`room > 0`), the bytes up
            // to `size + len` are written and the NUL follows them.
            unsafe { buf.set_len(size + bytes.len()) };
            return Ok(());
        }
        // SAFETY: they do not fit, so this only grows the buffer.
        if unsafe { buf.vprintf_written(len)? } {
            return Ok(());
        }
    }
}

// ---------------------------------------------------------------------------
// The safe API
// ---------------------------------------------------------------------------

/// The repository of the safe API.
pub use crate::repository::Repository;

/// The `git_repository *` of `repo`, live while it is.
pub(crate) fn repo_ptr(repo: &Repository) -> *mut git_repository {
    repo.raw_ptr().cast()
}

/// An object of a repository (`git_object *`), one reference of it; freed
/// (given back) on drop, duplicated by `Clone`.
pub struct Object<'repo> {
    raw: NonNull<GitObject>,
    _repo: PhantomData<&'repo Repository>,
}

impl<'repo> Object<'repo> {
    /// Adopt a reference to an object of `_repo`.
    ///
    /// # Safety
    ///
    /// `raw` is a live object of the repository, whose reference passes to
    /// the result.
    pub(crate) unsafe fn from_raw(
        _repo: &'repo Repository,
        raw: NonNull<GitObject>,
    ) -> Object<'repo> {
        Object {
            raw,
            _repo: PhantomData,
        }
    }

    /// Adopt a reference to an object of the repository an `other` of the
    /// same lifetime belongs to.
    ///
    /// # Safety
    ///
    /// As [`from_raw`](Self::from_raw).
    pub(crate) unsafe fn sibling(&self, raw: NonNull<GitObject>) -> Object<'repo> {
        Object {
            raw,
            _repo: PhantomData,
        }
    }

    /// `git_object_lookup`.
    pub fn lookup(
        repo: &'repo Repository,
        id: &Oid,
        kind: ObjectType,
    ) -> core::result::Result<Object<'repo>, Error> {
        // SAFETY: a live repository.
        let raw = unsafe { lookup(repo_ptr(repo), id, kind.raw())? };
        // SAFETY: an object of `repo`, with its reference.
        Ok(unsafe { Object::from_raw(repo, raw) })
    }

    /// `git_object_lookup_prefix`: by the first `len` hex digits of `id`.
    pub fn lookup_prefix(
        repo: &'repo Repository,
        id: &Oid,
        len: usize,
        kind: ObjectType,
    ) -> core::result::Result<Object<'repo>, Error> {
        // SAFETY: a live repository.
        let raw = unsafe { lookup_prefix(repo_ptr(repo), id, len, kind.raw())? };
        // SAFETY: as above.
        Ok(unsafe { Object::from_raw(repo, raw) })
    }

    pub(crate) fn header(&self) -> &GitObject {
        // SAFETY: a live object whose header is read only (module docs).
        unsafe { self.raw.as_ref() }
    }

    /// `git_object_id`.
    pub fn id(&self) -> &Oid {
        &self.header().cached.oid
    }

    /// `git_object_type`.
    pub fn kind(&self) -> ObjectType {
        ObjectType::from_raw(self.header().raw_kind()).unwrap_or(ObjectType::Any)
    }

    /// The C handle, live while `self` is.
    pub(crate) fn as_ptr(&self) -> NonNull<GitObject> {
        self.raw
    }

    /// `git_object_peel`.
    pub fn peel(&self, kind: ObjectType) -> core::result::Result<Object<'repo>, Error> {
        // SAFETY: a live object of a repository.
        let raw = unsafe { peel(self.raw, kind.raw())? };
        // SAFETY: an object of the same repository, with its reference.
        Ok(unsafe { self.sibling(raw) })
    }

    /// `git_object_short_id`.
    pub fn short_id(&self) -> core::result::Result<Vec<u8>, Error> {
        let mut s = GitStr::new();
        // SAFETY: a live object of a repository.
        unsafe { short_id(&mut s, self.raw)? };
        Ok(s.as_bytes().to_vec())
    }

    /// `git_object_lookup_bypath`.
    pub fn lookup_bypath(
        &self,
        path: &[u8],
        kind: ObjectType,
    ) -> core::result::Result<Object<'repo>, Error> {
        let c = std::ffi::CString::new(path).map_err(|_| {
            Error::from(git_error_set!(
                ErrorClass::INVALID,
                "path contains a NUL byte"
            ))
        })?;
        // SAFETY: a live object of a repository, a NUL-terminated path.
        let raw = unsafe { lookup_bypath(self.raw, &c, kind.raw())? };
        // SAFETY: an object of the same repository.
        Ok(unsafe { self.sibling(raw) })
    }

    /// This object as a blob, if it is one.
    pub fn into_blob(self) -> core::result::Result<crate::blob::Blob<'repo>, Object<'repo>> {
        crate::blob::Blob::try_from_object(self)
    }

    /// This object as a tag, if it is one.
    pub fn into_tag(self) -> core::result::Result<crate::tag::Tag<'repo>, Object<'repo>> {
        crate::tag::Tag::try_from_object(self)
    }
}

impl Clone for Object<'_> {
    /// `git_object_dup`.
    fn clone(&self) -> Self {
        // SAFETY: a live object.
        unsafe { add_ref(self.raw) };
        Object {
            raw: self.raw,
            _repo: PhantomData,
        }
    }
}

impl Drop for Object<'_> {
    fn drop(&mut self) {
        // SAFETY: the reference this value holds.
        unsafe { release(self.raw.as_ptr()) }
    }
}

impl core::fmt::Debug for Object<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Object")
            .field("kind", &self.kind())
            .field("id", &self.id().hex().as_str())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `object.c`.
pub mod ffi {
    use super::*;
    use crate::buf::GitBuf;
    use crate::util::export::{barrier, bytes, cstr, status};

    /// Write an object result to C's out-parameter.
    ///
    /// # Safety
    ///
    /// `out` is writable.
    #[inline]
    unsafe fn give(out: *mut *mut GitObject, r: Result<NonNull<GitObject>>) -> c_int {
        match r {
            Ok(o) => {
                // SAFETY: the caller's contract.
                unsafe { out.write(o.as_ptr()) };
                0
            }
            Err(e) => e.raw(),
        }
    }

    /// `git_object__from_raw`.
    ///
    /// # Safety
    ///
    /// `object_out` is NULL or writable; `data` is readable for `size`
    /// bytes and outlives the object.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object__from_raw(
        object_out: *mut *mut GitObject,
        data: *const c_char,
        size: usize,
        object_type: git_object_t,
        oid_type: crate::oid::git_oid_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!object_out.is_null(), "object_out", -1);
            // SAFETY: writable.
            unsafe { object_out.write(ptr::null_mut()) };
            // SAFETY: the caller's contract on `data`.
            let r = unsafe { from_raw(data, size, object_type, oid_type) };
            // SAFETY: writable.
            unsafe { give(object_out, r) }
        })
    }

    /// `git_object__init_from_odb_object`.
    ///
    /// # Safety
    ///
    /// `object_out` is NULL or writable; `repo` a live repository and
    /// `odb_obj` a live odb object.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object__init_from_odb_object(
        object_out: *mut *mut GitObject,
        repo: *mut git_repository,
        odb_obj: *mut git_odb_object,
        object_type: git_object_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!object_out.is_null(), "object_out", -1);
            // SAFETY: writable.
            unsafe { object_out.write(ptr::null_mut()) };
            // SAFETY: the caller's contract on `repo` and `odb_obj`.
            let r = unsafe { init_from_odb_object(repo, odb_obj, object_type) };
            // SAFETY: writable.
            unsafe { give(object_out, r) }
        })
    }

    /// `git_object__from_odb_object`.
    ///
    /// # Safety
    ///
    /// As [`git_object__init_from_odb_object`] (`object_out` writable).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object__from_odb_object(
        object_out: *mut *mut GitObject,
        repo: *mut git_repository,
        odb_obj: *mut git_odb_object,
        object_type: git_object_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!object_out.is_null(), "object_out", -1);
            // SAFETY: the caller's contract.
            match unsafe { from_odb_object(repo, odb_obj, object_type) } {
                Ok(o) => {
                    // SAFETY: writable.
                    unsafe { object_out.write(o.as_ptr()) };
                    0
                }
                // (`*object_out` is left untouched, as C.)
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_object__free`: free an object whose last reference is gone.
    ///
    /// # Safety
    ///
    /// `obj` is an object libgit2 made, with no reference left.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object__free(obj: *mut c_void) {
        barrier(|| {
            // SAFETY: the caller's contract.
            unsafe { free_object(obj.cast()) }
        })
    }

    /// `git_object_lookup_prefix`.
    ///
    /// # Safety
    ///
    /// `object_out` is NULL or writable; `repo` is NULL or a live
    /// repository; `id` is NULL or valid.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_lookup_prefix(
        object_out: *mut *mut GitObject,
        repo: *mut git_repository,
        id: *const Oid,
        len: usize,
        object_type: git_object_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!repo.is_null(), "repo", -1);
            git_assert_arg!(!object_out.is_null(), "object_out", -1);
            git_assert_arg!(!id.is_null(), "id", -1);
            // SAFETY: valid (oids are read only).
            let id = unsafe { &*id };
            // SAFETY: a live repository.
            let r = unsafe { lookup_prefix(repo, id, len, object_type) };
            // SAFETY: writable.
            unsafe { give(object_out, r) }
        })
    }

    /// `git_object_lookup`.
    ///
    /// # Safety
    ///
    /// As [`git_object_lookup_prefix`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_lookup(
        object_out: *mut *mut GitObject,
        repo: *mut git_repository,
        id: *const Oid,
        object_type: git_object_t,
    ) -> c_int {
        // (C reads `repo->oid_type` before checking `repo`.)
        let len = if repo.is_null() {
            0
        } else {
            // SAFETY: a live repository.
            oid::oid_hexsize(unsafe { repo_oid_type(repo) })
        };
        // SAFETY: forwarded.
        unsafe { git_object_lookup_prefix(object_out, repo, id, len, object_type) }
    }

    /// `git_object_free` (NULL allowed).
    ///
    /// # Safety
    ///
    /// `object` is NULL or an object the caller holds a reference to, not
    /// used again by the caller.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_free(object: *mut GitObject) {
        barrier(|| {
            // SAFETY: the caller's contract.
            unsafe { release(object) }
        })
    }

    /// `git_object_id`.
    ///
    /// # Safety
    ///
    /// `obj` is NULL or a live object.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_id(obj: *const GitObject) -> *const Oid {
        git_assert_arg!(!obj.is_null(), "obj", ptr::null());
        // SAFETY: a live object (no reference formed).
        unsafe { &raw const (*obj).cached.oid }
    }

    /// `git_object_type`.
    ///
    /// # Safety
    ///
    /// `obj` is NULL or a live object.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_type(obj: *const GitObject) -> git_object_t {
        git_assert_arg!(!obj.is_null(), "obj", GIT_OBJECT_INVALID);
        // SAFETY: a live object; its type is read only.
        git_object_t::from(unsafe { (*obj).cached.type_ })
    }

    /// `git_object_owner`.
    ///
    /// # Safety
    ///
    /// `obj` is NULL or a live object.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_owner(obj: *const GitObject) -> *mut git_repository {
        git_assert_arg!(!obj.is_null(), "obj", ptr::null_mut());
        // SAFETY: a live object; `repo` is read only.
        unsafe { (*obj).repo }
    }

    /// `git_object_type2string`.
    #[unsafe(no_mangle)]
    pub extern "C" fn git_object_type2string(t: git_object_t) -> *const c_char {
        type2string(t).as_ptr()
    }

    /// `git_object_string2type`.
    ///
    /// # Safety
    ///
    /// `s` is NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_string2type(s: *const c_char) -> git_object_t {
        // SAFETY: NULL or NUL-terminated.
        match unsafe { cstr(s) } {
            None => GIT_OBJECT_INVALID,
            Some(s) => stringn2type(s.to_bytes()),
        }
    }

    /// `git_object_stringn2type`.
    ///
    /// # Safety
    ///
    /// `s` is NULL or readable for `len` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_stringn2type(s: *const c_char, len: usize) -> git_object_t {
        barrier(|| {
            if s.is_null() {
                return GIT_OBJECT_INVALID;
            }
            // SAFETY: readable for `len` bytes.
            stringn2type(unsafe { bytes(s, len) })
        })
    }

    /// `git_object_type_is_valid`.
    #[unsafe(no_mangle)]
    pub extern "C" fn git_object_type_is_valid(t: git_object_t) -> c_int {
        c_int::from(type_is_valid(t))
    }

    /// `git_object_peel`.
    ///
    /// # Safety
    ///
    /// `peeled` is NULL or writable; `object` NULL or a live object of a
    /// repository.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_peel(
        peeled: *mut *mut GitObject,
        object: *const GitObject,
        target_type: git_object_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!object.is_null(), "object", -1);
            git_assert_arg!(!peeled.is_null(), "peeled", -1);
            // SAFETY: non-null.
            let o = unsafe { NonNull::new_unchecked(object.cast_mut()) };
            // SAFETY: a live object; `peeled` writable.
            match unsafe { peel(o, target_type) } {
                Ok(r) => {
                    // SAFETY: writable.
                    unsafe { peeled.write(r.as_ptr()) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_object_dup`.
    ///
    /// # Safety
    ///
    /// `dest` is writable and `source` a live object (C does not check
    /// either).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_dup(
        dest: *mut *mut GitObject,
        source: *mut GitObject,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!source.is_null(), "source", -1);
            // SAFETY: non-null (checked above).
            let nn = unsafe { NonNull::new_unchecked(source) };
            // SAFETY: a live object.
            unsafe { add_ref(nn) };
            // SAFETY: writable.
            unsafe { dest.write(source) };
            0
        })
    }

    /// `git_object_lookup_bypath`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `treeish` NULL or a live object of a
    /// repository; `path` NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_lookup_bypath(
        out: *mut *mut GitObject,
        treeish: *const GitObject,
        path: *const c_char,
        object_type: git_object_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null(), "out", -1);
            git_assert_arg!(!treeish.is_null(), "treeish", -1);
            git_assert_arg!(!path.is_null(), "path", -1);
            // SAFETY: non-null (checked above).
            let t = unsafe { NonNull::new_unchecked(treeish.cast_mut()) };
            // SAFETY: NUL-terminated (the caller's contract).
            let path = unsafe { CStr::from_ptr(path) };
            // SAFETY: a live object; `out` writable.
            match unsafe { lookup_bypath(t, path, object_type) } {
                Ok(r) => {
                    // SAFETY: writable.
                    unsafe { out.write(r.as_ptr()) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_object_short_id`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or a valid `git_buf`; `obj` NULL or a live object of a
    /// repository.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_short_id(out: *mut GitBuf, obj: *const GitObject) -> c_int {
        barrier(|| {
            let f = |s: &mut GitStr| {
                git_assert_arg!(!obj.is_null(), "obj");
                // SAFETY: non-null (checked above).
                let o = unsafe { NonNull::new_unchecked(obj.cast_mut()) };
                // SAFETY: a live object of a repository.
                unsafe { short_id(s, o) }
            };
            // SAFETY: the caller's contract on `out`.
            unsafe { buf_wrap(out, f) }
        })
    }

    /// `git_object__is_valid`.
    ///
    /// # Safety
    ///
    /// `repo` is a live repository; `id` valid.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object__is_valid(
        repo: *mut git_repository,
        id: *const Oid,
        expected_type: git_object_t,
    ) -> bool {
        barrier(|| {
            if !strict_input_validation() {
                return true;
            }
            // SAFETY: a valid id (the caller's contract).
            let id = unsafe { &*id };
            // SAFETY: a live repository.
            unsafe { is_valid(repo, id, expected_type) }
        })
    }

    /// `git_object_id_options_init`.
    ///
    /// # Safety
    ///
    /// `opts` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_id_options_init(
        opts: *mut GitObjectIdOptions,
        version: c_uint,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract.
            if let Err(e) = unsafe {
                check_version(
                    &raw const version,
                    GIT_OBJECT_ID_OPTIONS_VERSION,
                    "git_object_id_options",
                )
            } {
                return e.raw();
            }
            // SAFETY: writable.
            unsafe {
                opts.write(GitObjectIdOptions {
                    version: GIT_OBJECT_ID_OPTIONS_VERSION,
                    object_type: 0,
                    oid_type: 0,
                    filters: ptr::null_mut(),
                })
            };
            0
        })
    }

    /// `git_object_id_from_fd`.
    ///
    /// # Safety
    ///
    /// `id` is NULL or writable; `opts` NULL or valid (with a live filter
    /// list or none).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_id_from_fd(
        id: *mut Oid,
        fd: c_int,
        size: usize,
        opts: *mut GitObjectIdOptions,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!id.is_null(), "id", -1);
            // SAFETY: the caller's contract.
            write_id(id, unsafe { id_from_fd_c(fd, size, opts) })
        })
    }

    /// `git_object_id_from_symlink`.
    ///
    /// # Safety
    ///
    /// As [`git_object_id_from_fd`]; `path` NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_id_from_symlink(
        id: *mut Oid,
        path: *const c_char,
        opts: *const GitObjectIdOptions,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!id.is_null(), "id", -1);
            git_assert_arg!(!path.is_null(), "path", -1);
            // SAFETY: NUL-terminated.
            let path = unsafe { CStr::from_ptr(path) };
            // SAFETY: the caller's contract.
            write_id(id, unsafe { id_from_symlink_c(path, opts) })
        })
    }

    /// `git_object_id_from_file`.
    ///
    /// # Safety
    ///
    /// As [`git_object_id_from_symlink`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_id_from_file(
        id: *mut Oid,
        path: *const c_char,
        opts: *const GitObjectIdOptions,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!id.is_null(), "id", -1);
            git_assert_arg!(!path.is_null(), "path", -1);
            // SAFETY: NUL-terminated.
            let path = unsafe { CStr::from_ptr(path) };
            // SAFETY: the caller's contract.
            write_id(id, unsafe { id_from_file_c(path, opts) })
        })
    }

    /// `git_object_id_from_buffer`.
    ///
    /// # Safety
    ///
    /// `id` is NULL or writable; `data` NULL or readable for `len` bytes;
    /// `opts` as [`git_object_id_from_fd`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_id_from_buffer(
        id: *mut Oid,
        data: *const c_void,
        len: usize,
        opts: *const GitObjectIdOptions,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!id.is_null(), "id", -1);
            let d = if data.is_null() {
                None
            } else {
                // SAFETY: readable for `len` bytes.
                Some(unsafe { bytes(data.cast(), len) })
            };
            // SAFETY: the caller's contract.
            write_id(id, unsafe { id_from_buffer_c(d, len, opts) })
        })
    }

    fn write_id(id: *mut Oid, r: Result<Oid>) -> c_int {
        match r {
            Ok(v) => {
                // SAFETY: the exports check `id` and require it writable.
                unsafe { id.write(v) };
                0
            }
            Err(e) => e.raw(),
        }
    }

    /// `git_object_rawcontent_is_valid`.
    ///
    /// # Safety
    ///
    /// `valid` is NULL or writable; `buf` NULL or readable for `len` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_rawcontent_is_valid(
        valid: *mut c_int,
        buf: *const c_char,
        len: usize,
        object_type: git_object_t,
        oid_type: crate::oid::git_oid_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!valid.is_null(), "valid", -1);
            git_assert_arg!(!buf.is_null(), "buf", -1);
            if object_type == GIT_OBJECT_BLOB {
                // SAFETY: writable.
                unsafe { valid.write(1) };
                return 0;
            }
            // SAFETY: the buffer outlives the object, released at once.
            match unsafe { from_raw(buf, len, object_type, oid_type) } {
                Ok(o) => {
                    // SAFETY: the reference `from_raw` gave.
                    unsafe { release(o.as_ptr()) };
                    // SAFETY: writable.
                    unsafe { valid.write(1) };
                    0
                }
                Err(e) if e == ErrorCode::INVALID => {
                    // SAFETY: writable.
                    unsafe { valid.write(0) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_object__parse_oid_header`.
    ///
    /// # Safety
    ///
    /// `oid` is writable; `*buffer_out` .. `buffer_end` readable; `header`
    /// NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object__parse_oid_header(
        oid: *mut Oid,
        buffer_out: *mut *const c_char,
        buffer_end: *const c_char,
        header: *const c_char,
        oid_type: crate::oid::git_oid_t,
    ) -> c_int {
        barrier(|| {
            // SAFETY: readable.
            let start = unsafe { *buffer_out };
            let len = (buffer_end as usize).saturating_sub(start as usize);
            // SAFETY: the caller's readable range.
            let buf = unsafe { bytes(start, len) };
            // SAFETY: NUL-terminated.
            let header = unsafe { CStr::from_ptr(header) }.to_bytes();
            let Some(t) = OidType::from_raw(oid_type) else {
                // An unknown type: C's checks with a hex size of 0, then
                // `git_oid_from_prefix`'s "unknown type".
                let h = header.len();
                if h + 1 > buf.len() || &buf[..h] != header || buf[h] != b'\n' {
                    return -1;
                }
                let _ = git_error_set!(ErrorClass::INVALID, "unable to parse OID - unknown type");
                return -1;
            };
            match parse_oid_header(buf, header, t) {
                Ok((id, n)) => {
                    // SAFETY: writable.
                    unsafe { oid.write(id) };
                    // SAFETY: writable; within the range.
                    unsafe { buffer_out.write(start.wrapping_add(n)) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_object__write_oid_header`.
    ///
    /// # Safety
    ///
    /// `buf` is a valid `git_str`; `header` NUL-terminated (not in `buf`);
    /// `oid` valid.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object__write_oid_header(
        buf: *mut GitStr,
        header: *const c_char,
        oid: *const Oid,
    ) -> c_int {
        barrier(|| {
            // SAFETY: valid, read by value.
            let id = unsafe { oid.read() };
            // SAFETY: NUL-terminated, copied before `buf` is touched.
            let header = unsafe { CStr::from_ptr(header) }.to_bytes().to_vec();
            // SAFETY: a valid `git_str`, not otherwise accessed.
            status(write_oid_header(unsafe { &mut *buf }, &header, &id))
        })
    }

    /// `GIT_BUF_WRAP_PRIVATE`: run `f` on a `git_str` that takes `out`'s
    /// buffer, and give the result back to `out`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or a valid `git_buf`.
    pub(crate) unsafe fn buf_wrap(
        out: *mut GitBuf,
        f: impl FnOnce(&mut GitStr) -> Result<()>,
    ) -> c_int {
        let mut s = core::mem::MaybeUninit::<GitStr>::uninit();
        // SAFETY: the caller's contract; `s` is writable.
        let mut error = unsafe { crate::buf::ffi::git_buf_tostr(s.as_mut_ptr(), out) };
        if error != 0 {
            return error;
        }
        // SAFETY: `git_buf_tostr` succeeded, so it initialized `s`.
        let mut s = unsafe { s.assume_init() };
        error = status(f(&mut s));
        if error == 0 {
            // SAFETY: valid `git_buf` and `git_str`.
            error = unsafe { crate::buf::ffi::git_buf_fromstr(out, &raw mut s) };
        }
        drop(s);
        error
    }

    /// `git_error__check_version` (`GIT_ERROR_CHECK_VERSION`).
    ///
    /// # Safety
    ///
    /// `structure` is NULL or points to a readable `unsigned int`.
    pub(crate) unsafe fn check_version(
        structure: *const c_uint,
        expected_max: c_uint,
        name: &str,
    ) -> Result<()> {
        if structure.is_null() {
            return Ok(());
        }
        // SAFETY: readable.
        let actual = unsafe { structure.read() };
        if actual > 0 && actual <= expected_max {
            return Ok(());
        }
        Err(git_error_set!(
            ErrorClass::INVALID,
            "invalid version {} on {}",
            actual as c_int,
            name
        ))
    }
}

crate::c_layout_test! {
    headers: ["object.h", "cache.h", "commit.h", "tree.h", "git2/object.h"],
    types: [
        GitCachedObj => "git_cached_obj" { oid, kind as "type", flags, size, refcount },
        GitObject => "git_object" { cached, repo },
        GitObjectIdOptions => "git_object_id_options" { version, object_type, oid_type, filters },
        crate::sys::blob_filter::git_filter_options => "git_filter_options" { version, flags, reserved, attr_commit_id },
    ],
    asserts: [
        "sizeof(git_commit) == 184",
        "sizeof(git_tree) == 96",
        "_Alignof(git_commit) <= 8 && _Alignof(git_tree) <= 8",
        "GIT_CACHE_STORE_RAW == 1 && GIT_CACHE_STORE_PARSED == 2",
        "GIT_OBJECT_ANY == -2 && GIT_OBJECT_INVALID == -1 && GIT_OBJECT_COMMIT == 1 && GIT_OBJECT_TREE == 2 && GIT_OBJECT_BLOB == 3 && GIT_OBJECT_TAG == 4",
        "sizeof(git_object__strict_input_validation) == 1",
    ],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support;

    #[test]
    fn types_and_strings() {
        assert_eq!(type2string(GIT_OBJECT_TAG), c"tag");
        assert_eq!(type2string(0), c"");
        assert_eq!(type2string(7), c"");
        assert_eq!(stringn2type(b"commitx"), GIT_OBJECT_COMMIT);
        assert_eq!(stringn2type(b"comm"), GIT_OBJECT_INVALID);
        assert_eq!(stringn2type(b"tree"), GIT_OBJECT_TREE);
        assert_eq!(stringn2type(b""), GIT_OBJECT_INVALID);
        assert_eq!(stringn2type(b"\0blob"), GIT_OBJECT_INVALID);
        assert!(type_is_valid(GIT_OBJECT_BLOB));
        assert!(!type_is_valid(GIT_OBJECT_OFS_DELTA));
        assert_eq!(ObjectType::Blob.name(), "blob");
    }

    #[test]
    fn ids_of_buffers() {
        let _g = test_support::init();
        let id = id_from_buffer(b"", ObjectType::Blob, OidType::Sha1).unwrap();
        assert_eq!(
            id.hex().as_str(),
            "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
        );
        let mut h = [0u8; 64];
        assert_eq!(
            format_object_header(&mut h, 12, GIT_OBJECT_TREE).unwrap(),
            8
        );
        assert_eq!(&h[..8], b"tree 12\0");
    }

    #[test]
    #[cfg_attr(miri, ignore = "SHA-256 is OpenSSL's")]
    fn sha256_ids() {
        let _g = test_support::init();
        let id = id_from_buffer(b"hello\n", ObjectType::Blob, OidType::Sha256).unwrap();
        assert_eq!(id.kind(), Some(OidType::Sha256));
    }

    #[test]
    fn oid_headers() {
        let _g = test_support::init();
        let b = b"tree 0123456789012345678901234567890123456789\nrest";
        let (id, n) = parse_oid_header(b, b"tree ", OidType::Sha1).unwrap();
        assert_eq!(n, 46);
        let mut s = GitStr::new();
        write_oid_header(&mut s, b"tree ", &id).unwrap();
        assert_eq!(s.as_bytes(), &b[..46]);
        assert!(parse_oid_header(&b[..45], b"tree ", OidType::Sha1).is_err());
        assert!(parse_oid_header(b, b"parent ", OidType::Sha1).is_err());
        assert!(write_oid_header(&mut s, b"x", &Oid::NONE).is_err());
    }

    #[test]
    fn printf_bytes_grows_like_printf() {
        let _g = test_support::init();
        let mut s = GitStr::new();
        printf_bytes(&mut s, 3, b"\xffabc").unwrap();
        assert_eq!(s.as_bytes(), b"\xffabc");
        let long = vec![b'x'; 1000];
        printf_bytes(&mut s, 3, &long).unwrap();
        assert_eq!(s.len(), 1004);
    }

    /// `git_object__from_raw` of a tag and a blob, read through the
    /// exports and freed with the table's free (no C involved).
    #[test]
    fn raw_objects_without_c() {
        let _g = test_support::init();
        let text = b"object e90810b8df3e80c413d903f631643c716887138d\ntype blob\ntag t\n\nm";
        let mut o: *mut GitObject = ptr::null_mut();
        // SAFETY: a readable buffer that outlives the object; a writable out.
        let rc = unsafe {
            ffi::git_object__from_raw(
                &raw mut o,
                text.as_ptr().cast(),
                text.len(),
                GIT_OBJECT_TAG,
                1,
            )
        };
        assert_eq!(rc, 0);
        let t = o.cast::<crate::tag::GitTag>();
        // SAFETY: a live tag.
        let name = unsafe { crate::tag::ffi::git_tag_name(t) };
        // SAFETY: its NUL-terminated name.
        let name = unsafe { CStr::from_ptr(name) };
        assert_eq!(name.to_bytes(), b"t");
        // SAFETY: a live tag.
        let kind = unsafe { crate::tag::ffi::git_tag_target_type(t) };
        assert_eq!(kind, GIT_OBJECT_BLOB);
        // SAFETY: a live tag.
        let tagger = unsafe { crate::tag::ffi::git_tag_tagger(t) };
        assert!(tagger.is_null());
        // SAFETY: a live object.
        let refs = unsafe { (*o).cached.refcount.get() };
        assert_eq!(refs, 1);
        // SAFETY: the only reference: freed as the cache would.
        unsafe { free_object(o) };

        let data = b"blob contents";
        // SAFETY: as above.
        let rc = unsafe {
            ffi::git_object__from_raw(
                &raw mut o,
                data.as_ptr().cast(),
                data.len(),
                GIT_OBJECT_BLOB,
                1,
            )
        };
        assert_eq!(rc, 0);
        let b = o.cast::<crate::blob::GitBlob>();
        // SAFETY: a live blob.
        let size = unsafe { crate::blob::ffi::git_blob_rawsize(b) };
        assert_eq!(size, data.len() as u64);
        // SAFETY: a live blob.
        let content = unsafe { crate::blob::ffi::git_blob_rawcontent(b) };
        assert_eq!(content, data.as_ptr().cast());
        // SAFETY: a live object.
        let id = unsafe { ffi::git_object_id(o) };
        // SAFETY: its id, live with it.
        let id = unsafe { *id };
        assert_eq!(
            id,
            id_from_buffer(data, ObjectType::Blob, OidType::Sha1).unwrap()
        );
        // SAFETY: the only reference.
        unsafe { free_object(o) };

        // An unknown oid type: C's message, nothing leaked.
        // SAFETY: as above.
        let rc = unsafe {
            ffi::git_object__from_raw(
                &raw mut o,
                data.as_ptr().cast(),
                data.len(),
                GIT_OBJECT_BLOB,
                9,
            )
        };
        assert_eq!(rc, -1);
        assert!(o.is_null());
        assert_eq!(errors::last_message().unwrap(), b"unknown oid type");
    }
}
