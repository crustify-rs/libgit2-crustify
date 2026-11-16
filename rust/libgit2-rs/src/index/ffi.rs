//! The C ABI of `index.c`: thin shims over [`GitIndex`]'s methods and the
//! [`ops`](super::ops).
//!
//! Every export takes the index as C passes it. Where the C checked an
//! argument (`GIT_ASSERT_ARG`), the export does with the same message;
//! where the C dereferenced a NULL index or path, the export fails the same
//! check instead (see `rust/DEFECTS.md`).

use core::ffi::{c_char, c_int, c_uint, c_void};
use core::ptr::{self, NonNull};

use super::foreign::{Paths, Repo, TreeRef};
use super::ops::{self, MatchedPathCb};
use super::{
    GIT_INDEX_OPTIONS_VERSION, GitIndex, GitIndexConflictIterator, GitIndexEntry, GitIndexIterator,
    GitIndexNameEntry, GitIndexOptions, GitIndexReucEntry, GitIndexWriter, INDEX_VERSION_NUMBER_LB,
    INDEX_VERSION_NUMBER_UB, IndexPtr, as_entry, entry_srch,
};
use crate::oid::{Oid, OidType};
use crate::strarray::GitStrArray;
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::export::{barrier, cstr, cstr_bytes, status};
use crate::util::str::GitStr;
use crate::util::vector::{GitVector, VectorCmp};
use crate::{git_assert, git_assert_arg, git_error_set};

/// `git_index_matched_path_cb`.
pub type GitIndexMatchedPathCb = unsafe extern "C" fn(
    path: *const c_char,
    matched_pathspec: *const c_char,
    payload: *mut c_void,
) -> c_int;

/// A path argument: `path_len` bytes, or a string when 0.
///
/// # Safety
///
/// `path` is readable for `path_len` bytes, or a string when `path_len` is
/// 0, for `'a`.
unsafe fn path_arg<'a>(path: *const c_char, path_len: usize) -> &'a [u8] {
    if path_len == 0 {
        // SAFETY: the caller's contract.
        unsafe { cstr_bytes(path) }
    } else {
        // SAFETY: the caller's contract.
        unsafe { crate::util::export::bytes(path, path_len) }
    }
}

/// The handle of a non-null index pointer.
///
/// # Safety
///
/// `index` is NULL or a live index (this module's), used on this thread.
#[inline(always)]
unsafe fn handle(index: *const GitIndex) -> Option<IndexPtr> {
    // SAFETY: the caller's contract.
    NonNull::new(index.cast_mut()).map(|p| unsafe { IndexPtr::new(p) })
}

/// `GIT_ASSERT_ARG(index)`, the handle.
macro_rules! index_arg {
    ($index:expr, $ret:expr) => {{
        // SAFETY: the export's contract: NULL or a live index.
        match unsafe { handle($index) } {
            Some(ix) => ix,
            None => {
                let _ = errors::assert_arg_failed("index");
                return $ret;
            }
        }
    }};
}

/// `GIT_ASSERT_ARG(index)` for the exports without a result.
///
/// # Safety
///
/// As [`handle`].
unsafe fn index_or_assert(index: *const GitIndex) -> Option<IndexPtr> {
    // SAFETY: the caller's contract: NULL or a live index.
    let ix = unsafe { handle(index) };
    if ix.is_none() {
        let _ = errors::assert_arg_failed("index");
    }
    ix
}

/// A C matched-path callback as the operations call it.
fn c_callback<'a>(
    cb: Option<GitIndexMatchedPathCb>,
    payload: *mut c_void,
) -> Option<impl FnMut(*const c_char, *const c_char) -> c_int + 'a> {
    cb.map(move |f| {
        move |p: *const c_char, m: *const c_char| {
            // SAFETY: the caller's callback and payload, with two strings
            // (or a NULL match) live for the call.
            unsafe { f(p, m, payload) }
        }
    })
}

// ---------------------------------------------------------------------------
// Comparison and search functions (C sorts and searches with them)
// ---------------------------------------------------------------------------

/// The path of any `git_index_entry` (a C string).
///
/// # Safety
///
/// `e` is a live entry with a string path.
unsafe fn any_path<'a>(e: *const GitIndexEntry) -> &'a [u8] {
    // SAFETY: the caller's contract.
    let p = unsafe { (*e).path };
    // SAFETY: as above.
    unsafe { cstr_bytes(p) }
}

/// `git_index_entry_cmp`.
///
/// # Safety
///
/// `a` and `b` are live `git_index_entry`s with string paths.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_entry_cmp(a: *const c_void, b: *const c_void) -> c_int {
    let (a, b) = (a.cast::<GitIndexEntry>(), b.cast::<GitIndexEntry>());
    // SAFETY: the caller's contract.
    let a = unsafe { &*a };
    // SAFETY: as above.
    let b = unsafe { &*b };
    // SAFETY: as above.
    let pa = unsafe { any_path(a) };
    // SAFETY: as above.
    let pb = unsafe { any_path(b) };
    let d = crate::util::util::strcmp(pa, pb);
    if d != 0 { d } else { a.stage() - b.stage() }
}

/// `git_index_entry_icmp`.
///
/// # Safety
///
/// As [`git_index_entry_cmp`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_entry_icmp(a: *const c_void, b: *const c_void) -> c_int {
    let (a, b) = (a.cast::<GitIndexEntry>(), b.cast::<GitIndexEntry>());
    // SAFETY: the caller's contract.
    let a = unsafe { &*a };
    // SAFETY: as above.
    let b = unsafe { &*b };
    // SAFETY: as above.
    let pa = unsafe { any_path(a) };
    // SAFETY: as above.
    let pb = unsafe { any_path(b) };
    let d = crate::util::util::strcasecmp(pa, pb);
    if d != 0 { d } else { a.stage() - b.stage() }
}

/// `struct entry_srch_key`, mirrored (C makes none itself: this module's
/// searches do, and pass it to the search functions).
#[repr(C)]
pub struct EntrySrchKey {
    /// `path`.
    pub path: *const c_char,
    /// `pathlen`.
    pub pathlen: usize,
    /// `stage`.
    pub stage: c_int,
}

/// `git_index_entry_srch`.
///
/// # Safety
///
/// `key` is an [`EntrySrchKey`] whose path is readable for `pathlen`
/// bytes; `array_member` an index entry (`entry_internal`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_entry_srch(
    key: *const c_void,
    array_member: *const c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { srch(key, array_member, false) }
}

/// `git_index_entry_isrch`.
///
/// # Safety
///
/// As [`git_index_entry_srch`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_entry_isrch(
    key: *const c_void,
    array_member: *const c_void,
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { srch(key, array_member, true) }
}

