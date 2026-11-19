//! The exports of `odb.c`.
//!
//! # Contract
//!
//! As C's: a `git_odb *` is one `git_odb_new*` / `git_odb_open*` made and
//! that is not freed, used from any thread (the odb is thread-safe); a
//! `git_odb_object *` is one an odb returned (or a C-made object with a
//! `git__allocator` buffer), immutable but for its count; a backend added
//! to an odb belongs to it; streams are used by one thread at a time.
//! Out-parameters are writable; ids are readable `git_oid`s.
//!
//! A live odb is taken as `Option<&GitOdb>`: C writes only its `rc`
//! (atomics), `options` is never written once made, and the rest is behind
//! its mutex, its cache's locks or atomics, so a shared reference may cover
//! it for the call while other threads and re-entering callbacks use it;
//! nothing frees it during the call (the caller holds a reference). A NULL
//! odb is an invalid argument (C dereferenced some unchecked).

use crate::git_arg;
use crate::util::cabi::{CStrArg, Out};
use core::ffi::{c_char, c_int, c_uint, c_void};
use core::ptr::NonNull;

use super::*;
use crate::util::export::{barrier, bytes, status};
use ffibox::CShared;

/// `git_odb__format_object_header`.
///
/// # Safety
///
/// `written` is NULL or writable; `hdr` is writable for `hdr_size` bytes
/// (which need not be initialized) and does not overlap `written`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb__format_object_header(
    mut written: Out<'_, usize>,
    hdr: *mut c_char,
    hdr_size: usize,
    obj_len: git_object_size_t,
    obj_type: git_object_t,
) -> c_int {
    barrier(|| {
        let (tmp, n) = header_bytes(obj_len, obj_type);
        let hdr_max = hdr_size.min(c_int::MAX as usize - 2);
        if hdr_max > 0 {
            let k = n.min(hdr_max - 1);
            // SAFETY: `hdr` is writable for `hdr_size` bytes (this
            // export's contract) and `k < hdr_max <= hdr_size`; raw writes
            // (it may be uninitialized); `tmp` is a distinct local, readable
            // for `n >= k` bytes.
            unsafe { core::ptr::copy_nonoverlapping(tmp.as_ptr(), hdr.cast::<u8>(), k) };
            // SAFETY: as above: byte `k < hdr_size` is writable.
            unsafe { hdr.wrapping_add(k).write(0) };
        }
        if n >= hdr_max {
            return header_error().raw();
        }
        written.write(n + 1);
        0
    })
}

/// `git_odb_object__free`.
///
/// # Safety
///
/// `object` is NULL or an odb object nothing uses again (made by the odb,
/// or by C with `git__malloc` and a `git__allocator` buffer).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_object__free(object: *mut c_void) {
    barrier(|| {
        if let Some(o) = NonNull::new(object.cast::<GitOdbObject>()) {
            // SAFETY: `o` is such an object, not used again (this export's
            // contract), as `odb_object_free` requires.
            unsafe { odb_object_free(o) }
        }
    })
}

// The accessors take `Option<&GitOdbObject>`: a live odb object is not
// written once shared but for its atomic count (module docs), and the
// caller's reference keeps it alive for the call. A NULL object, which C
// dereferenced, is an invalid argument.

/// `git_odb_object_id`.
///
/// # Safety
///
/// `object` is NULL or a live object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_object_id(object: Option<&GitOdbObject>) -> *const Oid {
    &raw const git_arg!(object, "object", core::ptr::null()).cached.oid
}

/// `git_odb_object_data`.
///
/// # Safety
///
/// `object` is NULL or a live object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_object_data(object: Option<&GitOdbObject>) -> *const c_void {
    git_arg!(object, "object", core::ptr::null()).buffer
}

/// `git_odb_object_size`.
///
/// # Safety
///
/// `object` is NULL or a live object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_object_size(object: Option<&GitOdbObject>) -> usize {
    git_arg!(object, "object", 0).cached.size
}

