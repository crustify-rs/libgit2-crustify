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

use crate::git_arg;
use crate::util::cabi::{CStrArg, Out};
use core::ffi::{c_char, c_int, c_uint, c_void};
use core::ptr::NonNull;

use super::*;
use crate::util::export::{barrier, bytes, status};
use ffibox::CShared;

/// The odb at `db`.
///
/// # Safety
///
/// `db` is a live odb (the module's contract), alive for `'a`.
unsafe fn odb<'a>(db: *mut GitOdb) -> &'a GitOdb {
    // SAFETY: the caller's contract. C reads `rc` (atomics) and `options`
    // (never written once made) only; the rest is synchronized.
    unsafe { &*db }
}

/// `git_odb__format_object_header`.
///
/// # Safety
///
/// `written` is writable; `hdr` is writable for `hdr_size` bytes.
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
            // SAFETY: the caller's contract: `hdr` is writable for
            // `hdr_size >= k + 1` bytes (raw writes: it may be
            // uninitialized); `tmp` is a distinct local.
            unsafe { core::ptr::copy_nonoverlapping(tmp.as_ptr(), hdr.cast::<u8>(), k) };
            // SAFETY: as above: `k < hdr_size`.
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
/// `object` is NULL or an object nothing uses again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_object__free(object: *mut c_void) {
    barrier(|| {
        if let Some(o) = NonNull::new(object.cast::<GitOdbObject>()) {
            // SAFETY: the caller's contract.
            unsafe { odb_object_free(o) }
        }
    })
}

/// `git_odb_object_id`.
///
/// # Safety
///
/// `object` is a live object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_object_id(object: *mut GitOdbObject) -> *const Oid {
    // SAFETY: the caller's contract; a place projection.
    unsafe { &raw const (*object).cached.oid }
}

/// `git_odb_object_data`.
///
/// # Safety
///
/// `object` is a live object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_object_data(object: *mut GitOdbObject) -> *const c_void {
    // SAFETY: the caller's contract.
    unsafe { (*object).buffer }
}

/// `git_odb_object_size`.
///
/// # Safety
///
/// `object` is a live object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_object_size(object: *mut GitOdbObject) -> usize {
    // SAFETY: the caller's contract.
    unsafe { (*object).cached.size }
}

/// `git_odb_object_type`.
///
/// # Safety
///
/// `object` is a live object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_object_type(object: *mut GitOdbObject) -> git_object_t {
    // SAFETY: the caller's contract.
    git_object_t::from(unsafe { (*object).cached.type_ })
}

/// `git_odb_object_dup`.
///
/// # Safety
///
/// `dest` is writable; `source` is a live object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_object_dup(
    mut dest: Out<'_, *mut GitOdbObject>,
    source: *mut GitOdbObject,
) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract: a live object, whose count is
        // atomic.
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
            // SAFETY: the caller's contract.
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
pub unsafe extern "C" fn git_odb_options_init(opts: *mut GitOdbOptions, version: c_uint) -> c_int {
    barrier(|| {
        git_assert_arg!(!opts.is_null(), "opts", -1);
        if let Err(e) = check_version(Some(version), GIT_ODB_OPTIONS_VERSION, "git_odb_options") {
            return e.raw();
        }
        // SAFETY: the caller's contract.
        unsafe { opts.write(GitOdbOptions::default()) };
        0
    })
}

/// The options at `opts` (NULL: none), copied.
///
/// # Safety
///
/// `opts` is NULL or a readable `git_odb_options` (its version first).
unsafe fn read_opts(opts: *const GitOdbOptions) -> Option<GitOdbOptions> {
    // SAFETY: the caller's contract.
    (!opts.is_null()).then(|| unsafe { opts.read() })
}