/// The two search functions.
///
/// # Safety
///
/// As [`git_index_entry_srch`].
unsafe fn srch(key: *const c_void, member: *const c_void, icase: bool) -> c_int {
    // SAFETY: the caller's contract.
    let k = unsafe { &*key.cast::<EntrySrchKey>() };
    // SAFETY: as above.
    let path = unsafe { crate::util::export::bytes(k.path, k.pathlen) };
    // SAFETY: as above.
    unsafe { entry_srch(path, k.stage, as_entry(member.cast_mut()), icase) }
}

// ---------------------------------------------------------------------------
// Opening and freeing
// ---------------------------------------------------------------------------

/// `git_error__check_version`.
fn check_version(opts: *const GitIndexOptions, expected_max: c_uint, name: &str) -> Result<()> {
    if opts.is_null() {
        return Ok(());
    }
    // SAFETY: a non-null options pointer is readable (the exports'
    // contracts).
    let actual = unsafe { (*opts).version };
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

/// `git_index_options_init`.
///
/// # Safety
///
/// `opts` is NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_options_init(
    opts: *mut GitIndexOptions,
    version: c_uint,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!opts.is_null(), "opts", -1);
        let probe = GitIndexOptions {
            version,
            oid_type: 0,
        };
        if check_version(
            &raw const probe,
            GIT_INDEX_OPTIONS_VERSION,
            "git_index_options",
        )
        .is_err()
        {
            return -1;
        }
        // SAFETY: writable (the contract).
        unsafe {
            opts.write(GitIndexOptions {
                version: GIT_INDEX_OPTIONS_VERSION,
                oid_type: 0,
            })
        };
        0
    })
}

/// `git_index_open_ext`.
///
/// # Safety
///
/// `index_out` is NULL or writable; `index_path` NULL or a string; `opts`
/// NULL or a readable options struct.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_open_ext(
    index_out: *mut *mut GitIndex,
    index_path: *const c_char,
    opts: *const GitIndexOptions,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!index_out.is_null(), "index_out", -1);
        if check_version(opts, GIT_INDEX_OPTIONS_VERSION, "git_index_options").is_err() {
            return -1;
        }
        let raw_type = if opts.is_null() {
            0
        } else {
            // SAFETY: readable when non-null.
            unsafe { (*opts).oid_type }
        };
        let t = if raw_type != 0 {
            match OidType::from_raw(raw_type) {
                Some(t) => t,
                None => {
                    let _ = errors::assert_arg_failed("git_oid_type_is_valid(opts->oid_type)");
                    return -1;
                }
            }
        } else {
            OidType::Sha1
        };
        // SAFETY: a string when non-null.
        let path = unsafe { cstr(index_path) };
        match ops::open_ext(path, t) {
            Ok(ix) => {
                // SAFETY: writable.
                unsafe { index_out.write(ix.as_ptr()) };
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_index_open`.
///
/// # Safety
///
/// As [`git_index_open_ext`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_open(
    index_out: *mut *mut GitIndex,
    index_path: *const c_char,
) -> c_int {
    // SAFETY: forwarded.
    unsafe { git_index_open_ext(index_out, index_path, ptr::null()) }
}

/// `git_index_new`.
///
/// # Safety
///
/// `out` is NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_new(out: *mut *mut GitIndex) -> c_int {
    // SAFETY: forwarded.
    unsafe { git_index_open_ext(out, ptr::null(), ptr::null()) }
}

/// `git_index_new_ext`.
///
/// # Safety
///
/// As [`git_index_open_ext`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_new_ext(
    out: *mut *mut GitIndex,
    opts: *const GitIndexOptions,
) -> c_int {
    // SAFETY: forwarded.
    unsafe { git_index_open_ext(out, ptr::null(), opts) }
}

/// `git_index_free`.
///
/// # Safety
///
/// `index` is NULL or an index the caller holds a reference to, which it
/// gives up.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_free(index: *mut GitIndex) {
    barrier(|| {
        // SAFETY: the contract.
        if let Some(ix) = unsafe { handle(index) } {
            // SAFETY: the caller's reference.
            unsafe { ix.decref() };
        }
    })
}

/// `git_index_clear`.
///
/// # Safety
///
/// `index` is NULL or a live index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_clear(index: *mut GitIndex) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        // SAFETY: no foreign call.
        status(unsafe { ix.get() }.clear())
    })
}

/// `git_index_owner`.
///
/// # Safety
///
/// `index` is a live index (NULL: NULL).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_owner(index: *const GitIndex) -> *mut c_void {
    barrier(|| {
        let ix = index_arg!(index, ptr::null_mut());
        ix.owner()
    })
}

/// `git_index_path`.
///
/// # Safety
///
/// `index` is NULL or a live index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_path(index: *const GitIndex) -> *const c_char {
    barrier(|| {
        let ix = index_arg!(index, ptr::null());
        // SAFETY: no foreign call; the path's pointer.
        unsafe { ix.get() }.index_file_path
    })
}

/// `git_index_caps`.
///
/// # Safety
///
/// `index` is a live index (NULL: 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_caps(index: *const GitIndex) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, 0);
        // SAFETY: no foreign call.
        let i = unsafe { ix.get() };
        (if i.ignore_case() {
            super::GIT_INDEX_CAPABILITY_IGNORE_CASE
        } else {
            0
        }) | (if i.distrust_filemode() {
            super::GIT_INDEX_CAPABILITY_NO_FILEMODE
        } else {
            0
        }) | (if i.no_symlinks() {
            super::GIT_INDEX_CAPABILITY_NO_SYMLINKS
        } else {
            0
        })
    })
}

/// `git_index_set_caps`.
///
/// # Safety
///
/// `index` is NULL or a live index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_set_caps(index: *mut GitIndex, caps: c_int) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        status(ops::set_caps(ix, caps))
    })
}

/// `git_index_version`.
///
/// # Safety
///
/// `index` is NULL or a live index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_version(index: *mut GitIndex) -> c_uint {
    barrier(|| {
        let ix = index_arg!(index, c_uint::MAX);
        // SAFETY: no foreign call.
        unsafe { ix.get() }.version
    })
}

/// `git_index_set_version`.
///
/// # Safety
///
/// `index` is NULL or a live index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_set_version(index: *mut GitIndex, version: c_uint) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        if !(INDEX_VERSION_NUMBER_LB..=INDEX_VERSION_NUMBER_UB).contains(&version) {
            return git_error_set!(ErrorClass::INDEX, "invalid version number").raw();
        }
        // SAFETY: no foreign call.
        unsafe { ix.get() }.version = version;
        0
    })
}

/// `git_index_read`.
///
/// # Safety
///
/// `index` is NULL or a live index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_read(index: *mut GitIndex, force: c_int) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        match ops::read(ix, force != 0) {
            Ok(v) => v,
            Err(e) => e.raw(),
        }
    })
}

/// `git_index_read_safely`.
///
/// # Safety
///
/// `index` is NULL or a live index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_read_safely(index: *mut GitIndex) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        match ops::read_safely(ix) {
            Ok(v) => v,
            Err(e) => e.raw(),
        }
    })
}

/// `git_index_write`.
///
/// # Safety
///
/// `index` is NULL or a live index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_write(index: *mut GitIndex) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        status(ops::write(ix))
    })
}

/// `git_index_write_tree`.
///
/// # Safety
///
/// `oid` is NULL or writable; `index` NULL or a live index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_write_tree(oid: *mut Oid, index: *mut GitIndex) -> c_int {
    barrier(|| {
        git_assert_arg!(!oid.is_null(), "oid", -1);
        let ix = index_arg!(index, -1);
        // SAFETY: the owner outlives the index.
        let Some(repo) = (unsafe { Repo::new(ix.owner()) }) else {
            return errors::set_str(
                ErrorClass::INDEX,
                "Failed to write tree. the index file is not backed up by an existing repository",
            )
            .raw();
        };
        // SAFETY: no reference into the index is live.
        match unsafe { super::foreign::tree_write_index(ix.as_ptr(), repo) } {
            Ok(id) => {
                // SAFETY: writable.
                unsafe { oid.write(id) };
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_index_write_tree_to`.
///
/// # Safety
///
/// As [`git_index_write_tree`]; `repo` NULL or a live repository.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_write_tree_to(
    oid: *mut Oid,
    index: *mut GitIndex,
    repo: *mut c_void,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!oid.is_null(), "oid", -1);
        let ix = index_arg!(index, -1);
        // SAFETY: a live repository (the contract).
        let Some(repo) = (unsafe { Repo::new(repo) }) else {
            let _ = errors::assert_arg_failed("repo");
            return -1;
        };
        // C's `git_tree__write_index` writes `oid` on its paths; so does
        // this (through a local first).
        // SAFETY: no reference into the index is live.
        match unsafe { super::foreign::tree_write_index(ix.as_ptr(), repo) } {
            Ok(id) => {
                // SAFETY: writable.
                unsafe { oid.write(id) };
                0
            }
            Err(e) => e.raw(),
        }
    })
}

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

