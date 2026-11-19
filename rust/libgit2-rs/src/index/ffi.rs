//! The C ABI of `index.c`: thin shims over [`GitIndex`]'s methods and the
//! [`ops`](super::ops).
//!
//! Every export takes the index as C passes it. Where the C checked an
//! argument (`GIT_ASSERT_ARG`), the export does with the same message;
//! where the C dereferenced a NULL index or path, the export fails the same
//! check instead (see `rust/DEFECTS.md`).
//!
//! An `index` that an export's `# Safety` calls live is, besides, used as
//! libgit2 documents: by one thread at a time, except that several threads
//! may run the read-only exports on it at once (C's read paths, as
//! `threads::diff` does: entry count and lookups, snapshots, `read`); and
//! no reference into it is held across the call. The read-only exports
//! borrow it shared ([`IndexPtr::get_ref`], [`IndexPtr::sorted`]), the
//! others exclusively ([`IndexPtr::get`]), each for one statement that
//! calls no foreign code.

use crate::git_arg;
use crate::util::cabi::{CStrArg, Out};
use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::{self, NonNull};

use super::foreign::{Paths, Repo, TreeRef};
use super::ops::{self, MatchedPathCb};
use super::{
    GIT_INDEX_OPTIONS_VERSION, GitIndex, GitIndexConflictIterator, GitIndexEntry, GitIndexIterator,
    GitIndexNameEntry, GitIndexOptions, GitIndexReucEntry, GitIndexWriter, INDEX_VERSION_NUMBER_LB,
    INDEX_VERSION_NUMBER_UB, IndexPtr, entry_srch,
};
use crate::oid::{Oid, OidType};
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::export::{barrier, cstr_bytes, status};
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
/// `path` is NULL, or readable for `path_len` bytes, or a string when
/// `path_len` is 0, unwritten for `'a`.
unsafe fn path_arg<'a>(path: *const c_char, path_len: usize) -> &'a [u8] {
    if path_len == 0 {
        // SAFETY: NULL or a string, unwritten for `'a` (this fn's contract).
        unsafe { cstr_bytes(path) }
    } else {
        // SAFETY: NULL or `path_len` readable bytes, unwritten for `'a`
        // (this fn's contract), which one object holds (so `path_len <=
        // isize::MAX`).
        unsafe { crate::util::export::bytes(path, path_len) }
    }
}

/// The handle of a non-null index pointer.
///
/// # Safety
///
/// `index` is NULL or a live index (this module's), used as the module
/// documentation says while the handle is.
#[inline(always)]
unsafe fn handle(index: *const GitIndex) -> Option<IndexPtr> {
    // SAFETY: a non-NULL `index` is a live index, used as `IndexPtr::new`
    // requires while the handle is (this fn's contract).
    NonNull::new(index.cast_mut()).map(|p| unsafe { IndexPtr::new(p) })
}

/// A C matched-path callback as the operations call it.
///
/// # Safety
///
/// `cb`, if any, may be called with `payload` and two strings (the second
/// NULL or a string) while the result lives.
unsafe fn c_callback<'a>(
    cb: Option<GitIndexMatchedPathCb>,
    payload: *mut c_void,
) -> Option<impl FnMut(CStrArg<'_>, CStrArg<'_>) -> c_int + 'a> {
    cb.map(move |f| {
        move |p: CStrArg<'_>, m: CStrArg<'_>| {
            // SAFETY: `f` may be called with `payload` (this fn's contract,
            // the export's), and the strings are NULL or NUL-terminated and
            // live for the call (`CStrArg`).
            unsafe { f(p.as_ptr(), m.as_ptr(), payload) }
        }
    })
}

// ---------------------------------------------------------------------------
// Comparison and search functions (C sorts and searches with them)
// ---------------------------------------------------------------------------

/// A `git_index_entry` and its path (a C string), borrowed for `'a`.
///
/// # Safety
///
/// `e` is a live `git_index_entry` whose path is a string, both unwritten
/// for `'a`.
unsafe fn any_entry<'a>(e: *const c_void) -> (&'a GitIndexEntry, &'a [u8]) {
    // SAFETY: a live, unwritten entry (this fn's contract).
    let e = unsafe { &*e.cast::<GitIndexEntry>() };
    // SAFETY: as above: its path is a string, unwritten for `'a`.
    (e, unsafe { cstr_bytes(e.path) })
}

/// `git_index_entry_cmp`.
///
/// # Safety
///
/// `a` and `b` are live `git_index_entry`s with string paths, unwritten
/// during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_entry_cmp(a: *const c_void, b: *const c_void) -> c_int {
    // SAFETY: a live entry with a string path, unwritten during the call
    // (this export's contract).
    let (a, pa) = unsafe { any_entry(a) };
    // SAFETY: as above, for `b`.
    let (b, pb) = unsafe { any_entry(b) };
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
    // SAFETY: as in `git_index_entry_cmp`.
    let (a, pa) = unsafe { any_entry(a) };
    // SAFETY: as in `git_index_entry_cmp`.
    let (b, pb) = unsafe { any_entry(b) };
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
/// bytes; `array_member` a live index entry (`entry_internal`); neither
/// written during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_entry_srch(
    key: *const c_void,
    array_member: *const c_void,
) -> c_int {
    // SAFETY: `srch`'s contract is this export's.
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
    // SAFETY: `srch`'s contract is this export's.
    unsafe { srch(key, array_member, true) }
}

