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

use crate::util::cabi::Out;
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
#[inline]
pub fn repo_oid_type(repo: &git_repository) -> crate::oid::git_oid_t {
    repo.oid_type_raw()
}

/// `&repo->objects`.
#[inline]
pub fn repo_objects(repo: &git_repository) -> &p::git_cache {
    repo.objects()
}

/// `git_repository_odb__weakptr`: the repository's odb (made on first
/// use), owned by it.
pub fn repo_odb_weak(repo: &git_repository) -> Result<&git_odb> {
    repo.odb()
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
    // SAFETY: `obj` is a fresh block of at least a header's size, aligned
    // for it (`git__allocator` memory), zeroed or initialized by its
    // module, and no one else sees it: a place write of one field.
    unsafe { (*obj.as_ptr()).cached.type_ = t as i16 };
    Ok(obj)
}

/// `def->parse`: parse `odb_obj` into the fresh object `obj`.
///
/// # Safety
///
/// `obj` is a fresh object of its type (from [`alloc_object`]), not yet
/// shared; `odb_obj` a live odb object of that type, which the caller
/// holds a reference to for the call (the parser may take its own).
unsafe fn parse_odb(
    obj: NonNull<GitObject>,
    odb_obj: NonNull<git_odb_object>,
    oid_type: crate::oid::git_oid_t,
) -> c_int {
    // SAFETY: `obj` is a fresh object no one else reaches (this fn's
    // contract): its header is initialized and unwritten during the `&`.
    let t = unsafe { obj.as_ref() }.raw_kind();
    let o = obj.as_ptr().cast::<c_void>();
    let odb_obj = odb_obj.as_ptr();
    match t {
        // SAFETY: `o` is a zeroed `git_commit` of C's size that only this
        // call holds, `odb_obj` a live odb object (this fn's contract).
        GIT_OBJECT_COMMIT => unsafe { p::git_commit__parse(o, odb_obj, oid_type) },
        // SAFETY: as above, for a zeroed `git_tree`.
        GIT_OBJECT_TREE => unsafe { p::git_tree__parse(o, odb_obj, oid_type) },
        // SAFETY: a fresh blob of `blob::alloc`'s and a live odb object
        // (this fn's contract).
        GIT_OBJECT_BLOB => unsafe { crate::blob::parse(obj.cast(), odb_obj) },
        // SAFETY: a fresh tag of `tag::alloc`'s and a live odb object.
        GIT_OBJECT_TAG => unsafe { crate::tag::parse_odb(obj.cast(), odb_obj, oid_type) },
        _ => errors::assert_failed("def->free && def->parse").raw(),
    }
}

/// `def->parse_raw`.
///
/// # Safety
///
/// `obj` is a fresh object of its type (from [`alloc_object`]), not yet
/// shared; `data` is NULL or readable for `size` bytes, which nothing
/// writes while the object lives (a blob points into them).
unsafe fn parse_raw(
    obj: NonNull<GitObject>,
    data: *const c_char,
    size: usize,
    oid_type: crate::oid::git_oid_t,
) -> c_int {
    // SAFETY: as in `parse_odb`: a fresh object no one else reaches.
    let t = unsafe { obj.as_ref() }.raw_kind();
    let o = obj.as_ptr().cast::<c_void>();
    match t {
        // SAFETY: a zeroed `git_commit` of C's size that only this call
        // holds; `data` readable for `size` bytes (this fn's contract).
        GIT_OBJECT_COMMIT => unsafe { p::git_commit__parse_raw(o, data, size, oid_type) },
        // SAFETY: as above, for a zeroed `git_tree`.
        GIT_OBJECT_TREE => unsafe { p::git_tree__parse_raw(o, data, size, oid_type) },
        GIT_OBJECT_BLOB => {
            // SAFETY: a fresh blob; `data` outlives it, unwritten (this
            // fn's contract).
            unsafe { crate::blob::parse_raw(obj.cast(), data.cast(), size) };
            0
        }
        GIT_OBJECT_TAG => {
            // SAFETY: `data` is NULL (an empty slice) or readable for
            // `size` bytes, unwritten during the call (this fn's contract).
            let d = unsafe { crate::util::export::bytes(data, size) };
            // SAFETY: a fresh tag of `tag::alloc`'s, only this call holds.
            unsafe { crate::tag::parse_into(obj.cast(), d, oid_type) }
        }
        _ => errors::assert_failed("def->free && def->parse_raw").raw(),
    }
}

/// `git_object__free`: free `obj` with its type's `free` (or `git__free`).
///
/// # Safety
///
/// `obj` is an object of this module's making (or C's, of C's size for
/// its type), with no reference left: nothing uses it again.
pub unsafe fn free_object(obj: *mut GitObject) {
    // SAFETY: `obj` is live and no one else holds it (this fn's contract):
    // a read of one header field in place.
    let t = git_object_t::from(unsafe { (*obj).cached.type_ });
    let o = obj.cast::<c_void>();
    match t {
        // SAFETY: a commit made by `alloc_object` or C, of C's size, not
        // used again: the commit module's `free`.
        GIT_OBJECT_COMMIT => unsafe { p::git_commit__free(o) },
        // SAFETY: as above, for a tree.
        GIT_OBJECT_TREE => unsafe { p::git_tree__free(o) },
        GIT_OBJECT_BLOB | GIT_OBJECT_TAG => {
            // A live object (this fn's contract) is never NULL.
            let Some(nn) = NonNull::new(obj) else { return };
            if t == GIT_OBJECT_BLOB {
                // SAFETY: a blob of `blob::alloc`'s (`alloc_object`), with
                // no reference left, freed once.
                unsafe { crate::blob::free(nn.cast()) }
            } else {
                // SAFETY: a tag of `tag::alloc`'s, likewise.
                unsafe { crate::tag::free(nn.cast()) }
            }
        }
        // SAFETY: an object of another type is a plain `git__allocator`
        // block (C's `git__free`), not used again.
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
/// `data` is NULL or readable for `size` bytes, which nothing writes while
/// the object lives.
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
    // SAFETY: `obj` is fresh and no one else reaches it: a place write.
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
        // SAFETY: a non-NULL `data` is readable for `size` bytes that
        // nothing writes meanwhile (this fn's contract).
        Some(unsafe { crate::util::export::bytes(data, size) })
    };
    let id = match normalize_options(Some(&opts)).and_then(|o| id_from_data(d, size, &o)) {
        Ok(id) => id,
        Err(e) => {
            // (C leaks the object here: rust/DEFECTS.md.)
            // SAFETY: the fresh object, which no one else holds, freed once.
            unsafe { free_object(obj.as_ptr()) };
            return Err(e);
        }
    };
    // SAFETY: as above: a place write to the fresh object.
    unsafe { (*obj.as_ptr()).cached.oid = id };
    // SAFETY: a fresh object of `alloc_object`'s; `data` as this fn's
    // contract says.
    let r = unsafe { parse_raw(obj, data, size, oid_type) };
    if r < 0 {
        // SAFETY: the fresh object, which no one else holds, freed once.
        unsafe { free_object(obj.as_ptr()) };
        return Err(ErrorCode::from_raw(r).unwrap_or(ErrorCode::GENERIC));
    }
    // SAFETY: the object is live and still ours alone; `git_cached_obj_incref`
    // (an atomic).
    unsafe { obj.as_ref() }.cached.refcount.inc();
    Ok(obj)
}

/// The type, size and id of an odb object (its `git_cached_obj` prefix).
///
/// # Safety
///
/// `odb_obj` is a live odb object.
#[inline]
unsafe fn odb_header(odb_obj: NonNull<git_odb_object>) -> (Oid, git_object_t, usize) {
    // SAFETY: a live odb object (this fn's contract), whose header (id,
    // type, size) is written only before it is shared, and is only read
    // here; the count, the one field others write, is atomic.
    let c = unsafe { &odb_obj.as_ref().cached };
    (c.oid, git_object_t::from(c.type_), c.size)
}