/// `git_index_entrycount`.
///
/// # Safety
///
/// `index` is NULL or a live index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_entrycount(index: *const GitIndex) -> usize {
    barrier(|| {
        let ix = index_arg!(index, usize::MAX);
        // SAFETY: no foreign call.
        unsafe { ix.get() }.entries.len()
    })
}

/// `git_index_get_byindex`.
///
/// # Safety
///
/// `index` is NULL or a live index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_get_byindex(
    index: *mut GitIndex,
    n: usize,
) -> *const GitIndexEntry {
    barrier(|| {
        let ix = index_arg!(index, ptr::null());
        // SAFETY: no foreign call.
        let i = unsafe { ix.get() };
        i.sort();
        i.entry_at(n).map_or(ptr::null(), |e| {
            e.as_ptr().cast::<GitIndexEntry>().cast_const()
        })
    })
}

/// `git_index_get_bypath`.
///
/// # Safety
///
/// `index` is NULL or a live index; `path` a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_get_bypath(
    index: *mut GitIndex,
    path: *const c_char,
    stage: c_int,
) -> *const GitIndexEntry {
    barrier(|| {
        let ix = index_arg!(index, ptr::null());
        // SAFETY: a string.
        let path = unsafe { cstr_bytes(path) };
        // SAFETY: no foreign call.
        match unsafe { ix.get() }.get_bypath(path, stage) {
            Some(e) => e.as_ptr().cast::<GitIndexEntry>().cast_const(),
            None => {
                let _ = git_error_set!(
                    ErrorClass::INDEX,
                    "index does not contain '{}'",
                    Bytes(path)
                );
                ptr::null()
            }
        }
    })
}

/// `git_index_entry__init_from_stat`.
///
/// # Safety
///
/// `entry` is writable; `st` readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_entry__init_from_stat(
    entry: *mut GitIndexEntry,
    st: *mut libc::stat,
    trust_mode: bool,
) {
    barrier(|| {
        // SAFETY: the contract; C's entry is not otherwise accessed.
        let entry = unsafe { &mut *entry };
        // SAFETY: as above.
        let st = unsafe { &*st };
        super::init_from_stat(entry, st, trust_mode)
    })
}

/// `git_index__create_mode`.
#[unsafe(no_mangle)]
pub extern "C" fn git_index__create_mode(mode: c_uint) -> c_uint {
    super::create_mode(mode)
}

/// `git_index_entry_stage`.
///
/// # Safety
///
/// `entry` is a readable entry (NULL: 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_entry_stage(entry: *const GitIndexEntry) -> c_int {
    if entry.is_null() {
        return 0;
    }
    // SAFETY: readable.
    unsafe { (*entry).stage() }
}

/// `git_index_entry_is_conflict`.
///
/// # Safety
///
/// As [`git_index_entry_stage`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_entry_is_conflict(entry: *const GitIndexEntry) -> c_int {
    // SAFETY: forwarded.
    c_int::from(unsafe { git_index_entry_stage(entry) } > 0)
}

/// `GIT_ASSERT_ARG(source_entry && source_entry->path)`: the entry and
/// its path.
///
/// # Safety
///
/// `source` is NULL or a readable entry with a NULL or string path.
unsafe fn source_arg<'a>(source: *const GitIndexEntry) -> Option<(&'a GitIndexEntry, &'a [u8])> {
    // SAFETY: the caller's contract.
    let s = unsafe { source.as_ref() }?;
    if s.path.is_null() {
        return None;
    }
    // SAFETY: as above.
    Some((s, unsafe { cstr_bytes(s.path) }))
}

/// `git_index_add`.
///
/// # Safety
///
/// `index` NULL or a live index; `source_entry` NULL or a readable entry
/// whose path is NULL or a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_add(
    index: *mut GitIndex,
    source_entry: *const GitIndexEntry,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        // SAFETY: the contract.
        let Some((src, path)) = (unsafe { source_arg(source_entry) }) else {
            return errors::assert_arg_failed("source_entry && source_entry->path").raw();
        };
        // the entry is copied before anything reaches C (the caller's
        // entry may be one of the index's)
        let (src, path) = (*src, path.to_vec());
        status(ops::add(ix, &src, &path))
    })
}