/// `git_odb_object_type`.
///
/// # Safety
///
/// `object` is NULL or a live object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_object_type(object: Option<&GitOdbObject>) -> git_object_t {
    git_object_t::from(git_arg!(object, "object", GIT_OBJECT_INVALID).cached.type_)
}

/// `git_odb_object_dup`.
///
/// # Safety
///
/// `dest` is NULL or writable; `source` is a live object.
///
/// (`source` stays a raw pointer: it is handed back to C, which frees the
/// object through it.)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_object_dup(
    mut dest: Out<'_, *mut GitOdbObject>,
    source: *mut GitOdbObject,
) -> c_int {
    barrier(|| {
        // SAFETY: `source` is a live object (this export's contract); the
        // `&` covers only its atomic count.
        unsafe { (*source).cached.refcount.inc() };
        dest.write(source);
        0
    })
}

/// `git_odb_object_free`.
///
/// # Safety
///
/// `object` is NULL or a live object whose reference the caller gives up.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_object_free(object: *mut GitOdbObject) {
    barrier(|| {
        if let Some(o) = NonNull::new(object) {
            // SAFETY: `o` is a live odb object (a cached object stored raw,
            // or of flags 0 with `git__allocator` memory) whose reference
            // the caller gives up (this export's contract).
            unsafe { cache::decref(o.cast()) }
        }
    })
}

/// `git_odb_options_init`.
///
/// # Safety
///
/// `opts` is NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_options_init(
    mut opts: Out<'_, GitOdbOptions>,
    version: c_uint,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!opts.is_null(), "opts", -1);
        if let Err(e) = check_version(Some(version), GIT_ODB_OPTIONS_VERSION, "git_odb_options") {
            return e.raw();
        }
        opts.write(GitOdbOptions::default());
        0
    })
}

/// `git_odb_new_ext`.
///
/// # Safety
///
/// `out` is writable; `opts` is NULL or a readable `git_odb_options`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_new_ext(
    mut out: Out<'_, *mut GitOdb>,
    opts: Option<&GitOdbOptions>,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        match GitOdb::new(opts) {
            Ok(db) => {
                out.write(CShared::into_raw(db));
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_odb_new`.
///
/// # Safety
///
/// `out` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_new(out: Out<'_, *mut GitOdb>) -> c_int {
    // SAFETY: `out` is NULL or writable (this export's contract), and no
    // options is allowed.
    unsafe { git_odb_new_ext(out, None) }
}

/// `add_backend_internal` for the exports.
///
/// # Safety
///
/// `backend` is NULL or a live backend table whose ownership passes to the
/// odb on success.
unsafe fn add_backend(
    odb_: Option<&GitOdb>,
    backend: *mut GitOdbBackend,
    priority: c_int,
    alternate: bool,
) -> c_int {
    barrier(|| {
        let db = git_arg!(odb_, "odb", -1);
        let Some(b) = NonNull::new(backend) else {
            let _ = crate::util::errors::assert_arg_failed("backend");
            return -1;
        };
        // SAFETY: `b` is a live backend table whose ownership passes to the
        // odb on success (the exports' contract), as `add_backend` needs.
        status(unsafe { db.add_backend(b, priority, alternate) })
    })
}

/// `git_odb_add_backend`.
///
/// # Safety
///
/// `odb` is NULL or a live odb; `backend` is NULL or a live backend table
/// whose ownership passes to the odb on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_add_backend(
    odb: Option<&GitOdb>,
    backend: *mut GitOdbBackend,
    priority: c_int,
) -> c_int {
    // SAFETY: `backend` is NULL or a live table that passes to the odb on
    // success (this export's contract).
    unsafe { add_backend(odb, backend, priority, false) }
}

/// `git_odb_add_alternate`.
///
/// # Safety
///
/// As [`git_odb_add_backend`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_add_alternate(
    odb: Option<&GitOdb>,
    backend: *mut GitOdbBackend,
    priority: c_int,
) -> c_int {
    // SAFETY: as in `git_odb_add_backend`.
    unsafe { add_backend(odb, backend, priority, true) }
}

/// `git_odb_num_backends`.
///
/// # Safety
///
/// `db` is NULL or a live odb.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_num_backends(db: Option<&GitOdb>) -> usize {
    barrier(|| {
        // C returns `GIT_ASSERT_ARG`'s -1 as a `size_t`.
        git_arg!(db, "odb", usize::MAX).num_backends()
    })
}

/// `git_odb_get_backend`.
///
/// # Safety
///
/// `out` is writable; `db` is a live odb.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_get_backend(
    mut out: Out<'_, *mut GitOdbBackend>,
    db: Option<&GitOdb>,
    pos: usize,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        let db = git_arg!(db, "odb", -1);
        match db.get_backend(pos) {
            Ok(b) => {
                out.write(b.as_ptr());
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_odb__add_default_backends`.
///
/// # Safety
///
/// `db` is NULL or a live odb; `objects_dir` NULL or a NUL-terminated
/// string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb__add_default_backends(
    db: Option<&GitOdb>,
    objects_dir: CStrArg<'_>,
    as_alternates: bool,
    alternate_depth: c_int,
) -> c_int {
    barrier(|| {
        let db = git_arg!(db, "db", -1);
        let dir = objects_dir.get().unwrap_or(c"");
        status(db.add_default_backends(dir, as_alternates, alternate_depth))
    })
}

/// `git_odb_add_disk_alternate`.
///
/// # Safety
///
/// As [`git_odb__add_default_backends`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_add_disk_alternate(
    db: Option<&GitOdb>,
    path: CStrArg<'_>,
) -> c_int {
    // SAFETY: the same arguments, under the same contract.
    unsafe { git_odb__add_default_backends(db, path, true, 0) }
}