/// The two search functions.
///
/// # Safety
///
/// As [`git_index_entry_srch`].
unsafe fn srch(key: *const c_void, member: *const c_void, icase: bool) -> c_int {
    // SAFETY: `key` is a live, unwritten key (this fn's contract).
    let k = unsafe { &*key.cast::<EntrySrchKey>() };
    // SAFETY: as above: its path is readable and unwritten for `pathlen`
    // bytes, which one object holds (so within `isize::MAX`).
    let path = unsafe { crate::util::export::bytes(k.path, k.pathlen) };
    // SAFETY: `member` is a live `entry_internal` (an `entry_alloc` entry),
    // unwritten during the call (this fn's contract).
    unsafe { entry_srch(path, k.stage, member.cast(), icase) }
}

// ---------------------------------------------------------------------------
// Opening and freeing
// ---------------------------------------------------------------------------

/// `git_error__check_version`.
fn check_version(opts: Option<&GitIndexOptions>, expected_max: c_uint, name: &str) -> Result<()> {
    let Some(opts) = opts else {
        return Ok(());
    };
    let actual = opts.version;
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
    mut opts: Out<'_, GitIndexOptions>,
    version: c_uint,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!opts.is_null(), "opts", -1);
        let probe = GitIndexOptions {
            version,
            oid_type: 0,
        };
        if check_version(Some(&probe), GIT_INDEX_OPTIONS_VERSION, "git_index_options").is_err() {
            return -1;
        }
        opts.write(GitIndexOptions {
            version: GIT_INDEX_OPTIONS_VERSION,
            oid_type: 0,
        });
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
    index_out: Out<'_, *mut GitIndex>,
    index_path: CStrArg<'_>,
    opts: Option<&GitIndexOptions>,
) -> c_int {
    open_ext(index_out, index_path, opts)
}

/// The body of [`git_index_open_ext`] and the exports that forward to it.
fn open_ext(
    mut index_out: Out<'_, *mut GitIndex>,
    index_path: CStrArg<'_>,
    opts: Option<&GitIndexOptions>,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!index_out.is_null(), "index_out", -1);
        if check_version(opts, GIT_INDEX_OPTIONS_VERSION, "git_index_options").is_err() {
            return -1;
        }
        let raw_type = opts.map_or(0, |o| o.oid_type);
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
        let path = index_path.get();
        match ops::open_ext(path, t) {
            Ok(ix) => {
                index_out.write(ix.as_ptr());
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
    index_out: Out<'_, *mut GitIndex>,
    index_path: CStrArg<'_>,
) -> c_int {
    open_ext(index_out, index_path, None)
}

/// `git_index_new`.
///
/// # Safety
///
/// `out` is NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_new(out: Out<'_, *mut GitIndex>) -> c_int {
    open_ext(out, CStrArg::NULL, None)
}

/// `git_index_new_ext`.
///
/// # Safety
///
/// As [`git_index_open_ext`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_new_ext(
    out: Out<'_, *mut GitIndex>,
    opts: Option<&GitIndexOptions>,
) -> c_int {
    open_ext(out, CStrArg::NULL, opts)
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
        // SAFETY: NULL or a live index on which the caller holds a
        // reference (this export's contract), used only below.
        if let Some(ix) = unsafe { handle(index) } {
            // SAFETY: the caller's reference, which it gives up (this
            // export's contract).
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
pub unsafe extern "C" fn git_index_clear(index: Option<IndexPtr>) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        // SAFETY: the index is live, and no other thread uses it (this
        // export's contract); this export holds no other reference to it, and
        // `clear` calls no foreign code: the borrow ends with the statement.
        status(unsafe { ix.get() }.clear())
    })
}

/// `git_index_owner`.
///
/// # Safety
///
/// `index` is a live index (NULL: NULL).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_owner(index: Option<IndexPtr>) -> *mut c_void {
    barrier(|| {
        let ix = git_arg!(index, "index", ptr::null_mut());
        ix.owner()
    })
}

/// `git_index_path`.
///
/// # Safety
///
/// `index` is NULL or a live index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_path(index: Option<IndexPtr>) -> *const c_char {
    barrier(|| {
        let ix = git_arg!(index, "index", ptr::null());
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no `&mut` to
        // it, and the shared borrow ends before any foreign call.
        unsafe { ix.get_ref() }.index_file_path
    })
}

/// `git_index_caps`.
///
/// # Safety
///
/// `index` is a live index (NULL: 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_caps(index: Option<IndexPtr>) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", 0);
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no `&mut` to
        // it, and the shared borrow ends before any foreign call.
        let i = unsafe { ix.get_ref() };
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
pub unsafe extern "C" fn git_index_set_caps(index: Option<IndexPtr>, caps: c_int) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        status(ops::set_caps(ix, caps))
    })
}

/// `git_index_version`.
///
/// # Safety
///
/// `index` is NULL or a live index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_version(index: Option<IndexPtr>) -> c_uint {
    barrier(|| {
        let ix = git_arg!(index, "index", c_uint::MAX);
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no `&mut` to
        // it, and the shared borrow ends before any foreign call.
        unsafe { ix.get_ref() }.version
    })
}

/// `git_index_set_version`.
///
/// # Safety
///
/// `index` is NULL or a live index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_set_version(index: Option<IndexPtr>, version: c_uint) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        if !(INDEX_VERSION_NUMBER_LB..=INDEX_VERSION_NUMBER_UB).contains(&version) {
            return git_error_set!(ErrorClass::INDEX, "invalid version number").raw();
        }
        // SAFETY: the index is live, and no other thread uses it (this
        // export's contract); this export holds no other reference to it:
        // the borrow ends with the assignment.
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
pub unsafe extern "C" fn git_index_read(index: Option<IndexPtr>, force: c_int) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
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
pub unsafe extern "C" fn git_index_read_safely(index: Option<IndexPtr>) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
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
pub unsafe extern "C" fn git_index_write(index: Option<IndexPtr>) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        status(ops::write(ix))
    })
}