/// `git_index_add_from_buffer`.
///
/// # Safety
///
/// As [`git_index_add`]; `buffer` readable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_add_from_buffer(
    index: *mut GitIndex,
    source_entry: *const GitIndexEntry,
    buffer: *const c_void,
    len: usize,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        // SAFETY: the contract.
        let Some((src, path)) = (unsafe { source_arg(source_entry) }) else {
            return errors::assert_arg_failed("source_entry && source_entry->path").raw();
        };
        let (src, path) = (*src, path.to_vec());
        // a buffer above 4 GiB is refused (after the owner and mode
        // checks) before any slice is formed
        let buf = if len > u32::MAX as usize {
            Err(len)
        } else {
            // SAFETY: `len` readable bytes.
            Ok(unsafe { crate::util::export::bytes(buffer.cast(), len) })
        };
        status(ops::add_from_buffer(ix, &src, &path, buf))
    })
}

/// `git_index_add_bypath`.
///
/// # Safety
///
/// `index` NULL or a live index; `path` NULL or a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_add_bypath(index: *mut GitIndex, path: *const c_char) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        git_assert_arg!(!path.is_null(), "path", -1);
        // SAFETY: a string; copied (it may be an entry's path).
        let Ok(p) = std::ffi::CString::new(unsafe { cstr_bytes(path) }) else {
            return -1;
        };
        status(ops::add_bypath(ix, &p))
    })
}

/// `git_index_remove_bypath`.
///
/// # Safety
///
/// As [`git_index_add_bypath`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_remove_bypath(
    index: *mut GitIndex,
    path: *const c_char,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        git_assert_arg!(!path.is_null(), "path", -1);
        // SAFETY: a string, copied (it may be an entry's path).
        let p = unsafe { cstr_bytes(path) }.to_vec();
        // SAFETY: no foreign call.
        status(ops::remove_bypath(unsafe { ix.get() }, &p))
    })
}

/// `git_index__fill`.
///
/// # Safety
///
/// `index` NULL or a live index; `source_entries` a vector of readable
/// entries with string paths.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index__fill(
    index: *mut GitIndex,
    source_entries: *const GitVector,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        // SAFETY: a valid vector (the contract); its elements are copied.
        let v: Vec<*const GitIndexEntry> = unsafe { &*source_entries }
            .iter()
            .map(|p| p.cast::<GitIndexEntry>().cast_const())
            .collect();
        status(ops::fill(ix, &v))
    })
}

/// `git_index_remove`.
///
/// # Safety
///
/// `index` NULL or a live index; `path` NULL or a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_remove(
    index: *mut GitIndex,
    path: *const c_char,
    stage: c_int,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        git_assert_arg!(!path.is_null(), "path", -1);
        // SAFETY: a string, copied.
        let p = unsafe { cstr_bytes(path) }.to_vec();
        // SAFETY: no foreign call.
        status(unsafe { ix.get() }.remove(&p, stage))
    })
}

/// `git_index_remove_directory`.
///
/// # Safety
///
/// As [`git_index_remove`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_remove_directory(
    index: *mut GitIndex,
    dir: *const c_char,
    stage: c_int,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        git_assert_arg!(!dir.is_null(), "dir", -1);
        let mut pfx = GitStr::new();
        // SAFETY: a string.
        if let Err(e) = pfx.sets(unsafe { cstr_bytes(dir) }) {
            return e.raw();
        }
        if let Err(e) = crate::util::fs_path::to_dir(&mut pfx) {
            return e.raw();
        }
        // SAFETY: no foreign call.
        status(unsafe { ix.get() }.remove_directory(pfx.as_bytes(), stage))
    })
}

/// `git_index_find_prefix`.
///
/// # Safety
///
/// `at_pos` NULL or writable; `index` NULL or live; `prefix` a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_find_prefix(
    at_pos: *mut usize,
    index: *mut GitIndex,
    prefix: *const c_char,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        git_assert_arg!(!prefix.is_null(), "prefix", -1);
        // SAFETY: a string.
        let p = unsafe { cstr_bytes(prefix) };
        // SAFETY: no foreign call.
        match unsafe { ix.get() }.find_prefix(p) {
            Some(pos) => {
                // SAFETY: NULL or writable.
                unsafe { crate::util::export::write_opt(at_pos, pos) };
                0
            }
            None => ErrorCode::NOT_FOUND.raw(),
        }
    })
}

/// `git_index__find_pos`.
///
/// # Safety
///
/// `out` NULL or writable; `index` NULL or live; `path` NULL or readable
/// for `path_len` bytes (a string when `path_len` is 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index__find_pos(
    out: *mut usize,
    index: *mut GitIndex,
    path: *const c_char,
    path_len: usize,
    stage: c_int,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        git_assert_arg!(!path.is_null(), "path", -1);
        // SAFETY: the contract.
        let p = unsafe { path_arg(path, path_len) };
        // SAFETY: no foreign call.
        let r = unsafe { ix.get() }.find(p, stage);
        let (pos, ret) = match r {
            Ok(p) => (p, 0),
            Err(p) => (p, ErrorCode::NOT_FOUND.raw()),
        };
        // SAFETY: NULL or writable.
        unsafe { crate::util::export::write_opt(out, pos) };
        ret
    })
}

/// `git_index_find`.
///
/// # Safety
///
/// As [`git_index_find_prefix`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_find(
    at_pos: *mut usize,
    index: *mut GitIndex,
    path: *const c_char,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        git_assert_arg!(!path.is_null(), "path", -1);
        // SAFETY: a string.
        let p = unsafe { cstr_bytes(path) };
        // SAFETY: no foreign call.
        match unsafe { ix.get() }.find_path(p) {
            Some(pos) => {
                // SAFETY: NULL or writable.
                unsafe { crate::util::export::write_opt(at_pos, pos) };
                0
            }
            None => {
                let _ = git_error_set!(ErrorClass::INDEX, "index does not contain {}", Bytes(p));
                ErrorCode::NOT_FOUND.raw()
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Conflicts
// ---------------------------------------------------------------------------

/// `git_index_conflict_add`.
///
/// # Safety
///
/// `index` NULL or live; each entry NULL or readable with a string path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_conflict_add(
    index: *mut GitIndex,
    ancestor_entry: *const GitIndexEntry,
    our_entry: *const GitIndexEntry,
    their_entry: *const GitIndexEntry,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        // the entries are copied first (they may be the index's own)
        let copy = |p: *const GitIndexEntry| -> Option<(GitIndexEntry, Vec<u8>)> {
            // SAFETY: the contract.
            let e = unsafe { p.as_ref() }?;
            // SAFETY: as above.
            Some((*e, unsafe { cstr_bytes(e.path) }.to_vec()))
        };
        let a = copy(ancestor_entry);
        let o = copy(our_entry);
        let t = copy(their_entry);
        fn side(x: &Option<(GitIndexEntry, Vec<u8>)>) -> Option<(&GitIndexEntry, &[u8])> {
            x.as_ref().map(|(e, p)| (e, p.as_slice()))
        }
        status(ops::conflict_add(ix, [side(&a), side(&o), side(&t)]))
    })
}