/// `git_odb_set_commit_graph`.
///
/// # Safety
///
/// `db` is NULL or a live odb; `cgraph` is NULL or a commit-graph whose
/// ownership passes to the odb.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_set_commit_graph(
    db: Option<&GitOdb>,
    cgraph: *mut c_void,
) -> c_int {
    barrier(|| {
        let db = git_arg!(db, "odb", -1);
        // SAFETY: a non-NULL `cgraph` is a commit-graph whose ownership
        // passes to the odb (this export's contract), which `from_raw`
        // adopts.
        let g = NonNull::new(cgraph).map(|p| unsafe { CommitGraph::from_raw(p) });
        db.set_commit_graph(g);
        0
    })
}

/// `git_odb_open_ext`.
///
/// # Safety
///
/// `out` is writable; `objects_dir` is a NUL-terminated string; `opts` is
/// NULL or a readable `git_odb_options`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_open_ext(
    mut out: Out<'_, *mut GitOdb>,
    objects_dir: CStrArg<'_>,
    opts: Option<&GitOdbOptions>,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        let objects_dir = git_arg!(objects_dir.get(), "objects_dir", -1);
        if let Err(e) = check_version(
            opts.map(|o| o.version),
            GIT_ODB_OPTIONS_VERSION,
            "git_odb_options",
        ) {
            return e.raw();
        }
        out.write(core::ptr::null_mut());
        let db = match GitOdb::new(opts) {
            Ok(db) => db,
            Err(_) => return -1,
        };
        let dir = objects_dir;
        if db.add_default_backends(dir, false, 0).is_err() {
            return -1;
        }
        out.write(CShared::into_raw(db));
        0
    })
}

/// `git_odb_open`.
///
/// # Safety
///
/// As [`git_odb_open_ext`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_open(
    out: Out<'_, *mut GitOdb>,
    objects_dir: CStrArg<'_>,
) -> c_int {
    // SAFETY: the same arguments, under the same contract, and no options.
    unsafe { git_odb_open_ext(out, objects_dir, None) }
}

/// `git_odb__set_caps`.
///
/// # Safety
///
/// `db` is NULL or a live odb.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb__set_caps(db: Option<&GitOdb>, caps: c_int) -> c_int {
    barrier(|| status(git_arg!(db, "odb", -1).set_caps(caps)))
}