/// `git_index_write_tree`.
///
/// # Safety
///
/// `oid` is NULL or writable; `index` NULL or a live index that nothing
/// else (another thread, or the C this call runs) uses during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_write_tree(oid: *mut Oid, index: *mut GitIndex) -> c_int {
    barrier(|| {
        git_assert_arg!(!oid.is_null(), "oid", -1);
        // SAFETY: the export's contract: NULL or a live index.
        let ix = unsafe { handle(index) };
        let ix = git_arg!(ix, "index", -1);
        // SAFETY: the owner outlives the index.
        let Some(repo) = (unsafe { Repo::new(ix.owner()) }) else {
            return errors::set_str(
                ErrorClass::INDEX,
                "Failed to write tree. the index file is not backed up by an existing repository",
            )
            .raw();
        };
        // SAFETY: `oid` is non-null (checked above) and writable (this
        // export's contract), and no reference to it is live.
        let oid = unsafe { Out::from_ptr(oid) };
        // SAFETY: this export holds no reference into the index, and
        // nothing else uses it during the call (this export's contract).
        // `oid` is written where C's `git_tree__write_index` writes it.
        unsafe { super::foreign::tree_write_index_to(oid, ix, repo) }
    })
}

/// `git_index_write_tree_to`.
///
/// # Safety
///
/// As [`git_index_write_tree`]; `repo` NULL or a live repository.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_write_tree_to(
    oid: Out<'_, Oid>,
    index: Option<IndexPtr>,
    repo: *mut c_void,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!oid.is_null(), "oid", -1);
        let ix = git_arg!(index, "index", -1);
        // SAFETY: NULL or a live repository for the call (this export's
        // contract), for which the handle is used.
        let Some(repo) = (unsafe { Repo::new(repo) }) else {
            let _ = errors::assert_arg_failed("repo");
            return -1;
        };
        // SAFETY: this export holds no reference into the index, and
        // nothing else uses it during the call (this export's contract).
        // `oid` is written where C's `git_tree__write_index` writes it.
        unsafe { super::foreign::tree_write_index_to(oid, ix, repo) }
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
pub unsafe extern "C" fn git_index_entrycount(index: Option<IndexPtr>) -> usize {
    barrier(|| {
        let ix = git_arg!(index, "index", usize::MAX);
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no `&mut` to
        // it, and the shared borrow ends before any foreign call.
        unsafe { ix.get_ref() }.entries.len()
    })
}

/// `git_index_get_byindex`.
///
/// # Safety
///
/// `index` is NULL or a live index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_get_byindex(
    index: Option<IndexPtr>,
    n: usize,
) -> *const GitIndexEntry {
    barrier(|| {
        let ix = git_arg!(index, "index", ptr::null());
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no other
        // reference to it, and the borrow, shared but for a needed sort,
        // ends before any foreign call.
        let i = unsafe { ix.sorted() };

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
    index: Option<IndexPtr>,
    path: CStrArg<'_>,
    stage: c_int,
) -> *const GitIndexEntry {
    barrier(|| {
        let ix = git_arg!(index, "index", ptr::null());
        let path = path.to_bytes();
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no `&mut` to
        // it, and the shared borrow ends before any foreign call.
        match unsafe { ix.get_ref() }.get_bypath(path, stage) {
            Some(e) => e.as_ptr().as_ptr().cast::<GitIndexEntry>().cast_const(),
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
/// `entry` is NULL or a valid entry that nothing else accesses during the
/// call; `st` NULL or readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_entry__init_from_stat(
    entry: Option<&mut GitIndexEntry>,
    st: Option<&libc::stat>,
    trust_mode: bool,
) {
    barrier(|| {
        // (C dereferences both unchecked)
        let entry = git_arg!(entry, "entry", ());
        let st = git_arg!(st, "st", ());
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
/// `entry` is NULL or a readable entry (NULL: 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_entry_stage(entry: Option<&GitIndexEntry>) -> c_int {
    entry.map_or(0, GitIndexEntry::stage)
}

/// `git_index_entry_is_conflict`.
///
/// # Safety
///
/// As [`git_index_entry_stage`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_entry_is_conflict(entry: Option<&GitIndexEntry>) -> c_int {
    c_int::from(entry.map_or(0, GitIndexEntry::stage) > 0)
}

/// `GIT_ASSERT_ARG(source_entry && source_entry->path)`: the entry and
/// its path.
///
/// # Safety
///
/// `source` is NULL or a readable entry with a NULL or string path, both
/// unwritten for `'a`.
unsafe fn source_arg<'a>(source: *const GitIndexEntry) -> Option<(&'a GitIndexEntry, &'a [u8])> {
    // SAFETY: NULL or a readable entry, unwritten for `'a` (this fn's
    // contract).
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
    index: Option<IndexPtr>,
    source_entry: *const GitIndexEntry,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        // SAFETY: NULL or a readable entry with a NULL or string path
        // (this export's contract), unwritten until it is copied just below.
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
    index: Option<IndexPtr>,
    source_entry: *const GitIndexEntry,
    buffer: *const c_void,
    len: usize,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        // SAFETY: NULL or a readable entry with a NULL or string path
        // (this export's contract), unwritten until it is copied just below.
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
pub unsafe extern "C" fn git_index_add_bypath(index: Option<IndexPtr>, path: CStrArg<'_>) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        let path = git_arg!(path.get(), "path", -1);
        let p = match ops::c_copy(path.to_bytes()) {
            Ok(p) => p,
            Err(e) => return e.raw(),
        };
        status(ops::add_bypath(ix, p.as_c_str()))
    })
}

/// `git_index_remove_bypath`.
///
/// # Safety
///
/// As [`git_index_add_bypath`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_remove_bypath(
    index: Option<IndexPtr>,
    path: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        let path = git_arg!(path.get(), "path", -1);
        let p = path.to_bytes().to_vec();
        // SAFETY: the index is live, and no other thread uses it (this
        // export's contract); this export holds no other reference to it, and
        // `remove_bypath` calls no foreign code: the borrow ends with the statement.
        status(ops::remove_bypath(unsafe { ix.get() }, &p))
    })
}

/// `git_index__fill`.
///
/// # Safety
///
/// `index` NULL or a live index; `source_entries` NULL or a vector of live
/// entries with string paths, none of them `index`'s, that nothing writes
/// or frees during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index__fill(
    index: Option<IndexPtr>,
    source_entries: Option<&GitVector>,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        // (C dereferences a NULL `source_entries` next.)
        let source_entries = git_arg!(source_entries, "source_entries", -1);
        let v: Vec<*const GitIndexEntry> = source_entries
            .iter()
            .map(|p| p.cast::<GitIndexEntry>().cast_const())
            .collect();
        // SAFETY: live entries with string paths, not the index's,
        // unwritten during the call (this export's contract).
        status(unsafe { ops::fill(ix, &v) })
    })
}