/// Write the three sides of a conflict.
///
/// # Safety
///
/// The pointers are writable.
unsafe fn write_conflict(out: [*mut *const GitIndexEntry; 3], sides: [Option<super::EntryPtr>; 3]) {
    for i in 0..3 {
        let v = sides[i].map_or(ptr::null(), |e| {
            e.as_ptr().cast::<GitIndexEntry>().cast_const()
        });
        // SAFETY: the caller's contract.
        unsafe { out[i].write(v) };
    }
}

/// `git_index_conflict_get`.
///
/// # Safety
///
/// The outputs NULL or writable; `index` NULL or live; `path` NULL or a
/// string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_conflict_get(
    ancestor_out: *mut *const GitIndexEntry,
    our_out: *mut *const GitIndexEntry,
    their_out: *mut *const GitIndexEntry,
    index: *mut GitIndex,
    path: *const c_char,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!ancestor_out.is_null(), "ancestor_out", -1);
        git_assert_arg!(!our_out.is_null(), "our_out", -1);
        git_assert_arg!(!their_out.is_null(), "their_out", -1);
        let ix = index_arg!(index, -1);
        git_assert_arg!(!path.is_null(), "path", -1);
        let out = [ancestor_out, our_out, their_out];
        // SAFETY: writable.
        unsafe { write_conflict(out, [None; 3]) };
        // SAFETY: a string.
        let p = unsafe { cstr_bytes(path) };
        // SAFETY: no foreign call.
        match unsafe { ix.get() }.conflict_get(p) {
            Ok(sides) => {
                // SAFETY: writable.
                unsafe { write_conflict(out, sides) };
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_index_conflict_remove`.
///
/// # Safety
///
/// `index` NULL or live; `path` NULL or a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_conflict_remove(
    index: *mut GitIndex,
    path: *const c_char,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        git_assert_arg!(!path.is_null(), "path", -1);
        // SAFETY: a string, copied.
        let p = unsafe { cstr_bytes(path) }.to_vec();
        // SAFETY: no foreign call.
        status(unsafe { ix.get() }.conflict_remove(Some(&p)))
    })
}

/// `git_index_conflict_cleanup`.
///
/// # Safety
///
/// `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_conflict_cleanup(index: *mut GitIndex) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        // SAFETY: no foreign call.
        status(unsafe { ix.get() }.conflict_remove(None))
    })
}

/// `git_index_has_conflicts`.
///
/// # Safety
///
/// `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_has_conflicts(index: *const GitIndex) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        // SAFETY: no foreign call.
        c_int::from(unsafe { ix.get() }.has_conflicts())
    })
}

/// `git_index__conflict_to_reuc`.
///
/// # Safety
///
/// `index` NULL or live; `path` NULL or a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index__conflict_to_reuc(
    index: *mut GitIndex,
    path: *const c_char,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        git_assert_arg!(!path.is_null(), "path", -1);
        // SAFETY: a string, copied (it may be an entry's path).
        let p = unsafe { cstr_bytes(path) }.to_vec();
        // SAFETY: no foreign call.
        status(ops::conflict_to_reuc(unsafe { ix.get() }, &p))
    })
}

// ---------------------------------------------------------------------------
// Iterators and snapshots
// ---------------------------------------------------------------------------

/// `git_index_iterator_new`.
///
/// # Safety
///
/// `iterator_out` NULL or writable; `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_iterator_new(
    iterator_out: *mut *mut GitIndexIterator,
    index: *mut GitIndex,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!iterator_out.is_null(), "iterator_out", -1);
        let ix = index_arg!(index, -1);
        match ops::snapshot_new(ix) {
            Ok(snap) => {
                let it = Box::new(GitIndexIterator {
                    index: ix,
                    snap,
                    cur: 0,
                });
                // SAFETY: writable.
                unsafe { iterator_out.write(Box::into_raw(it)) };
                0
            }
            Err(e) => e.raw(),
        }
    })
}

/// `git_index_iterator_next`.
///
/// # Safety
///
/// `out` NULL or writable; `it` NULL or an iterator of
/// [`git_index_iterator_new`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_iterator_next(
    out: *mut *const GitIndexEntry,
    it: *mut GitIndexIterator,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        git_assert_arg!(!it.is_null(), "it", -1);
        // SAFETY: a live iterator, not otherwise accessed.
        let it = unsafe { &mut *it };
        match it.snap.get(it.cur) {
            None => ErrorCode::ITER_OVER.raw(),
            Some(e) => {
                it.cur += 1;
                // SAFETY: writable.
                unsafe { out.write(e.cast::<GitIndexEntry>().cast_const()) };
                0
            }
        }
    })
}

/// `git_index_iterator_free`.
///
/// # Safety
///
/// `it` NULL or an iterator of [`git_index_iterator_new`], not used
/// afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_iterator_free(it: *mut GitIndexIterator) {
    barrier(|| {
        if it.is_null() {
            return;
        }
        // SAFETY: made by `Box::into_raw`, freed once.
        let it = unsafe { Box::from_raw(it) };
        let GitIndexIterator { index, snap, .. } = *it;
        ops::snapshot_release(snap, index);
    })
}

/// `git_index_conflict_iterator_new`.
///
/// # Safety
///
/// `iterator_out` NULL or writable; `index` NULL or live (for the
/// iterator's life).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_conflict_iterator_new(
    iterator_out: *mut *mut GitIndexConflictIterator,
    index: *mut GitIndex,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!iterator_out.is_null(), "iterator_out", -1);
        let ix = index_arg!(index, -1);
        let it = Box::new(GitIndexConflictIterator { index: ix, cur: 0 });
        // SAFETY: writable.
        unsafe { iterator_out.write(Box::into_raw(it)) };
        0
    })
}