/// `git_object__init_from_odb_object`: a fresh object for `odb_obj`
/// (unparsed), owned by `repo` (stored, not read: NULL is allowed, as C).
///
/// # Safety
///
/// `odb_obj` is a live odb object.
pub unsafe fn init_from_odb_object(
    repo: *mut git_repository,
    odb_obj: NonNull<git_odb_object>,
    t: git_object_t,
) -> Result<NonNull<GitObject>> {
    // SAFETY: a live odb object (this fn's contract).
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
    // SAFETY: `obj` is fresh, its header initialized, and no one else
    // reaches it: this `&mut` is the only reference while it lives.
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
/// `odb_obj` is a live odb object, to which the caller holds a reference
/// for the call.
pub unsafe fn from_odb_object(
    repo: &git_repository,
    odb_obj: NonNull<git_odb_object>,
    t: git_object_t,
) -> Result<NonNull<GitObject>> {
    // The object keeps `repo` as C's `git_repository *` (the repository
    // is only reached through `&`).
    let repo_ptr = ptr::from_ref(repo).cast_mut();
    // SAFETY: a live odb object (this fn's contract).
    let obj = unsafe { init_from_odb_object(repo_ptr, odb_obj, t)? };
    // SAFETY: a fresh object of the odb object's type (`init_from_odb_object`
    // checked it), and the live odb object the caller holds.
    if unsafe { parse_odb(obj, odb_obj, repo.oid_type_raw()) } < 0 {
        // Parse errors (`GIT_EINVALID`) become -1.
        // SAFETY: the fresh object, which no one else holds, freed once.
        unsafe { free_object(obj.as_ptr()) };
        return Err(ErrorCode::GENERIC);
    }
    // SAFETY: a fresh, parsed `git_object` with a count of 0, private to
    // this call, used again only through the returned reference.
    let stored = unsafe { repo.objects().store_as(obj.cast(), GIT_CACHE_STORE_PARSED) };
    Ok(stored.cast())
}

/// `git_object_free`: give back one reference.
///
/// # Safety
///
/// `obj` is NULL or an object the caller holds a reference to, which this
/// call consumes.
#[inline]
pub unsafe fn release(obj: *mut GitObject) {
    if let Some(o) = NonNull::new(obj) {
        // SAFETY: a live object (a cached object stored parsed, or made by
        // `from_raw`, both with `GIT_CACHE_STORE_PARSED`), whose reference
        // the caller gives up; the last one frees it (`git_object__free`).
        unsafe { crate::cache::decref(o.cast()) }
    }
}

/// `git_object_dup`: take one more reference.
///
/// # Safety
///
/// `obj` is a live object.
#[inline]
pub unsafe fn add_ref(obj: NonNull<GitObject>) {
    // SAFETY: a live object (this fn's contract), whose header is not
    // written once shared but for the count, an atomic.
    unsafe { obj.as_ref() }.cached.refcount.inc();
}

// ---------------------------------------------------------------------------
// Lookup
// ---------------------------------------------------------------------------

/// A held odb object, freed on drop.
struct OdbObject(NonNull<git_odb_object>);

impl Drop for OdbObject {
    fn drop(&mut self) {
        // SAFETY: the live odb object whose reference this value holds
        // (from the cache or an odb read), given back once.
        unsafe { crate::cache::decref(self.0.cast()) }
    }
}

/// `git_object_lookup_prefix`: the object whose id starts with the first
/// `len` hex digits of `id` (all of them: the cache is searched first),
/// of type `t` (or any), with a new reference.
pub fn lookup_prefix(
    repo: &git_repository,
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
    let odb = repo.odb()?;
    let oid_type = repo.oid_type_raw();
    let hexsize = oid::oid_hexsize(oid_type);
    let len = len.min(hexsize);
    let odb_obj = if len == hexsize {
        // A full id: the cache first.
        match repo.objects().get(id, GIT_CACHE_STORE_ANY) {
            Some(cached) => {
                // SAFETY: a cached object, live with the reference the
                // cache just gave; its flags are set before it is shared.
                let flags = unsafe { cached.as_ref() }.flags;
                if flags == GIT_CACHE_STORE_PARSED {
                    let obj = cached.cast::<GitObject>();
                    // SAFETY: as above; an object stored parsed is a
                    // `git_object`, whose header is not written once shared
                    // but for its atomic count.
                    let kind = unsafe { obj.as_ref() }.raw_kind();
                    if t != GIT_OBJECT_ANY && t != kind {
                        // SAFETY: the reference the cache gave, given back.
                        unsafe { release(obj.as_ptr()) };
                        let _ = git_error_set!(
                            ErrorClass::INVALID,
                            "the requested type does not match the type in the ODB"
                        );
                        return Err(ErrorCode::NOT_FOUND);
                    }
                    return Ok(obj);
                } else if flags == GIT_CACHE_STORE_RAW {
                    cached.cast::<git_odb_object>()
                } else {
                    // (C leaks the reference here: rust/DEFECTS.md.)
                    // SAFETY: the reference the cache gave, given back.
                    unsafe { crate::cache::decref(cached) };
                    return Err(errors::assert_failed(
                        "!\"Wrong caching type in the global object cache\"",
                    ));
                }
            }
            None => odb.read(id)?,
        }
    } else {
        let short = Oid::from_raw_parts(oid_type as u8, *id.prefix(len).raw_id());
        odb.read_prefix(&short, len)?
    };
    let odb_obj = OdbObject(odb_obj);
    // SAFETY: a live odb object, which `odb_obj` holds a reference to
    // until after the call.
    unsafe { from_odb_object(repo, odb_obj.0, t) }
}

/// `git_object_lookup`.
pub fn lookup(repo: &git_repository, id: &Oid, t: git_object_t) -> Result<NonNull<GitObject>> {
    let len = oid::oid_hexsize(repo.oid_type_raw());
    lookup_prefix(repo, id, len, t)
}

// ---------------------------------------------------------------------------
// Peeling
// ---------------------------------------------------------------------------

/// `dereference_object`: what `obj` points to, one step.
///
/// # Safety
///
/// `obj` is a live object of a repository (whose repository is live).
unsafe fn dereference(obj: NonNull<GitObject>) -> core::result::Result<NonNull<GitObject>, c_int> {
    // SAFETY: a live object (this fn's contract), whose header is not
    // written once shared but for its atomic count.
    match unsafe { obj.as_ref() }.raw_kind() {
        GIT_OBJECT_COMMIT => {
            let mut out = ptr::null_mut();
            // SAFETY: an object of type commit is a live `git_commit` of a
            // live repository; `out` is a writable local.
            let r = unsafe { p::git_commit_tree(&raw mut out, obj.as_ptr().cast()) };
            if r != 0 {
                return Err(r);
            }
            NonNull::new(out.cast::<GitObject>()).ok_or(-1)
        }
        // SAFETY: an object of type tag is a live `GitTag` (this fn's
        // contract).
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
/// `object` is a live object of a repository (whose repository is live).
pub unsafe fn peel(object: NonNull<GitObject>, target: git_object_t) -> Result<NonNull<GitObject>> {
    git_assert_arg!(
        matches!(
            target,
            GIT_OBJECT_TAG | GIT_OBJECT_COMMIT | GIT_OBJECT_TREE | GIT_OBJECT_BLOB | GIT_OBJECT_ANY
        ),
        "target_type == GIT_OBJECT_TAG || target_type == GIT_OBJECT_COMMIT || target_type == GIT_OBJECT_TREE || target_type == GIT_OBJECT_BLOB || target_type == GIT_OBJECT_ANY"
    );
    let (otype, oid) = {
        // SAFETY: a live object (this fn's contract), whose header is not
        // written once shared but for its atomic count.
        let o = unsafe { object.as_ref() };
        (o.raw_kind(), o.cached.oid)
    };
    let e = check_type_combination(otype, target);
    if e < 0 {
        return Err(peel_error(e, &oid, target));
    }
    if otype == target {
        // SAFETY: a live object (this fn's contract).
        unsafe { add_ref(object) };
        return Ok(object);
    }
    let mut source = object;
    let error = loop {
        // SAFETY: `source` is `object` (live: this fn's contract) or an
        // object of the same repository this loop holds a reference to.
        match unsafe { dereference(source) } {
            Ok(deref) => {
                if source != object {
                    // SAFETY: the reference this loop took to `source`,
                    // given back once.
                    unsafe { release(source.as_ptr()) };
                }
                // SAFETY: `deref` is live with the reference just taken;
                // its header is not written once shared.
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
        // SAFETY: the reference this loop took to `source`, given back once.
        unsafe { release(source.as_ptr()) };
    }
    Err(peel_error(error, &oid, target))
}

/// `git_object_lookup_bypath`: the object at `path` in the tree that
/// `treeish` peels to, of type `t` (or any).
///
/// # Safety
///
/// `treeish` is a live object of a repository (whose repository is
/// live).
pub unsafe fn lookup_bypath(
    treeish: NonNull<GitObject>,
    path: &CStr,
    t: git_object_t,
) -> Result<NonNull<GitObject>> {
    /// A held tree entry, freed on drop.
    struct Entry(*mut p::git_tree_entry);
    impl Drop for Entry {
        fn drop(&mut self) {
            // SAFETY: NULL or the entry `git_tree_entry_bypath` gave this
            // value, which owns it, freed once.
            unsafe { p::git_tree_entry_free(self.0) }
        }
    }
    /// A held object reference, given back on drop.
    struct Held(NonNull<GitObject>);
    impl Drop for Held {
        fn drop(&mut self) {
            // SAFETY: the live object whose reference this value holds,
            // given back once.
            unsafe { release(self.0.as_ptr()) }
        }
    }

    // SAFETY: `treeish` is a live object of a live repository (this fn's
    // contract).
    let tree = Held(unsafe { peel(treeish, GIT_OBJECT_TREE)? });
    let mut entry = Entry(ptr::null_mut());
    // SAFETY: `tree.0` is a live tree (the reference `tree` holds), `path`
    // a string, and `entry.0` a writable local.
    let r =
        unsafe { p::git_tree_entry_bypath(&raw mut entry.0, tree.0.as_ptr().cast(), path.into()) };
    ErrorCode::check(r)?;
    // SAFETY: the live entry `entry` owns.
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
    // SAFETY: a live object (this fn's contract), whose header is not
    // written once shared.
    let repo = unsafe { treeish.as_ref() }.repo;
    // SAFETY: the treeish's live repository (this fn's contract), the live
    // entry `entry` owns, and `out` a writable local.
    let r = unsafe { p::git_tree_entry_to_object(&raw mut out, repo, entry.0) };
    ErrorCode::check(r)?;
    NonNull::new(out.cast::<GitObject>()).ok_or(ErrorCode::GENERIC)
}

/// `git_object__short_id`: the shortest unambiguous abbreviation of `obj`'s
/// id (at least `core.abbrev` digits), into `out`.
///
/// # Safety
///
/// `obj.repo` is NULL or a live repository (true of every object libgit2
/// made, for as long as the object is live).
pub unsafe fn short_id(out: &mut GitStr, obj: &GitObject) -> Result<()> {
    // SAFETY: NULL or a live repository (this fn's contract), only read
    // through `&` (its state is interior-mutable).
    let repo = unsafe { obj.repo.as_ref() };
    // C dereferences a NULL repository (an object from
    // `git_object__from_raw`) here: an error instead (rust/DEFECTS.md).
    let Some(repo) = repo else {
        return Err(errors::assert_failed("obj->repo"));
    };
    let oid_type = repo.oid_type_raw();
    let mut id = Oid::from_raw_parts(oid_type as u8, [0; oid::GIT_OID_MAX_SIZE]);
    let hexsize = oid::oid_hexsize(oid_type);
    let mut len: c_int = 0;
    // SAFETY: `len` is a writable local; `repo` a live repository.
    ErrorCode::check(unsafe { p::git_repository__abbrev_length(Out::from(&mut len), Some(repo)) })?;
    if len as usize == hexsize {
        // `git_oid_cpy` copies the type too.
        id = obj.cached.oid;
    }
    let mut odb = ptr::null_mut();
    // SAFETY: as above; `odb` a writable local, which receives a reference.
    ErrorCode::check(unsafe { p::git_repository_odb(&raw mut odb, Some(repo)) })?;
    /// The odb reference `git_repository_odb` gave, given back on drop.
    struct Held(*mut git_odb);
    impl Drop for Held {
        fn drop(&mut self) {
            // SAFETY: the reference `git_repository_odb` gave this value,
            // given back once.
            unsafe { p::git_odb_free(self.0) }
        }
    }
    let odb = Held(odb);
    // SAFETY: the live odb `odb` holds a reference to, for as long as this
    // borrow of it; only read through `&` (odb module docs).
    let Some(db) = (unsafe { odb.0.as_ref() }) else {
        return Err(errors::assert_failed("odb"));
    };
    let mut error = 0;
    while (len as usize) < hexsize {
        let l = len as usize;
        let n = l.div_ceil(2);
        let src = *obj.cached.oid.raw_id();
        let dst = id.raw_id_mut();
        dst[..n].copy_from_slice(&src[..n]);
        if l & 1 == 1 {
            dst[l / 2] &= 0xf0;
        }
        error = match db.exists_prefix(&id, l) {
            Ok(_) => 0,
            Err(e) => e.raw(),
        };
        if error != ErrorCode::AMBIGUOUS.raw() {
            break;
        }
        errors::clear();
        len += 1;
    }
    if error != 0 {
        return Err(ErrorCode::from_raw(error).unwrap_or(ErrorCode::GENERIC));
    }
    let len = len as usize;
    out.grow(len + 1).map(|()| {
        // `git_oid_tostr(out->ptr, len + 1, &id); out->size = len`.
        let hex = id.hex();
        let digits = &hex.as_bytes()[..len.min(hex.as_bytes().len())];
        out.clear();
        let _ = out.put(digits);
    })
}

/// `git_object__is_valid`: with strict object creation, whether `id` names
/// an object of type `expected` (or any) in the repository's odb.
pub fn is_valid(repo: &git_repository, id: &Oid, expected: git_object_t) -> bool {
    if !strict_input_validation() {
        return true;
    }
    let Ok(odb) = repo.odb() else {
        return false;
    };
    let Ok((_, actual)) = odb.read_header(id) else {
        return false;
    };
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

/// `normalize_options` (the filter list is copied, not read).
fn normalize_options(given: Option<&GitObjectIdOptions>) -> Result<IdOptions> {
    let g = given.copied();
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
    // SAFETY: a non-NULL `opts.filters` is a live filter list (this fn's
    // contract).
    !opts.filters.is_null()
        && unsafe { cfilter::list_length(opts.filters) } > 0
        && (opts.object_type == 0 || opts.object_type == GIT_OBJECT_BLOB)
}

/// `git_odb__format_object_header`: `"<type> <len>\0"`; the length
/// written, the NUL included.
pub fn format_object_header(out: &mut [u8; 64], len: u64, t: git_object_t) -> Result<usize> {
    // `%s %"PRId64` (at most 27 bytes with the NUL, so it always fits).
    let name = type2string(t).to_bytes();
    out[..name.len()].copy_from_slice(name);
    let mut n = name.len();
    out[n] = b' ';
    n += 1;
    let v = len as i64;
    if v < 0 {
        out[n] = b'-';
        n += 1;
    }
    let mut digits = [0u8; 20];
    let mut m = v.unsigned_abs();
    let mut d = digits.len();
    loop {
        d -= 1;
        digits[d] = b'0' + (m % 10) as u8;
        m /= 10;
        if m == 0 {
            break;
        }
    }
    let dl = digits.len() - d;
    out[n..n + dl].copy_from_slice(&digits[d..]);
    n += dl;
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
    let opts = normalize_options(Some(&GitObjectIdOptions {
        version: GIT_OBJECT_ID_OPTIONS_VERSION,
        object_type: t.raw(),
        oid_type: oid_type.raw(),
        filters: ptr::null_mut(),
    }))?;
    id_from_buffer_opts(data, &opts)
}

/// The id of `data` as an object (no filters); `None` is C's NULL, an
/// error unless `len` is 0.
fn id_from_data(data: Option<&[u8]>, len: usize, opts: &IdOptions) -> Result<Oid> {
    match data {
        None if len != 0 => Err(git_error_set!(ErrorClass::INVALID, "invalid object")),
        _ => id_from_buffer_opts(data.unwrap_or(&[]), opts),
    }
}

/// `git_object_id_from_buffer` with normalized options.
///
/// # Safety
///
/// `opts.filters` is NULL or a live filter list.
unsafe fn id_from_buffer_c(data: Option<&[u8]>, len: usize, opts: &IdOptions) -> Result<Oid> {
    let mut filtered = GitStr::new();
    let mut data = data;
    let mut len = len;
    // SAFETY: `opts.filters` is NULL or live (this fn's contract).
    if unsafe { needs_filter(opts) } {
        // SAFETY: as above, and non-NULL (`needs_filter`).
        unsafe { cfilter::apply_to_buffer(&mut filtered, opts.filters, data.unwrap_or(&[]))? };
        data = Some(filtered.as_bytes());
        len = filtered.len();
    }
    id_from_data(data, len, opts)
}

/// The `char buffer[GIT_BUFSIZE_FILEIO]` C reads files through: 64 KiB,
/// reserved fallibly and never zero-filled.
pub(crate) struct FileReadBuffer(Vec<u8>);

impl FileReadBuffer {
    /// Reserve the buffer (the out-of-memory error if that fails).
    pub(crate) fn new() -> Result<FileReadBuffer> {
        let mut v = Vec::new();
        v.try_reserve_exact(GIT_BUFSIZE_FILEIO)
            .map_err(|_| errors::oom())?;
        Ok(FileReadBuffer(v))
    }

    /// `p_read(fd, buffer, sizeof(buffer))`: the bytes read (empty at end of
    /// file), or the error with `errno` kept.
    pub(crate) fn read(&mut self, fd: c_int) -> posix::SysResult<&[u8]> {
        let spare = self.0.spare_capacity_mut();
        let len = spare.len().min(GIT_BUFSIZE_FILEIO);
        let p = spare.as_mut_ptr().cast::<u8>();
        // SAFETY: `p` is valid for writes of `len` bytes (the reserved,
        // unused capacity, borrowed through `&mut self`).
        let r = unsafe { posix::read_raw(fd, p, len) };
        let Ok(n) = usize::try_from(r) else {
            return Err(posix::Errno(errors::errno()));
        };
        // SAFETY: `read_raw` initialized the first `n <= len` bytes at `p`,
        // which stay borrowed (and unwritten) as long as the result.
        Ok(unsafe { core::slice::from_raw_parts(p, n) })
    }
}

/// `id_from_fd`: hash `size` bytes of `fd` as an object (no filters).
fn id_from_fd_raw(fd: c_int, mut size: usize, opts: &IdOptions) -> Result<Oid> {
    let mut h = Hasher::new(algorithm(opts.oid_type))?;
    let mut header = [0u8; 64];
    let n = format_object_header(&mut header, size as u64, opts.object_type)?;
    h.update(&header[..n])?;
    let mut buffer = FileReadBuffer::new()?;
    let mut read_failed = false;
    while size > 0 {
        match buffer.read(fd) {
            Ok([]) => break,
            Ok(data) => {
                if data.len() > size {
                    return Err(git_error_set!(
                        ErrorClass::OS,
                        "error reading file for hashing"
                    ));
                }
                h.update(data)?;
                size -= data.len();
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

/// `git_object_id_from_fd` with normalized options.
///
/// # Safety
///
/// `opts.filters` is NULL or a live filter list.
unsafe fn id_from_fd_c(fd: c_int, size: usize, opts: &IdOptions) -> Result<Oid> {
    // SAFETY: `opts.filters` is NULL or live (this fn's contract).
    if !unsafe { needs_filter(opts) } {
        return id_from_fd_raw(fd, size, opts);
    }
    // The size goes into the header: read it all, then filter.
    let mut raw = GitStr::new();
    futils::readbuffer_fd(&mut raw, fd, size)?;
    // SAFETY: as above.
    unsafe { id_from_buffer_c(Some(raw.as_bytes()), raw.len(), opts) }
}

/// `git_object_id_from_file` with normalized options.
///
/// # Safety
///
/// As [`id_from_fd_c`].
unsafe fn id_from_file_c(path: &CStr, opts: &IdOptions) -> Result<Oid> {
    let fd = futils::open_ro(path)?;
    let r = (|| {
        let size = futils::filesize(fd)?;
        let Ok(size) = usize::try_from(size) else {
            return Err(git_error_set!(
                ErrorClass::OS,
                "file size overflow for 32-bit systems"
            ));
        };
        // SAFETY: `opts.filters` is NULL or live (this fn's contract).
        unsafe { id_from_fd_c(fd, size, opts) }
    })();
    // SAFETY: the descriptor this fn opened above, closed once.
    let _ = unsafe { posix::close(fd) };
    r
}

/// `git_object_id_from_symlink` (the filters are ignored).
fn id_from_symlink_c(path: &CStr, given: Option<&GitObjectIdOptions>) -> Result<Oid> {
    let mut opts = normalize_options(given)?;
    opts.filters = ptr::null_mut();
    if opts.object_type != GIT_OBJECT_BLOB {
        return Err(git_error_set!(
            ErrorClass::INVALID,
            "symbolic links must be blob types"
        ));
    }
    // (C folds lstat's error to -1, keeping its message.)
    let st = crate::util::fs_path::lstat(path).map_err(|_| ErrorCode::GENERIC)?;
    if !posix::s_islnk(st.st_mode) {
        // SAFETY: `opts.filters` is NULL (cleared above).
        return unsafe { id_from_file_c(path, &opts) };
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
#[inline]
pub fn parse_oid_header(buf: &[u8], header: &[u8], oid_type: OidType) -> Result<(Oid, usize)> {
    let sha_len = oid_type.hexsize();
    let h = header.len();
    if h + sha_len + 1 > buf.len() || &buf[..h] != header || buf[h + sha_len] != b'\n' {
        return Err(ErrorCode::GENERIC);
    }
    let id = Oid::parse_prefix(&buf[h..h + sha_len], oid_type).map_err(|_| ErrorCode::GENERIC)?;
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
    printf_pieces(buf, format_len, &[bytes])
}

/// [`printf_bytes`] of the concatenation of `pieces` (no copy of them is
/// made first).
pub(crate) fn printf_pieces(buf: &mut GitStr, format_len: usize, pieces: &[&[u8]]) -> Result<()> {
    buf.vprintf_begin(format_len)?;
    let total = pieces
        .iter()
        .try_fold(0usize, |t, p| t.checked_add(p.len()));
    let Some(len) = total.and_then(|t| c_int::try_from(t).ok()) else {
        // `vsnprintf` fails beyond INT_MAX.
        // SAFETY: -1 is never written.
        unsafe { buf.vprintf_written(-1)? };
        return Err(ErrorCode::GENERIC);
    };
    let total = len as usize;
    loop {
        let size = buf.len();
        // The room after the contents, the NUL's slot included.
        let (spare, room) = buf.spare_ptr();
        if total < room {
            let mut at = spare;
            for p in pieces {
                // SAFETY: the pieces fit in the `room > total` bytes at
                // `spare`, the string's own buffer (borrowed through
                // `&mut buf`), disjoint from them.
                unsafe { ptr::copy_nonoverlapping(p.as_ptr(), at, p.len()) };
                at = at.wrapping_add(p.len());
            }
            // SAFETY: the byte after them, within `room`.
            unsafe { at.write(0) };
            // SAFETY: the string owns its buffer (`room > 0`), the bytes up
            // to `size + total` are written and the NUL follows them.
            unsafe { buf.set_len(size + total) };
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

/// An object of a repository (`git_object *`), one reference of it; given
/// back on drop, duplicated by `Clone` (`git_object_dup`).
///
/// [`Commit`](crate::Commit), [`Tree`](crate::Tree), [`Blob`](crate::Blob)
/// and [`Tag`](crate::Tag) are this type with its kind checked: an object
/// becomes one with [`into_commit`](Self::into_commit) & co. (or
/// `TryFrom`), which give the object back when it is of another kind, and
/// one becomes an object again with `into_object` (`as_object` borrows).
///
/// `Object` is `Send` and `Sync`: the object is immutable once parsed (only
/// its reference count, an atomic, changes), and what its methods reach
/// besides (the repository's object cache and odb) is synchronised.
pub struct Object<'repo> {
    raw: NonNull<GitObject>,
    _repo: PhantomData<&'repo Repository>,
}

// SAFETY: an `Object` holds one counted reference to an object that nothing
// writes after it is published (module docs: only `cached.refcount`, an
// atomic, changes; the C ABI's lazily set `summary`/`body` of a commit are
// never touched by the safe API, and are other fields). Giving the
// reference back on another thread is `git_cached_obj_decref`, an atomic
// decrement, and the last one frees the object (and its odb object, with
// its own atomic count) with `git__allocator`, which any thread may call.
// The repository it borrows is `Sync`.
unsafe impl Send for Object<'_> {}
// SAFETY: every `&self` method only reads the immutable object, takes
// another reference (atomic), or goes through the repository (lookups,
// peeling, the odb for short ids), whose caches, odb and configuration are
// synchronised (`Repository: Sync`).
unsafe impl Sync for Object<'_> {}

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

    /// Adopt a reference to an object (crate-internal: the repository's
    /// C-shaped code, which ties `'repo` to its own borrow, and objects
    /// parsed outside any repository, for as long as their data lives).
    ///
    /// # Safety
    ///
    /// `raw` is a live object whose reference passes to the result, and
    /// whose repository (if any) and data outlive `'repo`.
    pub(crate) unsafe fn adopt(raw: NonNull<GitObject>) -> Object<'repo> {
        Object {
            raw,
            _repo: PhantomData,
        }
    }

    /// Give the reference up, as a `git_object *`.
    pub(crate) fn into_raw(self) -> *mut GitObject {
        let p = self.raw.as_ptr();
        core::mem::forget(self);
        p
    }

    /// `git_object_peel` with C's type and error code (crate-internal).
    pub(crate) fn peel_raw(&self, kind: git_object_t) -> Result<Object<'repo>> {
        // SAFETY: a live object of a repository.
        let raw = unsafe { peel(self.raw, kind)? };
        // SAFETY: an object of the same repository, with its reference.
        Ok(unsafe { self.sibling(raw) })
    }

    /// Adopt a reference to an object of the repository `self` belongs to.
    ///
    /// # Safety
    ///
    /// `raw` is a live object of `self`'s repository whose reference passes
    /// to the result.
    pub(crate) unsafe fn sibling(&self, raw: NonNull<GitObject>) -> Object<'repo> {
        Object {
            raw,
            _repo: PhantomData,
        }
    }

    /// `git_object_lookup`: the object `id`, of type `kind` (`None`: any;
    /// another type is `NOT_FOUND`).
    pub fn lookup(
        repo: &'repo Repository,
        id: &Oid,
        kind: Option<ObjectType>,
    ) -> core::result::Result<Object<'repo>, Error> {
        let raw = lookup(repo.raw(), id, ObjectType::raw_or_any(kind))?;
        // SAFETY: a live object of `repo`, with the reference `lookup`
        // gave, which passes to the result.
        Ok(unsafe { Object::from_raw(repo, raw) })
    }

    /// `git_object_lookup_prefix`: by the first `len` hex digits of `id`
    /// (`AMBIGUOUS` below 4 or when several objects match).
    pub fn lookup_prefix(
        repo: &'repo Repository,
        id: &Oid,
        len: usize,
        kind: Option<ObjectType>,
    ) -> core::result::Result<Object<'repo>, Error> {
        let raw = lookup_prefix(repo.raw(), id, len, ObjectType::raw_or_any(kind))?;
        // SAFETY: as in `lookup`.
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
        // A cached object's type is one of the four (the parsers set it).
        ObjectType::from_raw(self.header().raw_kind()).unwrap_or(ObjectType::Blob)
    }

    /// The object's repository (crate-internal), `None` for an object
    /// parsed outside any.
    pub(crate) fn owner(&self) -> Option<&'repo crate::repository::GitRepository> {
        // SAFETY: NULL or the repository that outlives `'repo` (an object
        // of the safe API borrows its repository).
        unsafe {
            self.header()
                .repo
                .cast::<crate::repository::GitRepository>()
                .as_ref()
        }
    }

    /// The `git_repository *` of the object (NULL for one parsed outside
    /// any), live for `'repo`.
    pub(crate) fn repo_raw(&self) -> *mut git_repository {
        self.header().repo
    }

    /// An object parsed outside a repository cannot reach one (C would
    /// crash): `GIT_ASSERT_ARG(repo)`'s error.
    pub(crate) fn need_repo(&self) -> core::result::Result<(), Error> {
        if self.repo_raw().is_null() {
            return Err(Error::from(errors::assert_arg_failed("repo")));
        }
        Ok(())
    }

    /// The C handle, live while `self` is.
    pub(crate) fn as_ptr(&self) -> NonNull<GitObject> {
        self.raw
    }

    /// `git_object_peel`: peel tags (and commits to their trees) until an
    /// object of type `kind`; `None`: until an object that is not a tag.
    pub fn peel(&self, kind: Option<ObjectType>) -> core::result::Result<Object<'repo>, Error> {
        if kind != Some(self.kind()) {
            self.need_repo()?;
        }
        // SAFETY: a live object, of a repository unless no step is taken
        // (checked above).
        let raw = unsafe { peel(self.raw, ObjectType::raw_or_any(kind))? };
        // SAFETY: an object of the same repository, with its reference.
        Ok(unsafe { self.sibling(raw) })
    }

    /// `git_object_short_id`: the shortest unambiguous abbreviation of the
    /// id in the repository (at least `core.abbrev` digits), in hex.
    pub fn short_id(&self) -> core::result::Result<String, Error> {
        self.need_repo()?;
        let mut s = GitStr::new();
        // SAFETY: the object's `repo` is NULL or the live repository it
        // borrows (an `Object` of the safe API outlives neither).
        unsafe { short_id(&mut s, self.header())? };
        // Hex digits only.
        Ok(String::from_utf8_lossy(s.as_bytes()).into_owned())
    }

    /// `git_object_lookup_bypath`: the object at `path` (`/`-separated) in
    /// the tree this object peels to, of type `kind` (`None`: any).
    pub fn lookup_bypath(
        &self,
        path: impl AsRef<[u8]>,
        kind: Option<ObjectType>,
    ) -> core::result::Result<Object<'repo>, Error> {
        self.need_repo()?;
        let c = std::ffi::CString::new(path.as_ref()).map_err(|_| {
            Error::from(git_error_set!(
                ErrorClass::INVALID,
                "path contains a NUL byte"
            ))
        })?;
        // SAFETY: a live object of a repository (checked above), a
        // NUL-terminated path.
        let raw = unsafe { lookup_bypath(self.raw, &c, ObjectType::raw_or_any(kind))? };
        // SAFETY: an object of the same repository.
        Ok(unsafe { self.sibling(raw) })
    }

    /// This object as a commit, or the object back if it is not one.
    pub fn into_commit(self) -> core::result::Result<crate::commit::api::Commit<'repo>, Self> {
        crate::commit::api::Commit::try_from(self)
    }

    /// This object as a tree, or the object back if it is not one.
    pub fn into_tree(self) -> core::result::Result<crate::tree::api::Tree<'repo>, Self> {
        crate::tree::api::Tree::try_from(self)
    }

    /// This object as a blob, or the object back if it is not one.
    pub fn into_blob(self) -> core::result::Result<crate::blob::Blob<'repo>, Self> {
        crate::blob::Blob::try_from(self)
    }

    /// This object as a tag, or the object back if it is not one.
    pub fn into_tag(self) -> core::result::Result<crate::tag::Tag<'repo>, Self> {
        crate::tag::Tag::try_from(self)
    }

    /// This object borrowed as a tree, if it is one. (A commit cannot be
    /// borrowed: [`Commit`](crate::Commit) keeps caches beside the object;
    /// use [`into_commit`](Self::into_commit) on a clone.)
    pub fn as_tree(&self) -> Option<&crate::tree::api::Tree<'repo>> {
        (self.kind() == ObjectType::Tree).then(|| crate::tree::api::Tree::from_object_ref(self))
    }

    /// This object borrowed as a blob, if it is one.
    pub fn as_blob(&self) -> Option<&crate::blob::Blob<'repo>> {
        (self.kind() == ObjectType::Blob).then(|| crate::blob::Blob::from_object_ref(self))
    }

    /// This object borrowed as a tag, if it is one.
    pub fn as_tag(&self) -> Option<&crate::tag::Tag<'repo>> {
        (self.kind() == ObjectType::Tag).then(|| crate::tag::Tag::from_object_ref(self))
    }
}