/// `git_index_remove`.
///
/// # Safety
///
/// `index` NULL or a live index; `path` NULL or a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_remove(
    index: Option<IndexPtr>,
    path: CStrArg<'_>,
    stage: c_int,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        let path = git_arg!(path.get(), "path", -1);
        let p = path.to_bytes().to_vec();
        // SAFETY: the index is live, and no other thread uses it (this
        // export's contract); this export holds no other reference to it, and
        // `remove` calls no foreign code: the borrow ends with the statement.
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
    index: Option<IndexPtr>,
    dir: CStrArg<'_>,
    stage: c_int,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        let dir = git_arg!(dir.get(), "dir", -1);
        let mut pfx = GitStr::new();
        if let Err(e) = pfx.sets(dir.to_bytes()) {
            return e.raw();
        }
        if let Err(e) = crate::util::fs_path::to_dir(&mut pfx) {
            return e.raw();
        }
        // SAFETY: the index is live, and no other thread uses it (this
        // export's contract); this export holds no other reference to it, and
        // `remove_directory` calls no foreign code: the borrow ends with the statement.
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
    mut at_pos: Out<'_, usize>,
    index: Option<IndexPtr>,
    prefix: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        let prefix = git_arg!(prefix.get(), "prefix", -1);
        let p = prefix.to_bytes();
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no other
        // reference to it, and the borrow, shared but for a needed sort,
        // ends before any foreign call.
        match unsafe { ix.sorted() }.find_prefix_sorted(p) {
            Some(pos) => {
                at_pos.write(pos);
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
    mut out: Out<'_, usize>,
    index: Option<IndexPtr>,
    path: *const c_char,
    path_len: usize,
    stage: c_int,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        git_assert_arg!(!path.is_null(), "path", -1);
        // SAFETY: readable for `path_len` bytes, or a string when 0, and
        // unwritten during the call (this export's contract).
        let p = unsafe { path_arg(path, path_len) };
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no other
        // reference to it, and the borrow, shared but for a needed sort,
        // ends before any foreign call.
        let r = unsafe { ix.sorted() }.find_sorted(p, stage);
        let (pos, ret) = match r {
            Ok(p) => (p, 0),
            Err(p) => (p, ErrorCode::NOT_FOUND.raw()),
        };
        out.write(pos);
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
    mut at_pos: Out<'_, usize>,
    index: Option<IndexPtr>,
    path: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        let path = git_arg!(path.get(), "path", -1);
        let p = path.to_bytes();
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no other
        // reference to it, and the borrow, shared but for a needed sort,
        // ends before any foreign call.
        match unsafe { ix.sorted() }.find_path_sorted(p) {
            Some(pos) => {
                at_pos.write(pos);
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

/// A copy of the caller's entry `p` and of its path, or `None` for NULL.
///
/// # Safety
///
/// `p` is NULL or a readable entry whose path is NULL or a string.
unsafe fn entry_copy(p: *const GitIndexEntry) -> Option<(GitIndexEntry, Vec<u8>)> {
    // SAFETY: NULL or a readable entry (this fn's contract), copied before
    // anything else runs.
    let e = unsafe { p.as_ref() }?;
    // SAFETY: as above: its path is NULL or a string.
    Some((*e, unsafe { cstr_bytes(e.path) }.to_vec()))
}

/// `git_index_conflict_add`.
///
/// # Safety
///
/// `index` NULL or live; each entry NULL or readable with a string path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_conflict_add(
    index: Option<IndexPtr>,
    ancestor_entry: *const GitIndexEntry,
    our_entry: *const GitIndexEntry,
    their_entry: *const GitIndexEntry,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        // the entries are copied first (they may be the index's own)
        // SAFETY: each is NULL or a readable entry with a string path (this
        // export's contract), unwritten until copied here.
        let [a, o, t] = [ancestor_entry, our_entry, their_entry].map(|p| unsafe { entry_copy(p) });
        fn side(x: &Option<(GitIndexEntry, Vec<u8>)>) -> Option<(&GitIndexEntry, &[u8])> {
            x.as_ref().map(|(e, p)| (e, p.as_slice()))
        }
        status(ops::conflict_add(ix, [side(&a), side(&o), side(&t)]))
    })
}

/// The three out-parameters of a conflict.
type ConflictOut<'a> = [Out<'a, *const GitIndexEntry>; 3];