/// `git_index_conflict_next`.
///
/// # Safety
///
/// The outputs NULL or writable; `iterator` NULL or a conflict iterator
/// whose index is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_conflict_next(
    ancestor_out: *mut *const GitIndexEntry,
    our_out: *mut *const GitIndexEntry,
    their_out: *mut *const GitIndexEntry,
    iterator: *mut GitIndexConflictIterator,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!ancestor_out.is_null(), "ancestor_out", -1);
        git_assert_arg!(!our_out.is_null(), "our_out", -1);
        git_assert_arg!(!their_out.is_null(), "their_out", -1);
        git_assert_arg!(!iterator.is_null(), "iterator", -1);
        let out = [ancestor_out, our_out, their_out];
        // SAFETY: writable.
        unsafe { write_conflict(out, [None; 3]) };
        // SAFETY: a live iterator.
        let it = unsafe { &mut *iterator };
        // SAFETY: no foreign call.
        let index = unsafe { it.index.get() };
        match super::api::conflict_next(index, &mut it.cur) {
            Some(sides) => {
                // SAFETY: writable.
                unsafe { write_conflict(out, sides) };
                0
            }
            None => ErrorCode::ITER_OVER.raw(),
        }
    })
}

/// `git_index_conflict_iterator_free`.
///
/// # Safety
///
/// `iterator` NULL or a conflict iterator, not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_conflict_iterator_free(iterator: *mut GitIndexConflictIterator) {
    barrier(|| {
        if !iterator.is_null() {
            // SAFETY: made by `Box::into_raw`, freed once.
            drop(unsafe { Box::from_raw(iterator) });
        }
    })
}

/// `git_index_snapshot_new`.
///
/// # Safety
///
/// `snap` writable for a `git_vector` (its old contents are overwritten,
/// not freed); `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_snapshot_new(
    snap: *mut GitVector,
    index: *mut GitIndex,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        let (v, r) = match ops::snapshot_new(ix) {
            Ok(v) => (v, 0),
            Err(e) => (GitVector::new(), e.raw()),
        };
        // SAFETY: writable (raw write: no old value is dropped).
        unsafe { snap.write(v) };
        r
    })
}

/// `git_index_snapshot_release`.
///
/// # Safety
///
/// `snap` a snapshot of [`git_index_snapshot_new`] of `index`, which it
/// releases (the vector is left empty).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_snapshot_release(snap: *mut GitVector, index: *mut GitIndex) {
    barrier(|| {
        // SAFETY: the export's contract.
        let Some(ix) = (unsafe { index_or_assert(index) }) else {
            return;
        };
        let v = if snap.is_null() {
            GitVector::new()
        } else {
            // SAFETY: a valid vector (the contract), taken out and left
            // empty.
            core::mem::take(unsafe { &mut *snap })
        };
        ops::snapshot_release(v, ix);
    })
}

/// `git_index_snapshot_find`.
///
/// # Safety
///
/// `out` NULL or writable; `snap` a snapshot (or another vector of index
/// entries with a sound comparison); `entry_srch` sound on them with an
/// `entry_srch_key`; `path` readable for `path_len` bytes (a string when
/// 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_snapshot_find(
    out: *mut usize,
    snap: *mut GitVector,
    entry_srch_fn: Option<VectorCmp>,
    path: *const c_char,
    path_len: usize,
    stage: c_int,
) -> c_int {
    barrier(|| {
        // SAFETY: the contract.
        let p = unsafe { path_arg(path, path_len) };
        // SAFETY: a valid vector, not otherwise accessed.
        let v = unsafe { &mut *snap };
        let f = entry_srch_fn.map(|f| f as usize);
        let ours = (
            git_index_entry_srch as VectorCmp as usize,
            git_index_entry_isrch as VectorCmp as usize,
        );
        let own_cmp = |c: Option<VectorCmp>| {
            let c = c.map(|f| f as usize);
            c == Some(git_index_entry_cmp as VectorCmp as usize)
                || c == Some(git_index_entry_icmp as VectorCmp as usize)
        };
        let r = if (f == Some(ours.0) || f == Some(ours.1)) && own_cmp(v.cmp_fn()) {
            // the Rust search, as `index_find_in_entries` would through
            // the C function pointers
            let sort_icase =
                v.cmp_fn().map(|c| c as usize) == Some(git_index_entry_icmp as VectorCmp as usize);
            super::sort_entries(v, sort_icase);
            super::find_in_entries(v, p, stage, f == Some(ours.1))
        } else {
            let Some(srch) = entry_srch_fn else {
                let _ = errors::assert_failed("key_lookup");
                return -1;
            };
            if v.cmp_fn().is_none() {
                return -1;
            }
            let key = EntrySrchKey {
                path: p.as_ptr().cast(),
                pathlen: p.len(),
                stage,
            };
            let mut pos = 0;
            // SAFETY: the caller vouches for the functions on the vector.
            let r = unsafe { v.bsearch2(Some(&mut pos), srch, (&raw const key).cast()) };
            match r {
                Ok(()) => Ok(pos),
                Err(e) if e == ErrorCode::NOT_FOUND => Err(pos),
                Err(e) => return e.raw(),
            }
        };
        let (pos, ret) = match r {
            Ok(p) => (p, 0),
            Err(p) => (p, ErrorCode::NOT_FOUND.raw()),
        };
        // SAFETY: NULL or writable.
        unsafe { crate::util::export::write_opt(out, pos) };
        ret
    })
}

/// `git_index__set_ignore_case`.
///
/// # Safety
///
/// `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index__set_ignore_case(index: *mut GitIndex, ignore_case: bool) {
    barrier(|| {
        // SAFETY: the export's contract.
        let Some(ix) = (unsafe { index_or_assert(index) }) else {
            return;
        };
        // SAFETY: no foreign call.
        unsafe { ix.get() }.set_ignore_case(ignore_case);
    })
}

/// `git_index_read_tree`.
///
/// # Safety
///
/// `index` NULL or live; `tree` NULL or a live `git_tree`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_read_tree(index: *mut GitIndex, tree: *const c_void) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        let Some(t) = NonNull::new(tree.cast_mut()) else {
            let _ = errors::assert_arg_failed("tree");
            return -1;
        };
        // SAFETY: a live tree for the call.
        status(ops::read_tree(ix, unsafe { TreeRef::new(t) }))
    })
}

/// `git_index_read_index`.
///
/// # Safety
///
/// `index` and `new_index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_read_index(
    index: *mut GitIndex,
    new_index: *const GitIndex,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        let nx = index_arg!(new_index, -1);
        status(ops::read_index(ix, nx))
    })
}

// ---------------------------------------------------------------------------
// Pathspec operations
// ---------------------------------------------------------------------------