impl Object<'static> {
    /// `git_object_id_from_buffer`: the id `data` would have as an object
    /// (the header `"<type> <len>\0"` prepended, hashed), with `opts`'
    /// object and id types. libgit2's filters need a repository (their list
    /// is loaded from its attributes): use
    /// [`Repository::hashfile`](crate::Repository::hashfile) for a filtered
    /// blob id.
    pub fn id_from_buffer(
        data: impl AsRef<[u8]>,
        opts: ObjectIdOptions,
    ) -> core::result::Result<Oid, Error> {
        crate::libgit2::ensure_init()?;
        Ok(id_from_buffer(
            data.as_ref(),
            opts.object_type,
            opts.oid_type,
        )?)
    }

    /// `git_object_id_from_file`: as [`id_from_buffer`](Self::id_from_buffer)
    /// over the contents of the file at `path` (no filters, no attributes:
    /// see there).
    pub fn id_from_file(
        path: impl AsRef<std::path::Path>,
        opts: ObjectIdOptions,
    ) -> core::result::Result<Oid, Error> {
        use std::os::unix::ffi::OsStrExt as _;
        crate::libgit2::ensure_init()?;
        let path = std::ffi::CString::new(path.as_ref().as_os_str().as_bytes()).map_err(|_| {
            Error::from(git_error_set!(
                ErrorClass::INVALID,
                "path contains a NUL byte"
            ))
        })?;
        let o = normalize_options(Some(&opts.raw()))?;
        // SAFETY: `o.filters` is NULL (`ObjectIdOptions::raw` sets none).
        Ok(unsafe { id_from_file_c(&path, &o) }?)
    }

    /// `git_object_rawcontent_is_valid`: whether `data` parses as an object
    /// of type `kind` whose ids are of type `oid_type` (blobs always do).
    pub fn rawcontent_is_valid(
        data: impl AsRef<[u8]>,
        kind: ObjectType,
        oid_type: OidType,
    ) -> core::result::Result<bool, Error> {
        crate::libgit2::ensure_init()?;
        Ok(rawcontent_is_valid(data.as_ref(), kind, oid_type)?)
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

/// Options of [`Object::id_from_buffer`] and [`Object::id_from_file`]
/// (`git_object_id_options`, without the filters: see there). By-value
/// builder; the default is a blob with a SHA-1 id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectIdOptions {
    object_type: ObjectType,
    oid_type: OidType,
}