/// Write the three sides of a conflict, in order.
fn write_conflict(out: &mut ConflictOut<'_>, sides: [Option<super::EntryRef<'_>>; 3]) {
    for (o, side) in out.iter_mut().zip(sides) {
        o.write(side.map_or(ptr::null(), |e| {
            e.as_ptr().as_ptr().cast::<GitIndexEntry>().cast_const()
        }));
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
    ancestor_out: Out<'_, *const GitIndexEntry>,
    our_out: Out<'_, *const GitIndexEntry>,
    their_out: Out<'_, *const GitIndexEntry>,
    index: Option<IndexPtr>,
    path: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!ancestor_out.is_null(), "ancestor_out", -1);
        git_assert_arg!(!our_out.is_null(), "our_out", -1);
        git_assert_arg!(!their_out.is_null(), "their_out", -1);
        let ix = git_arg!(index, "index", -1);
        let path = git_arg!(path.get(), "path", -1);
        let mut out = [ancestor_out, our_out, their_out];
        write_conflict(&mut out, [None; 3]);
        let p = path.to_bytes();
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no other
        // reference to it, and the borrow, shared but for a needed sort,
        // ends before any foreign call.
        match unsafe { ix.sorted() }.conflict_get_sorted(p) {
            Ok(sides) => {
                write_conflict(&mut out, sides);
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
    index: Option<IndexPtr>,
    path: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        let path = git_arg!(path.get(), "path", -1);
        let p = path.to_bytes().to_vec();
        // SAFETY: the index is live, and no other thread uses it (this
        // export's contract); this export holds no other reference to it, and
        // `conflict_remove` calls no foreign code: the borrow ends with the statement.
        status(unsafe { ix.get() }.conflict_remove(Some(&p)))
    })
}

/// `git_index_conflict_cleanup`.
///
/// # Safety
///
/// `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_conflict_cleanup(index: Option<IndexPtr>) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        // SAFETY: the index is live, and no other thread uses it (this
        // export's contract); this export holds no other reference to it, and
        // `conflict_remove` calls no foreign code: the borrow ends with the statement.
        status(unsafe { ix.get() }.conflict_remove(None))
    })
}

/// `git_index_has_conflicts`.
///
/// # Safety
///
/// `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_has_conflicts(index: Option<IndexPtr>) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no `&mut` to
        // it, and the shared borrow ends before any foreign call.
        c_int::from(unsafe { ix.get_ref() }.has_conflicts())
    })
}

/// `git_index__conflict_to_reuc`.
///
/// # Safety
///
/// `index` NULL or live; `path` NULL or a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index__conflict_to_reuc(
    index: Option<IndexPtr>,
    path: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        let path = git_arg!(path.get(), "path", -1);
        let p = path.to_bytes().to_vec();
        // SAFETY: the index is live, and no other thread uses it (this
        // export's contract); this export holds no other reference to it, and
        // `conflict_to_reuc` calls no foreign code: the borrow ends with the statement.
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
    mut iterator_out: Out<'_, *mut GitIndexIterator>,
    index: Option<IndexPtr>,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!iterator_out.is_null(), "iterator_out", -1);
        let ix = git_arg!(index, "index", -1);
        match ops::snapshot_new(ix) {
            Ok(snap) => {
                let it = Box::new(GitIndexIterator {
                    index: ix,
                    snap,
                    cur: 0,
                });
                iterator_out.write(Box::into_raw(it));
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
/// [`git_index_iterator_new`], which nothing else uses during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_iterator_next(
    mut out: Out<'_, *const GitIndexEntry>,
    it: Option<&mut GitIndexIterator>,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!out.is_null(), "out", -1);
        let it = git_arg!(it, "it", -1);
        match it.snap.get(it.cur) {
            None => ErrorCode::ITER_OVER.raw(),
            Some(e) => {
                it.cur += 1;
                out.write(e.cast::<GitIndexEntry>().cast_const());
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
        // SAFETY: an iterator of `git_index_iterator_new` (its
        // `Box::into_raw`), which the caller gives up (this export's
        // contract): freed once.
        let it = unsafe { Box::from_raw(it) };
        let GitIndexIterator { index, snap, .. } = *it;
        // SAFETY: the iterator's snapshot, with its reader and reference
        // (`git_index_iterator_new`), given up with the iterator.
        unsafe { ops::snapshot_release(snap, index) };
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
    mut iterator_out: Out<'_, *mut GitIndexConflictIterator>,
    index: Option<IndexPtr>,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!iterator_out.is_null(), "iterator_out", -1);
        let ix = git_arg!(index, "index", -1);
        let it = Box::new(GitIndexConflictIterator { index: ix, cur: 0 });
        iterator_out.write(Box::into_raw(it));
        0
    })
}