/// `git_index_add_all`.
///
/// # Safety
///
/// `index` NULL or live; `paths` NULL or a valid `git_strarray`; `cb` and
/// `payload` the caller's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_add_all(
    index: *mut GitIndex,
    paths: *const GitStrArray,
    flags: c_uint,
    cb: Option<GitIndexMatchedPathCb>,
    payload: *mut c_void,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        let mut f = c_callback(cb, payload);
        let cbr: Option<MatchedPathCb<'_>> = f.as_mut().map(|f| f as MatchedPathCb<'_>);
        // SAFETY: the contract.
        ops::add_all(ix, unsafe { Paths::from_raw(paths) }, flags, cbr)
    })
}

/// `git_index_remove_all`.
///
/// # Safety
///
/// As [`git_index_add_all`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_remove_all(
    index: *mut GitIndex,
    pathspec: *const GitStrArray,
    cb: Option<GitIndexMatchedPathCb>,
    payload: *mut c_void,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        let mut f = c_callback(cb, payload);
        let cbr: Option<MatchedPathCb<'_>> = f.as_mut().map(|f| f as MatchedPathCb<'_>);
        // SAFETY: the contract.
        ops::remove_all(ix, unsafe { Paths::from_raw(pathspec) }, cbr)
    })
}

/// `git_index_update_all`.
///
/// # Safety
///
/// As [`git_index_add_all`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_update_all(
    index: *mut GitIndex,
    pathspec: *const GitStrArray,
    cb: Option<GitIndexMatchedPathCb>,
    payload: *mut c_void,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        let mut f = c_callback(cb, payload);
        let cbr: Option<MatchedPathCb<'_>> = f.as_mut().map(|f| f as MatchedPathCb<'_>);
        // SAFETY: the contract.
        ops::update_all(ix, unsafe { Paths::from_raw(pathspec) }, cbr)
    })
}

// ---------------------------------------------------------------------------
// Extensions
// ---------------------------------------------------------------------------

/// A signature argument: up to 4 bytes, ending at a NUL (C read 4 bytes
/// whatever the string's length).
///
/// # Safety
///
/// `s` is a string.
unsafe fn sig_arg(s: *const c_char) -> [u8; 4] {
    let mut sig = [0u8; 4];
    for (i, b) in sig.iter_mut().enumerate() {
        // SAFETY: a string: each byte up to its NUL is readable.
        let c = unsafe { *s.wrapping_add(i) } as u8;
        if c == 0 {
            break;
        }
        *b = c;
    }
    sig
}

/// `git_buf`'s fields.
#[repr(C)]
pub struct RawBuf {
    ptr: *mut c_char,
    reserved: usize,
    size: usize,
}

/// `git_index_extension_lookup`.
///
/// # Safety
///
/// `out` NULL or a writable `git_buf`; `index` NULL or live; `signature`
/// NULL or a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_extension_lookup(
    out: *mut RawBuf,
    index: *mut GitIndex,
    signature: *const c_char,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        let ix = index_arg!(index, -1);
        git_assert_arg!(!signature.is_null(), "signature", -1);
        // SAFETY: writable; only `ptr` and `size` are written, as C.
        let out = unsafe { &mut *out };
        out.size = 0;
        out.ptr = ptr::null_mut();
        // SAFETY: a string.
        let sig = unsafe { sig_arg(signature) };
        let key = &sig[..sig.iter().position(|&b| b == 0).unwrap_or(4)];
        // SAFETY: no foreign call.
        let index = unsafe { ix.get() };
        let Ok(pos) = index.extension_find(key) else {
            return ErrorCode::NOT_FOUND.raw();
        };
        let data = &index.extensions[pos].data;
        let p = crate::util::alloc::malloc(data.len()).cast::<u8>();
        if p.is_null() {
            return -1;
        }
        // SAFETY: `p` has room for the data; distinct.
        unsafe { ptr::copy_nonoverlapping(data.as_ptr(), p, data.len()) };
        out.ptr = p.cast();
        out.size = data.len();
        0
    })
}

/// `git_index_extension_add`.
///
/// # Safety
///
/// `index` NULL or live; `signature` NULL or a string; `data` readable for
/// `data_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_extension_add(
    index: *mut GitIndex,
    signature: *const c_char,
    data: *const c_char,
    data_len: usize,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        git_assert_arg!(!signature.is_null(), "signature", -1);
        git_assert!(data_len < u32::MAX as usize, "data_len < UINT32_MAX", -1);
        // SAFETY: the contract.
        let sig = unsafe { sig_arg(signature) };
        // SAFETY: the contract.
        let data = unsafe { crate::util::export::bytes(data, data_len) };
        // SAFETY: no foreign call.
        status(unsafe { ix.get() }.extension_add(sig, data))
    })
}

/// `git_index_extension_remove`.
///
/// # Safety
///
/// `index` NULL or live; `signature` NULL or a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_extension_remove(
    index: *mut GitIndex,
    signature: *const c_char,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        git_assert_arg!(!signature.is_null(), "signature", -1);
        // SAFETY: a string.
        let sig = unsafe { sig_arg(signature) };
        let key = &sig[..sig.iter().position(|&b| b == 0).unwrap_or(4)];
        // SAFETY: no foreign call.
        status(unsafe { ix.get() }.extension_remove(key))
    })
}

// ---------------------------------------------------------------------------
// NAME and REUC entries
// ---------------------------------------------------------------------------

/// `git_index_name_entrycount`.
///
/// # Safety
///
/// `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_name_entrycount(index: *mut GitIndex) -> usize {
    barrier(|| {
        let ix = index_arg!(index, usize::MAX);
        // SAFETY: no foreign call.
        unsafe { ix.get() }.names.len()
    })
}

/// `git_index_name_get_byindex`.
///
/// # Safety
///
/// `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_name_get_byindex(
    index: *mut GitIndex,
    n: usize,
) -> *const GitIndexNameEntry {
    barrier(|| {
        let ix = index_arg!(index, ptr::null());
        // SAFETY: no foreign call.
        let i = unsafe { ix.get() };
        i.sort_names();
        i.names
            .get(n)
            .map_or(ptr::null(), |p| p.cast::<GitIndexNameEntry>().cast_const())
    })
}

/// `git_index_name_add`.
///
/// # Safety
///
/// `index` NULL or live; the names NULL or strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_name_add(
    index: *mut GitIndex,
    ancestor: *const c_char,
    ours: *const c_char,
    theirs: *const c_char,
) -> c_int {
    barrier(|| {
        let (a, o, t) = (!ancestor.is_null(), !ours.is_null(), !theirs.is_null());
        git_assert_arg!(
            (a && o) || (a && t) || (o && t),
            "(ancestor && ours) || (ancestor && theirs) || (ours && theirs)",
            -1
        );
        let ix = index_arg!(index, -1);
        // SAFETY: strings or NULL.
        let [a, o, t] = [ancestor, ours, theirs].map(|p| unsafe { cstr(p) });
        // SAFETY: no foreign call.
        status(unsafe { ix.get() }.name_add(a, o, t))
    })
}