impl Default for ObjectIdOptions {
    fn default() -> Self {
        ObjectIdOptions::new()
    }
}

impl ObjectIdOptions {
    /// A blob, SHA-1 (`GIT_OBJECT_ID_OPTIONS_INIT`).
    pub const fn new() -> ObjectIdOptions {
        ObjectIdOptions {
            object_type: ObjectType::Blob,
            oid_type: OidType::Sha1,
        }
    }

    /// The type of the object the data is the content of.
    pub const fn object_type(mut self, t: ObjectType) -> Self {
        self.object_type = t;
        self
    }

    /// The type of id to compute.
    pub const fn oid_type(mut self, t: OidType) -> Self {
        self.oid_type = t;
        self
    }

    fn raw(self) -> GitObjectIdOptions {
        GitObjectIdOptions {
            version: GIT_OBJECT_ID_OPTIONS_VERSION,
            object_type: self.object_type.raw(),
            oid_type: self.oid_type.raw(),
            filters: ptr::null_mut(),
        }
    }
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

pub mod ffi {
    //! The exports of `object.c`.
    //!
    //! A live repository is taken as `Option<&GitRepository>` (it is only
    //! read through `&`: its state is interior-mutable), and an object
    //! that is only read as `Option<&GitObject>` (a published object's
    //! header is not written but for its atomic count, module docs). An
    //! object that the call hands back to C, or gives a reference of,
    //! stays a raw pointer, so that C frees it through C's own pointer.
    use super::*;
    use crate::buf::GitBuf;
    use crate::git_arg;
    use crate::util::cabi::{CStrArg, Out};
    use crate::util::export::{barrier, bytes, status};