/// `git_odb_free`.
///
/// # Safety
///
/// `db` is NULL or a live odb whose reference the caller gives up.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_free(db: *mut GitOdb) {
    // SAFETY: `db` is NULL or a live odb made by `CShared` (this crate's
    // `git_odb_new*`), whose reference the caller gives up (this export's
    // contract).
    barrier(|| unsafe { CShared::release(db) })
}

/// `git_odb__get_commit_graph_file`.
///
/// # Safety
///
/// `out` is NULL or writable; `db` is NULL or a live odb.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb__get_commit_graph_file(
    mut out: Out<'_, *mut c_void>,
    db: Option<&GitOdb>,
) -> c_int {
    barrier(|| match git_arg!(db, "db", -1).get_commit_graph_file() {
        Ok(f) => {
            out.write(f);
            0
        }
        Err(e) => e.raw(),
    })
}

/// `git_odb__freshen`.
///
/// # Safety
///
/// `db` is NULL or a live odb; `id` NULL or a readable id.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb__freshen(db: Option<&GitOdb>, id: Option<&Oid>) -> c_int {
    barrier(|| {
        let db = git_arg!(db, "db", -1);
        let id = git_arg!(id, "id", -1);
        c_int::from(db.freshen(id))
    })
}

/// `git_odb_exists`.
///
/// # Safety
///
/// As [`git_odb__freshen`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_exists(db: Option<&GitOdb>, id: Option<&Oid>) -> c_int {
    // SAFETY: the same arguments, under the same contract.
    unsafe { git_odb_exists_ext(db, id, 0) }
}

/// `git_odb_exists_ext`.
///
/// # Safety
///
/// As [`git_odb__freshen`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_exists_ext(
    db: Option<&GitOdb>,
    id: Option<&Oid>,
    flags: c_uint,
) -> c_int {
    barrier(|| {
        let db = git_arg!(db, "db", -1);
        let id = git_arg!(id, "id", -1);
        c_int::from(db.exists_ext(id, flags))
    })
}