/// `git_index_name_clear`.
///
/// # Safety
///
/// `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_name_clear(index: *mut GitIndex) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        // SAFETY: no foreign call.
        unsafe { ix.get() }.name_clear();
        0
    })
}

/// `git_index_reuc_entrycount`.
///
/// # Safety
///
/// `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_reuc_entrycount(index: *mut GitIndex) -> usize {
    barrier(|| {
        let ix = index_arg!(index, usize::MAX);
        // SAFETY: no foreign call.
        unsafe { ix.get() }.reuc.len()
    })
}

/// `git_index_reuc_add`.
///
/// # Safety
///
/// `index` NULL or live; `path` NULL or a string; each id NULL or
/// readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_reuc_add(
    index: *mut GitIndex,
    path: *const c_char,
    ancestor_mode: c_int,
    ancestor_id: *const Oid,
    our_mode: c_int,
    our_id: *const Oid,
    their_mode: c_int,
    their_id: *const Oid,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        git_assert_arg!(!path.is_null(), "path", -1);
        // SAFETY: readable or NULL.
        let oids = [ancestor_id, our_id, their_id].map(|p| unsafe { p.as_ref() }.copied());
        // SAFETY: a string, copied (it may be a REUC entry's path).
        let p = unsafe { cstr_bytes(path) }.to_vec();
        // SAFETY: no foreign call.
        status(unsafe { ix.get() }.reuc_add(&p, [ancestor_mode, our_mode, their_mode], oids))
    })
}

/// `git_index_reuc_find`.
///
/// # Safety
///
/// `at_pos` NULL or writable; `index` NULL or live; `path` NULL or a
/// string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_reuc_find(
    at_pos: *mut usize,
    index: *mut GitIndex,
    path: *const c_char,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        git_assert_arg!(!path.is_null(), "key", -1);
        // SAFETY: a string.
        let p = unsafe { cstr_bytes(path) };
        // SAFETY: no foreign call.
        let (pos, r) = match unsafe { ix.get() }.reuc_find(p) {
            Ok(pos) => (pos, 0),
            Err(pos) => (pos, ErrorCode::NOT_FOUND.raw()),
        };
        // SAFETY: NULL or writable.
        unsafe { crate::util::export::write_opt(at_pos, pos) };
        r
    })
}

/// `git_index_reuc_get_bypath`.
///
/// # Safety
///
/// `index` NULL or live; `path` NULL or a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_reuc_get_bypath(
    index: *mut GitIndex,
    path: *const c_char,
) -> *const GitIndexReucEntry {
    barrier(|| {
        let ix = index_arg!(index, ptr::null());
        git_assert_arg!(!path.is_null(), "path", ptr::null());
        // SAFETY: no foreign call.
        let i = unsafe { ix.get() };
        if i.reuc.is_empty() {
            return ptr::null();
        }
        git_assert!(
            i.reuc.is_sorted(),
            "git_vector_is_sorted(&index->reuc)",
            ptr::null()
        );
        // SAFETY: a string.
        let p = unsafe { cstr_bytes(path) };
        match i.reuc_find(p) {
            Ok(pos) => i
                .reuc
                .get(pos)
                .map_or(ptr::null(), |r| r.cast::<GitIndexReucEntry>().cast_const()),
            Err(_) => ptr::null(),
        }
    })
}

/// `git_index_reuc_get_byindex`.
///
/// # Safety
///
/// `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_reuc_get_byindex(
    index: *mut GitIndex,
    n: usize,
) -> *const GitIndexReucEntry {
    barrier(|| {
        let ix = index_arg!(index, ptr::null());
        // SAFETY: no foreign call.
        let i = unsafe { ix.get() };
        git_assert!(
            i.reuc.is_sorted(),
            "git_vector_is_sorted(&index->reuc)",
            ptr::null()
        );
        i.reuc
            .get(n)
            .map_or(ptr::null(), |r| r.cast::<GitIndexReucEntry>().cast_const())
    })
}

/// `git_index_reuc_remove`.
///
/// # Safety
///
/// `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_reuc_remove(index: *mut GitIndex, position: usize) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        // SAFETY: no foreign call.
        status(unsafe { ix.get() }.reuc_remove(position))
    })
}

/// `git_index_reuc_clear`.
///
/// # Safety
///
/// `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_reuc_clear(index: *mut GitIndex) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        // SAFETY: no foreign call.
        unsafe { ix.get() }.reuc_clear();
        0
    })
}

// ---------------------------------------------------------------------------
// The index writer
// ---------------------------------------------------------------------------

/// `git_indexwriter_init`.
///
/// # Safety
///
/// `writer` a valid `git_indexwriter` (`GIT_INDEXWRITER_INIT`); `index`
/// NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_indexwriter_init(
    writer: *mut GitIndexWriter,
    index: *mut GitIndex,
) -> c_int {
    barrier(|| {
        let ix = index_arg!(index, -1);
        // SAFETY: a valid writer, which only this call accesses.
        status(ops::writer_init(unsafe { &mut *writer }, ix))
    })
}

/// `git_indexwriter_init_for_operation`.
///
/// # Safety
///
/// As [`git_indexwriter_init`]; `repo` a live repository;
/// `checkout_strategy` readable and writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_indexwriter_init_for_operation(
    writer: *mut GitIndexWriter,
    repo: *mut c_void,
    checkout_strategy: *mut c_uint,
) -> c_int {
    barrier(|| {
        // SAFETY: a live repository (the contract).
        let Some(repo) = (unsafe { Repo::new(repo) }) else {
            let _ = errors::assert_arg_failed("repo");
            return -1;
        };
        // SAFETY: valid, unaliased (the contract).
        status(ops::writer_init_for_operation(
            unsafe { &mut *writer },
            repo,
            unsafe { &mut *checkout_strategy },
        ))
    })
}

/// `git_indexwriter_commit`.
///
/// # Safety
///
/// `writer` a writer of [`git_indexwriter_init`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_indexwriter_commit(writer: *mut GitIndexWriter) -> c_int {
    barrier(|| {
        // SAFETY: valid, unaliased (the contract).
        status(ops::writer_commit(unsafe { &mut *writer }))
    })
}

/// `git_indexwriter_cleanup`.
///
/// # Safety
///
/// `writer` a valid writer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_indexwriter_cleanup(writer: *mut GitIndexWriter) {
    barrier(|| {
        // SAFETY: valid, unaliased (the contract).
        ops::writer_cleanup(unsafe { &mut *writer })
    })
}