/// `git_odb_new_ext`.
///
/// # Safety
///
/// `out` is writable; `opts` is NULL or a readable `git_odb_options`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_new_ext(
    out: *mut *mut GitOdb,
    opts: *const GitOdbOptions,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        // SAFETY: the caller's contract.
        let opts = unsafe { read_opts(opts) };
        match GitOdb::new(opts.as_ref()) {
            Ok(db) => {
                // SAFETY: the caller's contract.
                unsafe { out.write(CShared::into_raw(db)) };
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
pub unsafe extern "C" fn git_odb_new(out: *mut *mut GitOdb) -> c_int {
    // SAFETY: forwarded.
    unsafe { git_odb_new_ext(out, core::ptr::null()) }
}

fn add_backend(
    odb_: *mut GitOdb,
    backend: *mut GitOdbBackend,
    priority: c_int,
    alternate: bool,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!odb_.is_null(), "odb", -1);
        let Some(b) = NonNull::new(backend) else {
            let _ = crate::util::errors::assert_arg_failed("backend");
            return -1;
        };
        // SAFETY: the exports' contract: a live odb.
        let db = unsafe { odb(odb_) };
        // SAFETY: the exports' contract: a live backend table, which the
        // odb takes on success.
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
    odb: *mut GitOdb,
    backend: *mut GitOdbBackend,
    priority: c_int,
) -> c_int {
    add_backend(odb, backend, priority, false)
}

/// `git_odb_add_alternate`.
///
/// # Safety
///
/// As [`git_odb_add_backend`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_add_alternate(
    odb: *mut GitOdb,
    backend: *mut GitOdbBackend,
    priority: c_int,
) -> c_int {
    add_backend(odb, backend, priority, true)
}

/// `git_odb_num_backends`.
///
/// # Safety
///
/// `db` is NULL or a live odb.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_num_backends(db: *mut GitOdb) -> usize {
    barrier(|| {
        // C returns `GIT_ASSERT_ARG`'s -1 as a `size_t`.
        git_assert_arg!(!db.is_null(), "odb", usize::MAX);
        // SAFETY: the exports' contract.
        unsafe { odb(db) }.num_backends()
    })
}

/// `git_odb_get_backend`.
///
/// # Safety
///
/// `out` is writable; `db` is a live odb.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_get_backend(
    out: *mut *mut GitOdbBackend,
    db: *mut GitOdb,
    pos: usize,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        git_assert_arg!(!db.is_null(), "odb", -1);
        // SAFETY: the exports' contract.
        match unsafe { odb(db) }.get_backend(pos) {
            Ok(b) => {
                // SAFETY: the caller's contract.
                unsafe { out.write(b.as_ptr()) };
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
/// `db` is a live odb; `objects_dir` a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb__add_default_backends(
    db: *mut GitOdb,
    objects_dir: CStrArg<'_>,
    as_alternates: bool,
    alternate_depth: c_int,
) -> c_int {
    barrier(|| {
        let dir = objects_dir.get().unwrap_or(c"");
        // SAFETY: as above.
        status(unsafe { odb(db) }.add_default_backends(dir, as_alternates, alternate_depth))
    })
}

/// `git_odb_add_disk_alternate`.
///
/// # Safety
///
/// As [`git_odb__add_default_backends`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_add_disk_alternate(db: *mut GitOdb, path: CStrArg<'_>) -> c_int {
    // SAFETY: forwarded.
    unsafe { git_odb__add_default_backends(db, path, true, 0) }
}

/// `git_odb_set_commit_graph`.
///
/// # Safety
///
/// `db` is NULL or a live odb; `cgraph` is NULL or a commit-graph whose
/// ownership passes to the odb.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_set_commit_graph(db: *mut GitOdb, cgraph: *mut c_void) -> c_int {
    barrier(|| {
        git_assert_arg!(!db.is_null(), "odb", -1);
        // SAFETY: the caller's contract.
        let g = NonNull::new(cgraph).map(|p| unsafe { CommitGraph::from_raw(p) });
        // SAFETY: as above.
        unsafe { odb(db) }.set_commit_graph(g);
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
    out: *mut *mut GitOdb,
    objects_dir: CStrArg<'_>,
    opts: *const GitOdbOptions,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        let objects_dir = git_arg!(objects_dir.get(), "objects_dir", -1);
        // SAFETY: the caller's contract.
        let opts = unsafe { read_opts(opts) };
        if let Err(e) = check_version(
            opts.map(|o| o.version),
            GIT_ODB_OPTIONS_VERSION,
            "git_odb_options",
        ) {
            return e.raw();
        }
        // SAFETY: the caller's contract.
        unsafe { out.write(core::ptr::null_mut()) };
        let db = match GitOdb::new(opts.as_ref()) {
            Ok(db) => db,
            Err(_) => return -1,
        };
        let dir = objects_dir;
        if db.add_default_backends(dir, false, 0).is_err() {
            return -1;
        }
        // SAFETY: the caller's contract.
        unsafe { out.write(CShared::into_raw(db)) };
        0
    })
}