/// `git_odb_exists_prefix`.
///
/// # Safety
///
/// `out` is NULL or writable; `db` NULL or a live odb; `short_id` NULL or
/// a readable id, which may be `out` (C expands an id in place: it copies
/// the prefix first and writes `out` last, so `short_id` stays a raw
/// pointer, read once before `out` is written).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_exists_prefix(
    mut out: Out<'_, Oid>,
    db: Option<&GitOdb>,
    short_id: *const Oid,
    len: usize,
) -> c_int {
    barrier(|| {
        let db = git_arg!(db, "db", -1);
        git_assert_arg!(!short_id.is_null(), "short_id", -1);
        // SAFETY: `short_id` is non-null, so a readable, aligned id (this
        // export's contract), copied here before `out` (which may be the
        // same id) is written; no reference to it is kept.
        let short_id = unsafe { short_id.read() };
        match db.exists_prefix(&short_id, len) {
            Ok(id) => {
                out.write(id);
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_odb_expand_ids`.
///
/// # Safety
///
/// `db` is NULL or a live odb; `ids` is NULL or `count` writable entries.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_expand_ids(
    db: Option<&GitOdb>,
    ids: *mut GitOdbExpandId,
    count: usize,
) -> c_int {
    barrier(|| {
        let db = git_arg!(db, "db", -1);
        git_assert_arg!(!ids.is_null(), "ids", -1);
        assert!(
            count <= isize::MAX as usize / size_of::<GitOdbExpandId>(),
            "count"
        );
        // SAFETY: `ids` is non-NULL, so `count` initialized, aligned
        // entries, readable and writable, that nothing else reads or writes
        // during the call (this export's contract); `count * size` is at
        // most `isize::MAX` (checked above).
        let ids = unsafe { core::slice::from_raw_parts_mut(ids, count) };
        status(db.expand_ids(ids))
    })
}

/// `git_odb_read_header`.
///
/// # Safety
///
/// `len_p` and `type_p` are NULL or writable; otherwise as
/// [`git_odb__freshen`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_read_header(
    len_p: Out<'_, usize>,
    type_p: Out<'_, git_object_t>,
    db: Option<&GitOdb>,
    id: Option<&Oid>,
) -> c_int {
    let mut obj: *mut GitOdbObject = core::ptr::null_mut();
    // SAFETY: `obj` is a local; the other arguments are this export's,
    // under the same contract.
    let r = unsafe { git_odb__read_header_or_object(Out::from(&mut obj), len_p, type_p, db, id) };
    if let Some(o) = NonNull::new(obj) {
        // SAFETY: `o` is the object the call returned, with a reference
        // that is ours and given up here.
        barrier(|| unsafe { cache::decref(o.cast()) });
    }
    r
}

/// `git_odb__read_header_or_object`.
///
/// # Safety
///
/// `out`, `len_p` and `type_p` are writable; otherwise as
/// [`git_odb__freshen`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb__read_header_or_object(
    mut out: Out<'_, *mut GitOdbObject>,
    mut len_p: Out<'_, usize>,
    mut type_p: Out<'_, git_object_t>,
    db: Option<&GitOdb>,
    id: Option<&Oid>,
) -> c_int {
    barrier(|| {
        let db = git_arg!(db, "db", -1);
        let id = git_arg!(id, "id", -1);
        git_assert_arg!(!out.is_null(), "out", -1);
        git_assert_arg!(!len_p.is_null(), "len_p", -1);
        git_assert_arg!(!type_p.is_null(), "type_p", -1);
        out.write(core::ptr::null_mut());
        match db.read_header_or_object(id) {
            Ok((len, t, obj)) => {
                len_p.write(len);
                type_p.write(t);
                out.write(obj.map_or(core::ptr::null_mut(), NonNull::as_ptr));
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_odb_read`.
///
/// # Safety
///
/// `out` is NULL or writable; otherwise as [`git_odb__freshen`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_read(
    mut out: Out<'_, *mut GitOdbObject>,
    db: Option<&GitOdb>,
    id: Option<&Oid>,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        let db = git_arg!(db, "db", -1);
        let id = git_arg!(id, "id", -1);
        match db.read(id) {
            Ok(o) => {
                out.write(o.as_ptr());
                0
            }
            Err(e) => {
                // C leaves `*out` NULL when the cache missed.
                if !id.is_zero() {
                    out.write(core::ptr::null_mut());
                }
                e.raw()
            }
        }
    })
}

/// `git_odb_read_prefix`.
///
/// # Safety
///
/// `out` is NULL or writable; otherwise as [`git_odb__freshen`] (for
/// `short_id`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_read_prefix(
    mut out: Out<'_, *mut GitOdbObject>,
    db: Option<&GitOdb>,
    short_id: Option<&Oid>,
    len: usize,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        let db = git_arg!(db, "db", -1);
        // C's checks come first; it reads `short_id` only after the length
        // check (and crashes on NULL there: here an invalid argument).
        if len < crate::oid::GIT_OID_MINPREFIXLEN {
            return crate::odb::error_ambiguous(b"prefix length too short").raw();
        }
        let short_id = git_arg!(short_id, "short_id", -1);
        match db.read_prefix(short_id, len) {
            Ok(o) => {
                out.write(o.as_ptr());
                0
            }
            Err(e) => {
                // C set `*out` to the cache's answer (NULL) for a full id.
                if len >= crate::oid::GIT_OID_MINPREFIXLEN && len >= db.hexsize() {
                    out.write(core::ptr::null_mut());
                }
                e.raw()
            }
        }
    })
}

/// `git_odb_foreach`.
///
/// # Safety
///
/// `db` is NULL or a live odb; `cb` is NULL or may be called with
/// `payload` (and any id), on this thread, during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_foreach(
    db: Option<&GitOdb>,
    cb: ForeachCb,
    payload: *mut c_void,
) -> c_int {
    barrier(|| {
        let db = git_arg!(db, "db", -1);
        // SAFETY: `cb` may be called with `payload` during the call (this
        // export's contract), which is what `foreach_raw` needs.
        unsafe { db.foreach_raw(cb, payload) }
    })
}