    /// Write an object result to C's out-parameter.
    #[inline]
    fn give(out: &mut Out<'_, *mut GitObject>, r: Result<NonNull<GitObject>>) -> c_int {
        match r {
            Ok(o) => {
                out.write(o.as_ptr());
                0
            }
            Err(e) => e.raw(),
        }
    }

    /// `git_object__from_raw`.
    ///
    /// # Safety
    ///
    /// `object_out` is NULL or writable; `data` is NULL or readable for
    /// `size` bytes, which nothing writes while the object lives.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object__from_raw(
        mut object_out: Out<'_, *mut GitObject>,
        data: *const c_char,
        size: usize,
        object_type: git_object_t,
        oid_type: crate::oid::git_oid_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!object_out.is_null(), "object_out", -1);
            object_out.write(ptr::null_mut());
            // SAFETY: `data` is NULL or readable for `size` bytes that
            // outlive the object, unwritten (this export's contract).
            let r = unsafe { from_raw(data, size, object_type, oid_type) };
            give(&mut object_out, r)
        })
    }

    /// `git_object__init_from_odb_object`.
    ///
    /// # Safety
    ///
    /// `object_out` is NULL or writable; `repo` anything (it is stored in
    /// the object, not read: NULL or a live repository for the object to
    /// be usable); `odb_obj` NULL or a live odb object.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object__init_from_odb_object(
        mut object_out: Out<'_, *mut GitObject>,
        repo: *mut git_repository,
        odb_obj: *mut git_odb_object,
        object_type: git_object_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!object_out.is_null(), "object_out", -1);
            object_out.write(ptr::null_mut());
            // (C dereferences a NULL `odb_obj`.)
            let odb_obj = git_arg!(NonNull::new(odb_obj), "odb_obj", -1);
            // SAFETY: a live odb object (this export's contract).
            let r = unsafe { init_from_odb_object(repo, odb_obj, object_type) };
            give(&mut object_out, r)
        })
    }

    /// `git_object__from_odb_object`.
    ///
    /// # Safety
    ///
    /// `object_out` is NULL or writable; `repo` NULL or a live repository;
    /// `odb_obj` NULL or a live odb object the caller holds a reference to.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object__from_odb_object(
        mut object_out: Out<'_, *mut GitObject>,
        repo: Option<&git_repository>,
        odb_obj: *mut git_odb_object,
        object_type: git_object_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!object_out.is_null(), "object_out", -1);
            // (C dereferences a NULL `odb_obj` or `repo`.)
            let odb_obj = git_arg!(NonNull::new(odb_obj), "odb_obj", -1);
            let repo = git_arg!(repo, "repo", -1);
            // SAFETY: a live odb object, held by the caller for the call
            // (this export's contract).
            match unsafe { from_odb_object(repo, odb_obj, object_type) } {
                Ok(o) => {
                    object_out.write(o.as_ptr());
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
    /// `obj` is an object libgit2 made (or C, of C's size for its type),
    /// with no reference left: nothing uses it again.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object__free(obj: *mut c_void) {
        barrier(|| {
            // SAFETY: such an object, not used again (this export's
            // contract), as `free_object` requires.
            unsafe { free_object(obj.cast()) }
        })
    }

    /// `git_object_lookup_prefix`, for the lookup exports (and
    /// `object_api.c`'s).
    pub(crate) fn lookup_prefix_c(
        mut object_out: Out<'_, *mut GitObject>,
        repo: Option<&git_repository>,
        id: Option<&Oid>,
        len: usize,
        object_type: git_object_t,
    ) -> c_int {
        barrier(|| {
            let repo = git_arg!(repo, "repo", -1);
            git_assert_arg!(!object_out.is_null(), "object_out", -1);
            let id = git_arg!(id, "id", -1);
            let r = lookup_prefix(repo, id, len, object_type);
            give(&mut object_out, r)
        })
    }

    /// `git_object_lookup_prefix`.
    ///
    /// # Safety
    ///
    /// `object_out` is NULL or writable; `repo` is NULL or a live
    /// repository; `id` is NULL or a readable id that nothing writes during
    /// the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_lookup_prefix(
        object_out: Out<'_, *mut GitObject>,
        repo: Option<&git_repository>,
        id: Option<&Oid>,
        len: usize,
        object_type: git_object_t,
    ) -> c_int {
        lookup_prefix_c(object_out, repo, id, len, object_type)
    }

    /// `git_object_lookup`.
    ///
    /// # Safety
    ///
    /// As [`git_object_lookup_prefix`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_lookup(
        object_out: Out<'_, *mut GitObject>,
        repo: Option<&git_repository>,
        id: Option<&Oid>,
        object_type: git_object_t,
    ) -> c_int {
        lookup_c(object_out, repo, id, object_type)
    }

    /// `git_object_lookup`, for the lookup exports (and `object_api.c`'s).
    pub(crate) fn lookup_c(
        object_out: Out<'_, *mut GitObject>,
        repo: Option<&git_repository>,
        id: Option<&Oid>,
        object_type: git_object_t,
    ) -> c_int {
        // (C reads `repo->oid_type` before checking `repo`.)
        let len = repo.map_or(0, |r| oid::oid_hexsize(repo_oid_type(r)));
        lookup_prefix_c(object_out, repo, id, len, object_type)
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
            // SAFETY: NULL or an object whose reference the caller gives up
            // (this export's contract).
            unsafe { release(object) }
        })
    }

    /// `git_object_id`.
    ///
    /// # Safety
    ///
    /// `obj` is NULL or a live object.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_id(obj: Option<&GitObject>) -> *const Oid {
        object_id(obj)
    }

    /// `git_object_id`'s body (and `object_api.c`'s).
    pub(crate) fn object_id(obj: Option<&GitObject>) -> *const Oid {
        &raw const git_arg!(obj, "obj", ptr::null()).cached.oid
    }

    /// `git_object_type`.
    ///
    /// # Safety
    ///
    /// `obj` is NULL or a live object.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_type(obj: Option<&GitObject>) -> git_object_t {
        git_arg!(obj, "obj", GIT_OBJECT_INVALID).raw_kind()
    }

    /// `git_object_owner`.
    ///
    /// # Safety
    ///
    /// `obj` is NULL or a live object.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_owner(obj: Option<&GitObject>) -> *mut git_repository {
        object_owner(obj)
    }

    /// `git_object_owner`'s body (and `object_api.c`'s).
    pub(crate) fn object_owner(obj: Option<&GitObject>) -> *mut git_repository {
        git_arg!(obj, "obj", ptr::null_mut()).repo
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
    pub unsafe extern "C" fn git_object_string2type(s: CStrArg<'_>) -> git_object_t {
        match s.get() {
            None => GIT_OBJECT_INVALID,
            Some(s) => stringn2type(s.to_bytes()),
        }
    }

    /// `git_object_stringn2type`.
    ///
    /// # Safety
    ///
    /// `s` is NULL or readable for `len` bytes, which nothing writes during
    /// the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_stringn2type(s: *const c_char, len: usize) -> git_object_t {
        barrier(|| {
            if s.is_null() {
                return GIT_OBJECT_INVALID;
            }
            // SAFETY: a non-NULL `s` is readable for `len` unwritten bytes
            // (this export's contract).
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
    /// live repository.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_peel(
        mut peeled: Out<'_, *mut GitObject>,
        object: *const GitObject,
        target_type: git_object_t,
    ) -> c_int {
        barrier(|| {
            let o = git_arg!(NonNull::new(object.cast_mut()), "object", -1);
            git_assert_arg!(!peeled.is_null(), "peeled", -1);
            // SAFETY: a live object of a live repository (this export's
            // contract).
            let r = unsafe { peel(o, target_type) };
            give(&mut peeled, r)
        })
    }

    /// `git_object_dup`.
    ///
    /// # Safety
    ///
    /// `dest` is NULL or writable and `source` NULL or a live object.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_dup(
        mut dest: Out<'_, *mut GitObject>,
        source: *mut GitObject,
    ) -> c_int {
        barrier(|| {
            let nn = git_arg!(NonNull::new(source), "source", -1);
            // SAFETY: a live object (this export's contract).
            unsafe { add_ref(nn) };
            dest.write(source);
            0
        })
    }

    /// `git_object_lookup_bypath`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `treeish` NULL or a live object of a live
    /// repository; `path` NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_lookup_bypath(
        mut out: Out<'_, *mut GitObject>,
        treeish: *const GitObject,
        path: CStrArg<'_>,
        object_type: git_object_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!out.is_null(), "out", -1);
            let t = git_arg!(NonNull::new(treeish.cast_mut()), "treeish", -1);
            let path = git_arg!(path.get(), "path", -1);
            // SAFETY: a live object of a live repository (this export's
            // contract).
            let r = unsafe { lookup_bypath(t, path, object_type) };
            give(&mut out, r)
        })
    }

    /// `git_object_short_id`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or a valid `git_buf`; `obj` NULL or a live object.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_short_id(
        out: *mut GitBuf,
        obj: Option<&GitObject>,
    ) -> c_int {
        barrier(|| {
            let f = |s: &mut GitStr| {
                let o = git_arg!(obj, "obj");
                // SAFETY: a live object (this export's contract), whose
                // `repo` is NULL or a live repository (libgit2 made it).
                unsafe { short_id(s, o) }
            };
            // SAFETY: `out` is NULL or a valid `git_buf` (this export's
            // contract).
            unsafe { buf_wrap(out, f) }
        })
    }

    /// `git_object__is_valid`.
    ///
    /// # Safety
    ///
    /// `repo` is NULL or a live repository; `id` NULL or a readable id that
    /// nothing writes during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object__is_valid(
        repo: Option<&git_repository>,
        id: Option<&Oid>,
        expected_type: git_object_t,
    ) -> bool {
        barrier(|| {
            if !strict_input_validation() {
                return true;
            }
            let id = git_arg!(id, "id", false);
            // (C fails `git_repository_odb__weakptr`'s argument check.)
            let repo = git_arg!(repo, "repo", false);
            is_valid(repo, id, expected_type)
        })
    }

    /// `git_object_id_options_init`.
    ///
    /// # Safety
    ///
    /// `opts` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_id_options_init(
        mut opts: Out<'_, GitObjectIdOptions>,
        version: c_uint,
    ) -> c_int {
        barrier(|| {
            if let Err(e) = check_version(
                Some(&version),
                GIT_OBJECT_ID_OPTIONS_VERSION,
                "git_object_id_options",
            ) {
                return e.raw();
            }
            opts.write(GitObjectIdOptions {
                version: GIT_OBJECT_ID_OPTIONS_VERSION,
                object_type: 0,
                oid_type: 0,
                filters: ptr::null_mut(),
            });
            0
        })
    }

    /// `git_object_id_from_fd`.
    ///
    /// # Safety
    ///
    /// `id` is NULL or writable; `opts` NULL or readable, unwritten during
    /// the call, with a live filter list or none.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_id_from_fd(
        id: Out<'_, Oid>,
        fd: c_int,
        size: usize,
        opts: Option<&GitObjectIdOptions>,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!id.is_null(), "id", -1);
            let r = normalize_options(opts).and_then(|o| {
                // SAFETY: `o.filters` is NULL or the caller's live list
                // (this export's contract).
                unsafe { id_from_fd_c(fd, size, &o) }
            });
            write_id(id, r)
        })
    }

    /// `git_object_id_from_symlink`.
    ///
    /// # Safety
    ///
    /// `id` is NULL or writable; `opts` NULL or readable, unwritten during
    /// the call (its filters are not used); `path` NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_id_from_symlink(
        id: Out<'_, Oid>,
        path: CStrArg<'_>,
        opts: Option<&GitObjectIdOptions>,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!id.is_null(), "id", -1);
            let path = git_arg!(path.get(), "path", -1);
            write_id(id, id_from_symlink_c(path, opts))
        })
    }

    /// `git_object_id_from_file`.
    ///
    /// # Safety
    ///
    /// As [`git_object_id_from_fd`]; `path` NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_id_from_file(
        id: Out<'_, Oid>,
        path: CStrArg<'_>,
        opts: Option<&GitObjectIdOptions>,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!id.is_null(), "id", -1);
            let path = git_arg!(path.get(), "path", -1);
            let r = normalize_options(opts).and_then(|o| {
                // SAFETY: `o.filters` is NULL or the caller's live list
                // (this export's contract).
                unsafe { id_from_file_c(path, &o) }
            });
            write_id(id, r)
        })
    }

    /// `git_object_id_from_buffer`.
    ///
    /// # Safety
    ///
    /// `id` is NULL or writable; `data` NULL or readable for `len` bytes,
    /// which nothing writes during the call; `opts` as
    /// [`git_object_id_from_fd`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_id_from_buffer(
        id: Out<'_, Oid>,
        data: *const c_void,
        len: usize,
        opts: Option<&GitObjectIdOptions>,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!id.is_null(), "id", -1);
            let d = if data.is_null() {
                None
            } else {
                // SAFETY: a non-NULL `data` is readable for `len` unwritten
                // bytes (this export's contract).
                Some(unsafe { bytes(data.cast(), len) })
            };
            let r = normalize_options(opts).and_then(|o| {
                // SAFETY: `o.filters` is NULL or the caller's live list
                // (this export's contract).
                unsafe { id_from_buffer_c(d, len, &o) }
            });
            write_id(id, r)
        })
    }

    /// Write an id result to C's out-parameter.
    fn write_id(mut id: Out<'_, Oid>, r: Result<Oid>) -> c_int {
        match r {
            Ok(v) => {
                id.write(v);
                0
            }
            Err(e) => e.raw(),
        }
    }

    /// `git_object_rawcontent_is_valid`.
    ///
    /// # Safety
    ///
    /// `valid` is NULL or writable; `buf` NULL or readable for `len` bytes,
    /// which nothing writes during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object_rawcontent_is_valid(
        mut valid: Out<'_, c_int>,
        buf: *const c_char,
        len: usize,
        object_type: git_object_t,
        oid_type: crate::oid::git_oid_t,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!valid.is_null(), "valid", -1);
            git_assert_arg!(!buf.is_null(), "buf", -1);
            if object_type == GIT_OBJECT_BLOB {
                valid.write(1);
                return 0;
            }
            // SAFETY: `buf` is readable for `len` bytes, unwritten during
            // the call (this export's contract), which outlives the
            // object: it is released below.
            match unsafe { from_raw(buf, len, object_type, oid_type) } {
                Ok(o) => {
                    // SAFETY: the reference `from_raw` gave, given back.
                    unsafe { release(o.as_ptr()) };
                    valid.write(1);
                    0
                }
                Err(e) if e == ErrorCode::INVALID => {
                    valid.write(0);
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
    /// `oid` is NULL or writable; `buffer_out` is readable and writable,
    /// and `*buffer_out` .. `buffer_end` a readable range (of one
    /// allocation) that nothing writes during the call; `header` NULL or
    /// NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object__parse_oid_header(
        mut oid: Out<'_, Oid>,
        buffer_out: *mut *const c_char,
        buffer_end: *const c_char,
        header: CStrArg<'_>,
        oid_type: crate::oid::git_oid_t,
    ) -> c_int {
        barrier(|| {
            // SAFETY: `buffer_out` is readable (this export's contract).
            let start = unsafe { *buffer_out };
            let len = (buffer_end as usize).saturating_sub(start as usize);
            // SAFETY: `start .. start + len` is within the caller's
            // readable, unwritten range (`len` is 0 if it is empty).
            let buf = unsafe { bytes(start, len) };
            // (C's `strlen` dereferences a NULL `header`.)
            let header = git_arg!(header.get(), "header", -1).to_bytes();
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
                    oid.write(id);
                    // SAFETY: `buffer_out` is writable (this export's
                    // contract); `start + n` stays within the range.
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
    /// `buf` is a valid `git_str` that no other thread uses during the
    /// call; `header` a NUL-terminated string, which may lie in `buf`'s
    /// buffer, otherwise unwritten during the call; `oid` NULL or a
    /// readable id.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_object__write_oid_header(
        buf: *mut GitStr,
        header: *const c_char,
        oid: Option<&Oid>,
    ) -> c_int {
        barrier(|| {
            let oid = git_arg!(oid, "oid", -1);
            let id = *oid;
            // A header inside `buf`'s own buffer (C's `git_str_puts`
            // allows it) is copied first; any other is borrowed.
            // SAFETY: `buf` is a valid `git_str` (this export's contract);
            // this `&` ends at once, before anything writes `buf`.
            let s = unsafe { &*buf };
            let (base, cap) = (s.as_ptr() as usize, s.capacity());
            let h = header as usize;
            let inside = h >= base && h < base.wrapping_add(cap);
            // SAFETY: `header` is a NUL-terminated string (this export's
            // contract); nothing writes it while `hb` lives: `buf`'s
            // buffer, which may hold it, is written only after `hb` is
            // copied (`inside`) or, if it is outside, not at all.
            let hb = unsafe { CStr::from_ptr(header) }.to_bytes();
            let copy;
            let header: &[u8] = if inside {
                copy = hb.to_vec();
                &copy
            } else {
                hb
            };
            // SAFETY: a valid `git_str`, used by no one else during the
            // call; `header` does not point into it (copied if it did).
            status(write_oid_header(unsafe { &mut *buf }, header, &id))
        })
    }

    /// `GIT_BUF_WRAP_PRIVATE`: run `f` on a `git_str` that takes `out`'s
    /// buffer, and give the result back to `out`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or a valid `git_buf`, which nothing else uses during
    /// the call (`f` included).
    pub(crate) unsafe fn buf_wrap(
        out: *mut GitBuf,
        f: impl FnOnce(&mut GitStr) -> Result<()>,
    ) -> c_int {
        let mut s = core::mem::MaybeUninit::<GitStr>::uninit();
        // SAFETY: `s` is a writable local; `out` NULL or a valid `git_buf`
        // (this fn's contract).
        let mut error = unsafe { crate::buf::ffi::git_buf_tostr(s.as_mut_ptr(), out) };
        if error != 0 {
            return error;
        }
        // SAFETY: `git_buf_tostr` succeeded, so it initialized `s`.
        let mut s = unsafe { s.assume_init() };
        error = status(f(&mut s));
        if error == 0 {
            // SAFETY: `out` is a valid `git_buf` (it was not NULL:
            // `git_buf_tostr` succeeded), and `s` a valid `git_str`.
            error = unsafe { crate::buf::ffi::git_buf_fromstr(out, Some(&mut s)) };
        }
        drop(s);
        error
    }

    /// `git_error__check_version` (`GIT_ERROR_CHECK_VERSION`).
    pub(crate) fn check_version(
        structure: Option<&c_uint>,
        expected_max: c_uint,
        name: &str,
    ) -> Result<()> {
        let Some(&actual) = structure else {
            return Ok(());
        };
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
        // `(int64_t)` of a huge length, and the longest header.
        let n = format_object_header(&mut h, u64::MAX, GIT_OBJECT_BLOB).unwrap();
        assert_eq!(&h[..n], b"blob -1\0");
        let n = format_object_header(&mut h, 1 << 63, GIT_OBJECT_COMMIT).unwrap();
        assert_eq!(&h[..n], b"commit -9223372036854775808\0");
        let n = format_object_header(&mut h, 0, 0).unwrap();
        assert_eq!(&h[..n], b" 0\0");
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
    fn write_oid_header_with_its_own_buffer_as_header() {
        let _g = test_support::init();
        let id =
            Oid::parse_hex(b"0123456789012345678901234567890123456789", OidType::Sha1).unwrap();
        let mut s = GitStr::new();
        s.puts(b"tree ").unwrap();
        let header = s.as_ptr();
        // SAFETY: a valid `git_str`, its own NUL-terminated contents as the
        // header (as C may pass), a valid id.
        let rc = unsafe { ffi::git_object__write_oid_header(&raw mut s, header, Some(&id)) };
        assert_eq!(rc, 0);
        assert_eq!(
            s.as_bytes(),
            b"tree tree 0123456789012345678901234567890123456789\n"
        );
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
                Out::from(&mut o),
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
                Out::from(&mut o),
                data.as_ptr().cast(),
                data.len(),
                GIT_OBJECT_BLOB,
                1,
            )
        };
        assert_eq!(rc, 0);
        // SAFETY: a live blob, read only.
        let b = unsafe { o.cast::<crate::blob::GitBlob>().as_ref() };
        // SAFETY: a live blob.
        let size = unsafe { crate::blob::ffi::git_blob_rawsize(b) };
        assert_eq!(size, data.len() as u64);
        // SAFETY: a live blob.
        let content = unsafe { crate::blob::ffi::git_blob_rawcontent(b) };
        assert_eq!(content, data.as_ptr().cast());
        // SAFETY: a live object, read only.
        let obj = unsafe { o.as_ref() };
        // SAFETY: a live object.
        let id = unsafe { ffi::git_object_id(obj) };
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
                Out::from(&mut o),
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