/// `git_odb_open`.
///
/// # Safety
///
/// As [`git_odb_open_ext`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_open(out: *mut *mut GitOdb, objects_dir: CStrArg<'_>) -> c_int {
    // SAFETY: forwarded.
    unsafe { git_odb_open_ext(out, objects_dir, core::ptr::null()) }
}

/// `git_odb__set_caps`.
///
/// # Safety
///
/// `db` is a live odb.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb__set_caps(db: *mut GitOdb, caps: c_int) -> c_int {
    // SAFETY: the caller's contract.
    barrier(|| status(unsafe { odb(db) }.set_caps(caps)))
}

/// `git_odb_free`.
///
/// # Safety
///
/// `db` is NULL or a live odb whose reference the caller gives up.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_free(db: *mut GitOdb) {
    // SAFETY: the caller's contract.
    barrier(|| unsafe { CShared::release(db) })
}

/// `git_odb__get_commit_graph_file`.
///
/// # Safety
///
/// `out` is writable; `db` is a live odb.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb__get_commit_graph_file(
    mut out: Out<'_, *mut c_void>,
    db: *mut GitOdb,
) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract.
        match unsafe { odb(db) }.get_commit_graph_file() {
            Ok(f) => {
                out.write(f);
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_odb__freshen`.
///
/// # Safety
///
/// `db` is NULL or a live odb; `id` NULL or a readable id.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb__freshen(db: *mut GitOdb, id: Option<&Oid>) -> c_int {
    barrier(|| {
        git_assert_arg!(!db.is_null(), "db", -1);
        let id = git_arg!(id, "id", -1);
        // SAFETY: the caller's contract: a live odb.
        let db = unsafe { odb(db) };
        let id_ = id;
        c_int::from(db.freshen(id_))
    })
}

/// `git_odb_exists`.
///
/// # Safety
///
/// As [`git_odb__freshen`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_exists(db: *mut GitOdb, id: Option<&Oid>) -> c_int {
    // SAFETY: forwarded.
    unsafe { git_odb_exists_ext(db, id, 0) }
}

/// `git_odb_exists_ext`.
///
/// # Safety
///
/// As [`git_odb__freshen`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_exists_ext(
    db: *mut GitOdb,
    id: Option<&Oid>,
    flags: c_uint,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!db.is_null(), "db", -1);
        let id = git_arg!(id, "id", -1);
        // SAFETY: the caller's contract: a live odb.
        let db = unsafe { odb(db) };
        let id_ = id;
        c_int::from(db.exists_ext(id_, flags))
    })
}

/// `git_odb_exists_prefix`.
///
/// # Safety
///
/// `out` is NULL or writable; otherwise as [`git_odb__freshen`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_exists_prefix(
    mut out: Out<'_, Oid>,
    db: *mut GitOdb,
    short_id: *const Oid,
    len: usize,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!db.is_null(), "db", -1);
        git_assert_arg!(!short_id.is_null(), "short_id", -1);
        // SAFETY: the caller's contract: a live odb.
        let db = unsafe { odb(db) };
        // SAFETY: the caller's contract: a readable id.
        let short_id_ = unsafe { &*short_id };
        match db.exists_prefix(short_id_, len) {
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
    db: *mut GitOdb,
    ids: *mut GitOdbExpandId,
    count: usize,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!db.is_null(), "db", -1);
        git_assert_arg!(!ids.is_null(), "ids", -1);
        assert!(
            count <= isize::MAX as usize / size_of::<GitOdbExpandId>(),
            "count"
        );
        // SAFETY: the caller's contract; the entries are the caller's
        // alone during the call.
        let ids = unsafe { core::slice::from_raw_parts_mut(ids, count) };
        // SAFETY: as above.
        status(unsafe { odb(db) }.expand_ids(ids))
    })
}

/// `git_odb_read_header`.
///
/// # Safety
///
/// `len_p` and `type_p` are writable; otherwise as [`git_odb__freshen`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_read_header(
    len_p: *mut usize,
    type_p: *mut git_object_t,
    db: *mut GitOdb,
    id: Option<&Oid>,
) -> c_int {
    let mut obj: *mut GitOdbObject = core::ptr::null_mut();
    // SAFETY: forwarded; `obj` is a local.
    let r = unsafe { git_odb__read_header_or_object(&raw mut obj, len_p, type_p, db, id) };
    if let Some(o) = NonNull::new(obj) {
        // SAFETY: the reference it returned.
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
    out: *mut *mut GitOdbObject,
    len_p: *mut usize,
    type_p: *mut git_object_t,
    db: *mut GitOdb,
    id: Option<&Oid>,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!db.is_null(), "db", -1);
        let id = git_arg!(id, "id", -1);
        git_assert_arg!(!out.is_null(), "out", -1);
        git_assert_arg!(!len_p.is_null(), "len_p", -1);
        git_assert_arg!(!type_p.is_null(), "type_p", -1);
        // SAFETY: the caller's contract.
        unsafe { out.write(core::ptr::null_mut()) };
        // SAFETY: the caller's contract: a live odb.
        let db = unsafe { odb(db) };
        let id_ = id;
        match db.read_header_or_object(id_) {
            Ok((len, t, obj)) => {
                // SAFETY: the caller's contract.
                unsafe { len_p.write(len) };
                // SAFETY: the caller's contract.
                unsafe { type_p.write(t) };
                // SAFETY: the caller's contract.
                unsafe { out.write(obj.map_or(core::ptr::null_mut(), NonNull::as_ptr)) };
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
/// `out` is writable; otherwise as [`git_odb__freshen`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_read(
    out: *mut *mut GitOdbObject,
    db: *mut GitOdb,
    id: *const Oid,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        git_assert_arg!(!db.is_null(), "db", -1);
        git_assert_arg!(!id.is_null(), "id", -1);
        // SAFETY: the caller's contract: a live odb.
        let db = unsafe { odb(db) };
        // SAFETY: the caller's contract: a readable id.
        let id_ = unsafe { &*id };
        match db.read(id_) {
            Ok(o) => {
                // SAFETY: the caller's contract.
                unsafe { out.write(o.as_ptr()) };
                0
            }
            Err(e) => {
                // C leaves `*out` NULL when the cache missed.
                // SAFETY: the caller's contract.
                if !unsafe { (*id).is_zero() } {
                    // SAFETY: the caller's contract.
                    unsafe { out.write(core::ptr::null_mut()) };
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
/// `out` is writable; `db` a live odb; `short_id` a readable id.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_read_prefix(
    out: *mut *mut GitOdbObject,
    db: *mut GitOdb,
    short_id: Option<&Oid>,
    len: usize,
) -> c_int {
    barrier(|| {
        let short_id = git_arg!(short_id, "short_id", -1);
        git_assert_arg!(!out.is_null(), "out", -1);
        git_assert_arg!(!db.is_null(), "db", -1);
        // SAFETY: the caller's contract: a live odb.
        let db = unsafe { odb(db) };
        let short_id_ = short_id;
        match db.read_prefix(short_id_, len) {
            Ok(o) => {
                // SAFETY: the caller's contract.
                unsafe { out.write(o.as_ptr()) };
                0
            }
            Err(e) => {
                // C set `*out` to the cache's answer (NULL) for a full id.
                if len >= crate::oid::GIT_OID_MINPREFIXLEN && len >= db.hexsize() {
                    // SAFETY: the caller's contract.
                    unsafe { out.write(core::ptr::null_mut()) };
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
/// `db` is a live odb; `cb` may be called with `payload`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_foreach(
    db: *mut GitOdb,
    cb: ForeachCb,
    payload: *mut c_void,
) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract: a live odb.
        let db = unsafe { odb(db) };
        // SAFETY: the caller's contract on `cb` and `payload`.
        unsafe { db.foreach_raw(cb, payload) }
    })
}

/// `git_odb_write`.
///
/// # Safety
///
/// `oid` is writable; `db` a live odb; `data` readable for `len` bytes
/// (or NULL).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_write(
    oid: *mut Oid,
    db: *mut GitOdb,
    data: *const c_void,
    len: usize,
    type_: git_object_t,
) -> c_int {
    barrier(|| {
        // C reads `db->options` before these checks (a NULL `db` crashed).
        git_assert_arg!(!oid.is_null(), "oid", -1);
        git_assert_arg!(!db.is_null(), "db", -1);
        if data.is_null() && len != 0 {
            // `git_object_id_from_buffer`'s "invalid object"; `*oid` is
            // not written.
            // SAFETY: the caller's contract: a live odb.
            let oid_type = unsafe { odb(db) }.oid_type();
            return hash_object(None, len, type_, oid_type).map_or_else(|e| e.raw(), |_| -1);
        }
        // SAFETY: the caller's contract (NULL only with `len == 0`).
        let data = unsafe { bytes(data.cast(), len) };
        let mut id = None;
        // SAFETY: the caller's contract.
        let r = unsafe { odb(db) }.write(data, type_, &mut id);
        if let Some(id) = id {
            // C wrote `*oid` once hashed, whatever followed.
            // SAFETY: the caller's contract.
            unsafe { oid.write(id) };
        }
        status(r)
    })
}

/// `git_odb_open_wstream`.
///
/// # Safety
///
/// `stream` is writable; `db` a live odb.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_open_wstream(
    stream: *mut *mut GitOdbStream,
    db: *mut GitOdb,
    size: git_object_size_t,
    type_: git_object_t,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!stream.is_null(), "stream", -1);
        git_assert_arg!(!db.is_null(), "db", -1);
        // SAFETY: the caller's contract.
        match unsafe { odb(db) }.open_wstream(size, type_) {
            Ok(s) => {
                // SAFETY: the caller's contract.
                unsafe { stream.write(s.as_ptr()) };
                0
            }
            Err(ErrorCode::PASSTHROUGH) => 0,
            Err(e) => e.raw(),
        }
    })
}

/// `git_odb_stream__invalid_length`.
fn invalid_length(stream: *const GitOdbStream, action: &str) -> c_int {
    // SAFETY: the callers' stream is live.
    let s = unsafe { &*stream };
    let (declared, received) = (s.declared_size, s.received_bytes);
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
/// `stream` is a live stream from `git_odb_open_wstream`; `buffer` is
/// readable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_stream_write(
    stream: *mut GitOdbStream,
    buffer: *const c_char,
    len: usize,
) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract: the stream's context (NULL for a
        // stream not opened for writing, which C dereferenced) is a
        // `git_hash_ctx` the odb initialized.
        let ctx = unsafe { (*stream).hash_ctx };
        // SAFETY: as above: NULL or the odb's initialized context, which
        // nothing else accesses during the call.
        if let Some(ctx) = unsafe { ctx.cast::<crate::util::hash::HashCtx>().as_mut() } {
            // SAFETY: as above; the caller's `buffer`.
            let _ =
                unsafe { crate::util::hash::ffi::git_hash_update(Some(ctx), buffer.cast(), len) };
        }
        // SAFETY: the caller's contract: the stream is the caller's alone
        // during the call (and no slot of it runs).
        let s = unsafe { &mut *stream };
        s.received_bytes = s.received_bytes.wrapping_add(len as u64);
        if s.received_bytes > s.declared_size {
            return invalid_length(stream, "stream_write()");
        }
        // SAFETY: as above.
        unsafe { stream_write_slot(stream, buffer, len) }
    })
}

/// `git_odb_stream_finalize_write`.
///
/// # Safety
///
/// `out` is writable; `stream` is a live stream from
/// `git_odb_open_wstream`, whose backend belongs to a live odb.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_stream_finalize_write(
    out: *mut Oid,
    stream: *mut GitOdbStream,
) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract: the caller's stream.
        let s = unsafe { &*stream };
        let (received, declared, ctx, oid_type, backend) = (
            s.received_bytes,
            s.declared_size,
            s.hash_ctx,
            s.oid_type,
            s.backend,
        );
        if received != declared {
            return invalid_length(stream, "stream_finalize_write()");
        }
        let mut id = [0u8; crate::oid::GIT_OID_MAX_SIZE];
        // SAFETY: NULL or the odb's initialized context, which nothing
        // else accesses during the call (the caller's stream).
        if let Some(ctx) = unsafe { ctx.cast::<crate::util::hash::HashCtx>().as_mut() } {
            // SAFETY: the odb's context; `id` holds any digest.
            let _ = unsafe { crate::util::hash::ffi::git_hash_final(id.as_mut_ptr(), Some(ctx)) };
        }
        let oid = Oid::from_raw_parts(oid_type as u8, id);
        // SAFETY: the caller's contract.
        unsafe { out.write(oid) };
        // SAFETY: the caller's contract: a live backend of a live odb.
        let db = unsafe { (*backend).odb };
        // SAFETY: as above.
        if !db.is_null() && unsafe { odb(db) }.freshen(&oid) {
            return 0;
        }
        // SAFETY: as above.
        match unsafe { (*stream).finalize_write } {
            // SAFETY: as above.
            Some(f) => unsafe { f(stream, out) },
            None => git_error_set!(ErrorClass::ODB, "stream does not support writing").raw(),
        }
    })
}

/// `git_odb_stream_read`.
///
/// # Safety
///
/// `stream` is a live read stream; `buffer` is writable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_stream_read(
    stream: *mut GitOdbStream,
    buffer: *mut c_char,
    len: usize,
) -> c_int {
    barrier(|| {
        // SAFETY: the caller's contract.
        match unsafe { (*stream).read } {
            // SAFETY: as above.
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
            // SAFETY: the caller's contract.
            unsafe { stream_free(stream) }
        }
    })
}

/// `git_odb_open_rstream`.
///
/// # Safety
///
/// `stream`, `len` and `type_` are writable; `db` a live odb; `oid` a
/// readable id.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_open_rstream(
    stream: *mut *mut GitOdbStream,
    mut len: Out<'_, usize>,
    mut type_: Out<'_, git_object_t>,
    db: *mut GitOdb,
    oid: Option<&Oid>,
) -> c_int {
    barrier(|| {
        let oid = git_arg!(oid, "oid", -1);
        git_assert_arg!(!stream.is_null(), "stream", -1);
        git_assert_arg!(!db.is_null(), "db", -1);
        // SAFETY: the caller's contract: a live odb.
        let db = unsafe { odb(db) };
        let oid_ = oid;
        match db.open_rstream(oid_) {
            // C: 0, with `*stream` as the backend left it (untouched).
            Ok(None) => 0,
            Ok(Some((s, l, t))) => {
                // SAFETY: the caller's contract.
                unsafe { stream.write(s.as_ptr()) };
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
/// `out` is writable; `db` a live odb; `progress_cb` may be called with
/// `progress_payload`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_write_pack(
    out: *mut *mut GitOdbWritepack,
    db: *mut GitOdb,
    progress_cb: IndexerProgressCb,
    progress_payload: *mut c_void,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        git_assert_arg!(!db.is_null(), "db", -1);
        // SAFETY: the caller's contract: a live odb.
        let db = unsafe { odb(db) };
        // SAFETY: the caller's contract on the callback.
        match unsafe { db.write_pack(progress_cb, progress_payload) } {
            Ok(w) => {
                if !w.is_null() {
                    // SAFETY: the caller's contract.
                    unsafe { out.write(w) };
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
pub unsafe extern "C" fn git_odb_write_multi_pack_index(db: *mut GitOdb) -> c_int {
    barrier(|| {
        git_assert_arg!(!db.is_null(), "db", -1);
        // SAFETY: the caller's contract.
        status(unsafe { odb(db) }.write_multi_pack_index())
    })
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
    // SAFETY: the caller's contract.
    barrier(|| unsafe { alloc::free(data) })
}

/// `git_odb_refresh`.
///
/// # Safety
///
/// `db` is NULL or a live odb.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_refresh(db: *mut GitOdb) -> c_int {
    barrier(|| {
        git_assert_arg!(!db.is_null(), "db", -1);
        // SAFETY: the caller's contract.
        status(unsafe { odb(db) }.refresh())
    })
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
        let e = expected;
        let a = actual;
        error_mismatch(e, a).raw()
    })
}

/// `git_odb__error_notfound`.
///
/// # Safety
///
/// `message` is a NUL-terminated string; `oid` is NULL or readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb__error_notfound(
    message: CStrArg<'_>,
    oid: *const Oid,
    oid_len: usize,
) -> c_int {
    barrier(|| {
        let m = message.to_bytes();
        // SAFETY: the caller's contract.
        let o = unsafe { oid.as_ref() };
        error_notfound(m, o, oid_len).raw()
    })
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
/// `backend` is writable for a `git_odb_backend`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_odb_init_backend(
    backend: *mut GitOdbBackend,
    version: c_uint,
) -> c_int {
    barrier(|| {
        if let Err(e) = check_version(Some(version), GIT_ODB_BACKEND_VERSION, "git_odb_backend") {
            return e.raw();
        }
        // SAFETY: the caller's contract.
        unsafe {
            backend.write(GitOdbBackend {
                version: GIT_ODB_BACKEND_VERSION,
                ..GitOdbBackend::default()
            })
        };
        0
    })
}