/// `git_odb_write`.
///
/// # Safety
///
/// `oid` is NULL or writable; `db` NULL or a live odb; `data` NULL or
/// readable for `len` bytes, which nothing writes during the call (and
/// `oid` is not among them).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_write(
    mut oid: Out<'_, Oid>,
    db: Option<&GitOdb>,
    data: *const c_void,
    len: usize,
    type_: git_object_t,
) -> c_int {
    barrier(|| {
        // C reads `db->options` before these checks (a NULL `db` crashed).
        git_assert_arg!(!oid.is_null(), "oid", -1);
        let db = git_arg!(db, "db", -1);
        if data.is_null() && len != 0 {
            // `git_object_id_from_buffer`'s "invalid object"; `*oid` is
            // not written.
            let oid_type = db.oid_type();
            return hash_object(None, len, type_, oid_type).map_or_else(|e| e.raw(), |_| -1);
        }
        // SAFETY: `data` is non-NULL here, or `len` is 0; a non-NULL `data`
        // is readable for `len` bytes that nothing writes during the call
        // (this export's contract).
        let data = unsafe { bytes(data.cast(), len) };
        let mut id = None;
        let r = db.write(data, type_, &mut id);
        if let Some(id) = id {
            // C wrote `*oid` once hashed, whatever followed.
            oid.write(id);
        }
        status(r)
    })
}

/// `git_odb_open_wstream`.
///
/// # Safety
///
/// `stream` is NULL or writable; `db` NULL or a live odb.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_open_wstream(
    mut stream: Out<'_, *mut GitOdbStream>,
    db: Option<&GitOdb>,
    size: git_object_size_t,
    type_: git_object_t,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!stream.is_null(), "stream", -1);
        let db = git_arg!(db, "db", -1);
        match db.open_wstream(size, type_) {
            Ok(s) => {
                stream.write(s.as_ptr());
                0
            }
            Err(ErrorCode::PASSTHROUGH) => 0,
            Err(e) => e.raw(),
        }
    })
}

/// `git_odb_stream__invalid_length`, for a stream's declared and received
/// sizes.
fn invalid_length(declared: git_object_size_t, received: git_object_size_t, action: &str) -> c_int {
    let _ = git_error_set!(
        ErrorClass::ODB,
        "cannot {action} - Invalid length. {} was expected. The total size of the received chunks amounts to {}.",
        declared as i64,
        received as i64
    );
    -1
}

/// `git_odb_stream_write`.
///
/// # Safety
///
/// `stream` is a live stream from `git_odb_open_wstream` (or a backend's,
/// with a NULL `hash_ctx`), used by no other thread or reference during
/// the call; `buffer` is readable for `len` bytes, which nothing writes
/// during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_stream_write(
    stream: *mut GitOdbStream,
    buffer: *const c_char,
    len: usize,
) -> c_int {
    barrier(|| {
        // SAFETY: `stream` is a live stream, the caller's alone (this
        // export's contract): a read of one field in place. (C
        // dereferenced a NULL `stream` too.)
        let ctx = unsafe { (*stream).hash_ctx };
        // SAFETY: as above: NULL or the odb's initialized context, which
        // nothing else accesses during the call.
        if let Some(ctx) = unsafe { ctx.cast::<crate::util::hash::HashCtx>().as_mut() } {
            // SAFETY: as above; the caller's `buffer`.
            let _ =
                unsafe { crate::util::hash::ffi::git_hash_update(Some(ctx), buffer.cast(), len) };
        }
        // SAFETY: the caller's contract: the stream is the caller's alone
        // during the call, and no slot of it runs while this `&mut` lives
        // (it ends before `stream_write_slot`).
        let s = unsafe { &mut *stream };
        s.received_bytes = s.received_bytes.wrapping_add(len as u64);
        if s.received_bytes > s.declared_size {
            return invalid_length(s.declared_size, s.received_bytes, "stream_write()");
        }
        // SAFETY: `stream` is live (no reference to it is live now), and
        // `buffer` readable for `len` bytes (this export's contract).
        unsafe { stream_write_slot(stream, buffer, len) }
    })
}