/// `git_index_conflict_next`.
///
/// # Safety
///
/// The outputs NULL or writable; `iterator` NULL or a conflict iterator
/// whose index is live, which nothing else uses during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_conflict_next(
    ancestor_out: Out<'_, *const GitIndexEntry>,
    our_out: Out<'_, *const GitIndexEntry>,
    their_out: Out<'_, *const GitIndexEntry>,
    iterator: Option<&mut GitIndexConflictIterator>,
) -> c_int {
    barrier(|| {
        git_assert_arg!(!ancestor_out.is_null(), "ancestor_out", -1);
        git_assert_arg!(!our_out.is_null(), "our_out", -1);
        git_assert_arg!(!their_out.is_null(), "their_out", -1);
        let it = git_arg!(iterator, "iterator", -1);
        let mut out = [ancestor_out, our_out, their_out];
        write_conflict(&mut out, [None; 3]);
        let cur = it.cur;
        // SAFETY: the iterator's index is live (this export's contract) and
        // used as the module documentation says; this export holds no other
        // reference to it, and the borrow, shared but for a needed sort (as
        // `api::conflict_next`), ends before any foreign call.
        let index = unsafe {
            it.index.prepared(
                |i| cur >= i.entries.len() || i.entries.is_sorted(),
                GitIndex::sort,
            )
        };
        match super::api::conflict_next_sorted(index, &mut it.cur) {
            Some(sides) => {
                write_conflict(&mut out, sides);
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
            // SAFETY: an iterator of `git_index_conflict_iterator_new` (its
            // `Box::into_raw`), which the caller gives up (this export's
            // contract): freed once.
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
    mut snap: Out<'_, GitVector>,
    index: Option<IndexPtr>,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        let (v, r) = match ops::snapshot_new(ix) {
            Ok(v) => (v, 0),
            Err(e) => (GitVector::new(), e.raw()),
        };
        snap.write(v);
        r
    })
}

/// `git_index_snapshot_release`.
///
/// # Safety
///
/// `snap` NULL or a snapshot of [`git_index_snapshot_new`] of `index`,
/// which it releases (the vector is left empty), with the reader and
/// reference the snapshot holds; `index` NULL or that snapshot's index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_snapshot_release(
    snap: Option<&mut GitVector>,
    index: Option<IndexPtr>,
) {
    barrier(|| {
        let ix = git_arg!(index, "index", ());
        // taken out, and left empty
        let v = snap.map_or_else(GitVector::new, core::mem::take);
        // SAFETY: the snapshot's reader and reference on `index`, which the
        // caller gives up (this export's contract).
        unsafe { ops::snapshot_release(v, ix) };
    })
}

/// `git_index_snapshot_find`.
///
/// # Safety
///
/// `out` NULL or writable; `snap` NULL or a snapshot (or another vector of
/// live index entries, `entry_internal`s, with a sound comparison), which
/// nothing else uses, and whose entries nothing writes, during the call;
/// `entry_srch` sound on them with an `entry_srch_key`; `path` readable
/// for `path_len` bytes (a string when 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_snapshot_find(
    mut out: Out<'_, usize>,
    snap: Option<&mut GitVector>,
    entry_srch_fn: Option<VectorCmp>,
    path: *const c_char,
    path_len: usize,
    stage: c_int,
) -> c_int {
    barrier(|| {
        // SAFETY: readable for `path_len` bytes, or a string when 0, and
        // unwritten during the call (this export's contract).
        let p = unsafe { path_arg(path, path_len) };
        let snap = git_arg!(snap, "v", -1);
        let v = snap;
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
            // SAFETY: the snapshot's elements are live index entries
            // (`entry_internal`, as C's search functions require), unwritten
            // during the call (this export's contract).
            unsafe { super::sort_entries(v, sort_icase) };
            // SAFETY: as above.
            unsafe { super::find_in_entries(v, p, stage, f == Some(ours.1)) }
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
            // SAFETY: `srch` is sound on the vector's elements with an
            // `entry_srch_key` (this export's contract), and `key` is one,
            // whose path (`p`) is readable for the call.
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
        out.write(pos);
        ret
    })
}

/// `git_index__set_ignore_case`.
///
/// # Safety
///
/// `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index__set_ignore_case(index: Option<IndexPtr>, ignore_case: bool) {
    barrier(|| {
        let ix = git_arg!(index, "index", ());
        // SAFETY: the index is live, and no other thread uses it (this
        // export's contract); this export holds no other reference to it, and
        // `set_ignore_case` calls no foreign code: the borrow ends with the statement.
        unsafe { ix.get() }.set_ignore_case(ignore_case);
    })
}

/// `git_index_read_tree`.
///
/// # Safety
///
/// `index` NULL or live; `tree` NULL or a live `git_tree`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_read_tree(
    index: Option<IndexPtr>,
    tree: *const c_void,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        let Some(t) = NonNull::new(tree.cast_mut()) else {
            let _ = errors::assert_arg_failed("tree");
            return -1;
        };
        // SAFETY: a live tree for the call (this export's contract), which
        // the handle does not outlive.
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
    index: Option<IndexPtr>,
    new_index: Option<IndexPtr>,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        // C: `git_index_owner(new_index)`'s assertion
        let nx = git_arg!(new_index, "index", -1);
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
/// `index` NULL or live; `paths` NULL or a valid `git_strarray` whose
/// strings are NULL or NUL-terminated, unwritten during the call; `cb`
/// NULL or a function that may be called with `payload` and two strings
/// (the second NULL or a string) during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_add_all(
    index: Option<IndexPtr>,
    paths: Paths<'_>,
    flags: c_uint,
    cb: Option<GitIndexMatchedPathCb>,
    payload: *mut c_void,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        // SAFETY: `cb` may be called with `payload` during the call (this
        // export's contract), which `f` does not outlive.
        let mut f = unsafe { c_callback(cb, payload) };
        let cbr: Option<MatchedPathCb<'_>> = f.as_mut().map(|f| f as MatchedPathCb<'_>);
        ops::add_all(ix, paths, flags, cbr)
    })
}

/// `git_index_remove_all`.
///
/// # Safety
///
/// As [`git_index_add_all`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_remove_all(
    index: Option<IndexPtr>,
    pathspec: Paths<'_>,
    cb: Option<GitIndexMatchedPathCb>,
    payload: *mut c_void,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        // SAFETY: `cb` may be called with `payload` during the call (this
        // export's contract), which `f` does not outlive.
        let mut f = unsafe { c_callback(cb, payload) };
        let cbr: Option<MatchedPathCb<'_>> = f.as_mut().map(|f| f as MatchedPathCb<'_>);
        ops::remove_all(ix, pathspec, cbr)
    })
}

/// `git_index_update_all`.
///
/// # Safety
///
/// As [`git_index_add_all`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_update_all(
    index: Option<IndexPtr>,
    pathspec: Paths<'_>,
    cb: Option<GitIndexMatchedPathCb>,
    payload: *mut c_void,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        // SAFETY: `cb` may be called with `payload` during the call (this
        // export's contract), which `f` does not outlive.
        let mut f = unsafe { c_callback(cb, payload) };
        let cbr: Option<MatchedPathCb<'_>> = f.as_mut().map(|f| f as MatchedPathCb<'_>);
        ops::update_all(ix, pathspec, cbr)
    })
}