/// `git_odb_stream_finalize_write`.
///
/// # Safety
///
/// `out` is writable and readable (the backend's `finalize_write` reads
/// the id written to it); `stream` is a live stream from
/// `git_odb_open_wstream`, used by no other thread or reference during the
/// call, whose backend is live and belongs to a live odb (or to none).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_stream_finalize_write(
    out: *mut Oid,
    stream: *mut GitOdbStream,
) -> c_int {
    barrier(|| {
        // SAFETY: `stream` is live and the caller's alone (this export's
        // contract); the `&` ends with this statement's copies, before any
        // slot runs.
        let s = unsafe { &*stream };
        let (received, declared, ctx, oid_type, backend) = (
            s.received_bytes,
            s.declared_size,
            s.hash_ctx,
            s.oid_type,
            s.backend,
        );
        if received != declared {
            return invalid_length(declared, received, "stream_finalize_write()");
        }
        let mut id = [0u8; crate::oid::GIT_OID_MAX_SIZE];
        // SAFETY: NULL or the odb's initialized context, which nothing
        // else accesses during the call (the caller's stream).
        if let Some(ctx) = unsafe { ctx.cast::<crate::util::hash::HashCtx>().as_mut() } {
            // SAFETY: the odb's context; `id` holds any digest.
            let _ = unsafe { crate::util::hash::ffi::git_hash_final(id.as_mut_ptr(), Some(ctx)) };
        }
        let oid = Oid::from_raw_parts(oid_type as u8, id);
        // SAFETY: `out` is writable (this export's contract).
        unsafe { out.write(oid) };
        // SAFETY: the stream's backend is a live table (this export's
        // contract): one field read in place.
        let db = unsafe { (*backend).odb };
        // SAFETY: a backend's `odb` is NULL or the live odb it belongs to
        // (this export's contract), only read through `&` (module docs).
        if let Some(db) = unsafe { db.as_ref() }
            && db.freshen(&oid)
        {
            return 0;
        }
        // SAFETY: `stream` is live (no reference to it is live now): one
        // field read in place.
        match unsafe { (*stream).finalize_write } {
            // SAFETY: the backend's own slot, called with its live stream
            // and `out`, which holds the id it reads.
            Some(f) => unsafe { f(stream, out) },
            None => git_error_set!(ErrorClass::ODB, "stream does not support writing").raw(),
        }
    })
}

/// `git_odb_stream_read`.
///
/// # Safety
///
/// `stream` is a live read stream, used by no other thread during the
/// call; `buffer` is writable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_stream_read(
    stream: *mut GitOdbStream,
    buffer: *mut c_char,
    len: usize,
) -> c_int {
    barrier(|| {
        // SAFETY: `stream` is live (this export's contract): one field read
        // in place.
        match unsafe { (*stream).read } {
            // SAFETY: the backend's own slot, called with its live stream
            // and the caller's writable `buffer` of `len` bytes.
            Some(r) => unsafe { r(stream, buffer, len) },
            None => git_error_set!(ErrorClass::ODB, "stream does not support reading").raw(),
        }
    })
}

/// `git_odb_stream_free`.
///
/// # Safety
///
/// `stream` is NULL or a live stream nothing uses again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_stream_free(stream: *mut GitOdbStream) {
    barrier(|| {
        if !stream.is_null() {
            // SAFETY: a non-NULL `stream` is live and not used again (this
            // export's contract); its `hash_ctx` is NULL or the odb's
            // `git__allocator` context.
            unsafe { stream_free(stream) }
        }
    })
}

/// `git_odb_open_rstream`.
///
/// # Safety
///
/// `stream`, `len` and `type_` are NULL or writable; `db` NULL or a live
/// odb; `oid` NULL or a readable id.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_open_rstream(
    mut stream: Out<'_, *mut GitOdbStream>,
    mut len: Out<'_, usize>,
    mut type_: Out<'_, git_object_t>,
    db: Option<&GitOdb>,
    oid: Option<&Oid>,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!stream.is_null(), "stream", -1);
        let db = git_arg!(db, "db", -1);
        // (`oid` is checked where a backend would read it, as C reaches it.)
        match db.open_rstream(oid) {
            // C: 0, with `*stream` as the backend left it (untouched).
            Ok(None) => 0,
            Ok(Some((s, l, t))) => {
                stream.write(s.as_ptr());
                len.write(l);
                type_.write(t);
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_odb_write_pack`.
///
/// # Safety
///
/// `out` is NULL or writable; `db` NULL or a live odb; `progress_cb` is
/// NULL or may be called with `progress_payload` until the writepack is
/// freed, on whichever thread uses the writepack.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_write_pack(
    mut out: Out<'_, *mut GitOdbWritepack>,
    db: Option<&GitOdb>,
    progress_cb: IndexerProgressCb,
    progress_payload: *mut c_void,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        let db = git_arg!(db, "db", -1);
        // SAFETY: `progress_cb` may be called with `progress_payload` while
        // the writepack lives (this export's contract), as `write_pack`
        // requires.
        match unsafe { db.write_pack(progress_cb, progress_payload) } {
            Ok(w) => {
                if !w.is_null() {
                    out.write(w);
                }
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_odb_write_multi_pack_index`.
///
/// # Safety
///
/// `db` is NULL or a live odb.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_write_multi_pack_index(db: Option<&GitOdb>) -> c_int {
    barrier(|| status(git_arg!(db, "db", -1).write_multi_pack_index()))
}

/// `git_odb_backend_data_alloc`: `git__malloc(len)`.
///
/// # Safety
///
/// None: `backend` is not read.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_backend_data_alloc(
    backend: *mut GitOdbBackend,
    len: usize,
) -> *mut c_void {
    let _ = backend;
    barrier(|| alloc::malloc(len))
}

/// `git_odb_backend_data_free`: `git__free(data)`.
///
/// # Safety
///
/// `data` is NULL or from `git_odb_backend_data_alloc` (`git__allocator`),
/// not used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_backend_data_free(backend: *mut GitOdbBackend, data: *mut c_void) {
    let _ = backend;
    // SAFETY: `data` is NULL or a `git__allocator` block nothing uses again
    // (this export's contract).
    barrier(|| unsafe { alloc::free(data) })
}

/// `git_odb_refresh`.
///
/// # Safety
///
/// `db` is NULL or a live odb.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_refresh(db: Option<&GitOdb>) -> c_int {
    barrier(|| status(git_arg!(db, "db", -1).refresh()))
}

/// `git_odb__error_mismatch`.
///
/// # Safety
///
/// Both ids are readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb__error_mismatch(
    expected: Option<&Oid>,
    actual: Option<&Oid>,
) -> c_int {
    barrier(|| {
        let actual = git_arg!(actual, "actual", -1);
        let expected = git_arg!(expected, "expected", -1);
        error_mismatch(expected, actual).raw()
    })
}

/// `git_odb__error_notfound`.
///
/// # Safety
///
/// `message` is NULL or a NUL-terminated string; `oid` is NULL or a
/// readable id that nothing writes during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb__error_notfound(
    message: CStrArg<'_>,
    oid: Option<&Oid>,
    oid_len: usize,
) -> c_int {
    barrier(|| error_notfound(message.to_bytes(), oid, oid_len).raw())
}

/// `git_odb__error_ambiguous`.
///
/// # Safety
///
/// `message` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb__error_ambiguous(message: CStrArg<'_>) -> c_int {
    barrier(|| error_ambiguous(message.to_bytes()).raw())
}

/// `git_odb_init_backend`.
///
/// # Safety
///
/// `backend` is NULL or writable for a `git_odb_backend`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_init_backend(
    mut backend: Out<'_, GitOdbBackend>,
    version: c_uint,
) -> c_int {
    barrier(|| {
        if let Err(e) = check_version(Some(version), GIT_ODB_BACKEND_VERSION, "git_odb_backend") {
            return e.raw();
        }
        backend.write(GitOdbBackend {
            version: GIT_ODB_BACKEND_VERSION,
            ..GitOdbBackend::default()
        });
        0
    })
}