// ---------------------------------------------------------------------------
// Extensions
// ---------------------------------------------------------------------------

/// A signature argument: up to 4 bytes, ending at a NUL (C read 4 bytes
/// whatever the string's length).
fn sig_arg(s: &CStr) -> [u8; 4] {
    let mut sig = [0u8; 4];
    for (b, &c) in sig.iter_mut().zip(s.to_bytes()) {
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

impl Default for RawBuf {
    /// `GIT_BUF_INIT`.
    fn default() -> RawBuf {
        RawBuf {
            ptr: ptr::null_mut(),
            reserved: 0,
            size: 0,
        }
    }
}

impl RawBuf {
    /// `ptr`.
    pub fn ptr(&self) -> *mut c_char {
        self.ptr
    }
    /// `size`.
    pub fn size(&self) -> usize {
        self.size
    }
}

/// `git_index_extension_lookup`.
///
/// # Safety
///
/// `out` NULL or a writable `git_buf`; `index` NULL or live; `signature`
/// NULL or a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_extension_lookup(
    out: Option<&mut RawBuf>,
    index: Option<IndexPtr>,
    signature: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        let out = git_arg!(out, "out", -1);
        let ix = git_arg!(index, "index", -1);
        let signature = git_arg!(signature.get(), "signature", -1);
        out.size = 0;
        out.ptr = ptr::null_mut();
        let sig = sig_arg(signature);
        let key = &sig[..sig.iter().position(|&b| b == 0).unwrap_or(4)];
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no `&mut` to
        // it, and the shared borrow ends before any foreign call.
        let index = unsafe { ix.get_ref() };
        let Ok(pos) = index.extension_find(key) else {
            return ErrorCode::NOT_FOUND.raw();
        };
        let data = &index.extensions[pos].data;
        let p = crate::util::alloc::malloc(data.len()).cast::<u8>();
        if p.is_null() {
            return -1;
        }
        // SAFETY: `p` is a fresh `git__allocator` block of `data.len()`
        // bytes, writable and distinct from the index's data.
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
/// `data_len` bytes, unwritten during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_extension_add(
    index: Option<IndexPtr>,
    signature: CStrArg<'_>,
    data: *const c_char,
    data_len: usize,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        let signature = git_arg!(signature.get(), "signature", -1);
        git_assert!(data_len < u32::MAX as usize, "data_len < UINT32_MAX", -1);
        let sig = sig_arg(signature);
        // SAFETY: `data` is readable for `data_len` bytes (this export's
        // contract), which is below `u32::MAX` (checked above), so within
        // `isize::MAX`; nothing writes them during the call.
        let data = unsafe { crate::util::export::bytes(data, data_len) };
        // SAFETY: no reference into the index is live, and
        // `extension_add` calls no foreign code: the borrow ends with the
        // call.
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
    index: Option<IndexPtr>,
    signature: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        let signature = git_arg!(signature.get(), "signature", -1);
        let sig = sig_arg(signature);
        let key = &sig[..sig.iter().position(|&b| b == 0).unwrap_or(4)];
        // SAFETY: the index is live, and no other thread uses it (this
        // export's contract); this export holds no other reference to it, and
        // `extension_remove` calls no foreign code: the borrow ends with the statement.
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
pub unsafe extern "C" fn git_index_name_entrycount(index: Option<IndexPtr>) -> usize {
    barrier(|| {
        let ix = git_arg!(index, "index", usize::MAX);
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no `&mut` to
        // it, and the shared borrow ends before any foreign call.
        unsafe { ix.get_ref() }.names.len()
    })
}

/// `git_index_name_get_byindex`.
///
/// # Safety
///
/// `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_name_get_byindex(
    index: Option<IndexPtr>,
    n: usize,
) -> *const GitIndexNameEntry {
    barrier(|| {
        let ix = git_arg!(index, "index", ptr::null());
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no other
        // reference to it, and the borrow, shared but for a needed sort,
        // ends before any foreign call.
        let i = unsafe { ix.prepared(|i| i.names.is_sorted(), GitIndex::sort_names) };

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
    index: Option<IndexPtr>,
    ancestor: CStrArg<'_>,
    ours: CStrArg<'_>,
    theirs: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        let (a, o, t) = (!ancestor.is_null(), !ours.is_null(), !theirs.is_null());
        git_assert_arg!(
            (a && o) || (a && t) || (o && t),
            "(ancestor && ours) || (ancestor && theirs) || (ours && theirs)",
            -1
        );
        let ix = git_arg!(index, "index", -1);
        let [a, o, t] = [ancestor, ours, theirs].map(CStrArg::get);
        // SAFETY: the index is live, and no other thread uses it (this
        // export's contract); this export holds no other reference to it, and
        // `name_add` calls no foreign code: the borrow ends with the statement.
        status(unsafe { ix.get() }.name_add(a, o, t))
    })
}

/// `git_index_name_clear`.
///
/// # Safety
///
/// `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_name_clear(index: Option<IndexPtr>) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        // SAFETY: the index is live, and no other thread uses it (this
        // export's contract); this export holds no other reference to it, and
        // `name_clear` calls no foreign code: the borrow ends with the statement.
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
pub unsafe extern "C" fn git_index_reuc_entrycount(index: Option<IndexPtr>) -> usize {
    barrier(|| {
        let ix = git_arg!(index, "index", usize::MAX);
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no `&mut` to
        // it, and the shared borrow ends before any foreign call.
        unsafe { ix.get_ref() }.reuc.len()
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
    index: Option<IndexPtr>,
    path: CStrArg<'_>,
    ancestor_mode: c_int,
    ancestor_id: *const Oid,
    our_mode: c_int,
    our_id: *const Oid,
    their_mode: c_int,
    their_id: *const Oid,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        let path = git_arg!(path.get(), "path", -1);
        // (raw: an id may be one of the index's REUC records, which the
        // call frees; each is copied here, before anything runs)
        // SAFETY: each NULL or a readable id (this export's contract).
        let oids = [ancestor_id, our_id, their_id].map(|p| unsafe { p.as_ref() }.copied());
        let p = path.to_bytes().to_vec();
        // SAFETY: the index is live, and no other thread uses it (this
        // export's contract); this export holds no other reference to it, and
        // `reuc_add` calls no foreign code: the borrow ends with the statement.
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
    mut at_pos: Out<'_, usize>,
    index: Option<IndexPtr>,
    path: CStrArg<'_>,
) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        let path = git_arg!(path.get(), "key", -1);
        let p = path.to_bytes();
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no other
        // reference to it, and the borrow, shared but for a needed sort,
        // ends before any foreign call.
        let (pos, r) = match unsafe { ix.prepared(|i| i.reuc.is_sorted(), GitIndex::sort_reuc) }
            .reuc_find_sorted(p)
        {
            Ok(pos) => (pos, 0),
            Err(pos) => (pos, ErrorCode::NOT_FOUND.raw()),
        };
        at_pos.write(pos);
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
    index: Option<IndexPtr>,
    path: CStrArg<'_>,
) -> *const GitIndexReucEntry {
    barrier(|| {
        let ix = git_arg!(index, "index", ptr::null());
        let path = git_arg!(path.get(), "path", ptr::null());
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no `&mut` to
        // it, and the shared borrow ends before any foreign call.
        let i = unsafe { ix.get_ref() };
        if i.reuc.is_empty() {
            return ptr::null();
        }
        git_assert!(
            i.reuc.is_sorted(),
            "(((&index->reuc)->flags & GIT_VECTOR_SORTED) != 0)",
            ptr::null()
        );
        let p = path.to_bytes();
        match i.reuc_find_sorted(p) {
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
    index: Option<IndexPtr>,
    n: usize,
) -> *const GitIndexReucEntry {
    barrier(|| {
        let ix = git_arg!(index, "index", ptr::null());
        // SAFETY: the index is live, and other threads only read it (this
        // export's contract: C's read paths); this export holds no `&mut` to
        // it, and the shared borrow ends before any foreign call.
        let i = unsafe { ix.get_ref() };
        git_assert!(
            i.reuc.is_sorted(),
            "(((&index->reuc)->flags & GIT_VECTOR_SORTED) != 0)",
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
pub unsafe extern "C" fn git_index_reuc_remove(index: Option<IndexPtr>, position: usize) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        // SAFETY: the index is live, and no other thread uses it (this
        // export's contract); this export holds no other reference to it, and
        // `reuc_remove` calls no foreign code: the borrow ends with the statement.
        status(unsafe { ix.get() }.reuc_remove(position))
    })
}

/// `git_index_reuc_clear`.
///
/// # Safety
///
/// `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_index_reuc_clear(index: Option<IndexPtr>) -> c_int {
    barrier(|| {
        let ix = git_arg!(index, "index", -1);
        // SAFETY: the index is live, and no other thread uses it (this
        // export's contract); this export holds no other reference to it, and
        // `reuc_clear` calls no foreign code: the borrow ends with the statement.
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
/// `writer` NULL or a valid `git_indexwriter` (`GIT_INDEXWRITER_INIT`)
/// that nothing else uses during the call; `index` NULL or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_indexwriter_init(
    writer: Option<&mut GitIndexWriter>,
    index: Option<IndexPtr>,
) -> c_int {
    barrier(|| {
        // (C dereferences `writer` unchecked)
        let writer = git_arg!(writer, "writer", -1);
        let ix = git_arg!(index, "index", -1);
        status(ops::writer_init(writer, ix))
    })
}

/// `git_indexwriter_init_for_operation`.
///
/// # Safety
///
/// As [`git_indexwriter_init`]; `repo` NULL or a live repository;
/// `checkout_strategy` NULL or readable and writable, and distinct from
/// `writer`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_indexwriter_init_for_operation(
    writer: Option<&mut GitIndexWriter>,
    repo: *mut c_void,
    checkout_strategy: Option<&mut c_uint>,
) -> c_int {
    barrier(|| {
        // SAFETY: `repo` is NULL or a live repository (this export's
        // contract), which outlives the call.
        let Some(repo) = (unsafe { Repo::new(repo) }) else {
            let _ = errors::assert_arg_failed("repo");
            return -1;
        };
        // (C dereferences these unchecked)
        let writer = git_arg!(writer, "writer", -1);
        let checkout_strategy = git_arg!(checkout_strategy, "checkout_strategy", -1);
        status(ops::writer_init_for_operation(
            writer,
            repo,
            checkout_strategy,
        ))
    })
}

/// `git_indexwriter_commit`.
///
/// # Safety
///
/// `writer` NULL or a writer of [`git_indexwriter_init`] that nothing else
/// uses during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_indexwriter_commit(writer: Option<&mut GitIndexWriter>) -> c_int {
    barrier(|| {
        let writer = git_arg!(writer, "writer", -1);
        status(ops::writer_commit(writer))
    })
}

/// `git_indexwriter_cleanup`.
///
/// # Safety
///
/// `writer` NULL or a valid writer that nothing else uses during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn git_indexwriter_cleanup(writer: Option<&mut GitIndexWriter>) {
    barrier(|| {
        let writer = git_arg!(writer, "writer", ());
        ops::writer_cleanup(writer)
    })
}
