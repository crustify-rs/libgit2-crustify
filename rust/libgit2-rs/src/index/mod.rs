//! Port of `src/libgit2/index.c`: the index (staging area), its entries,
//! conflicts, resolve-undo (`REUC`) and conflict-name (`NAME`) records,
//! extensions, its on-disk format (versions 2 to 4, SHA-1 and SHA-256), and
//! snapshots for the C iterators.
//!
//! # Types
//!
//! - [`GitIndex`] (`git_index`) is a *shared prefix* (kind 2): only this
//!   module allocates it (in Rust's allocator), but the C that stays reads
//!   and writes some fields directly: `rc` (`GIT_REFCOUNT_OWN`), `entries`
//!   (iterator, merge, stash), `stamp`, `checksum`, the bitfields
//!   `on_disk`, `ignore_case`, `distrust_filemode` and `dirty`, `oid_type`,
//!   `tree` and `tree_pool` (`tree.c`), `names` and `reuc` (checkout), and
//!   `index_file_path` (tests). Those stay in the header, in the C's order;
//!   the rest (the path map, deferred deletions, readers count, unknown
//!   extensions, version) moved out of the header into Rust-native state
//!   after the prefix.
//! - [`GitIndexEntry`], [`GitIndexTime`], [`GitIndexReucEntry`],
//!   [`GitIndexNameEntry`] and [`GitIndexOptions`] are public-header types,
//!   mirrored (kind 3). An index entry is allocated as `struct
//!   entry_internal` ([`EntryHead`] then the path's bytes), which C reads
//!   through `git_index_entry_srch` (its `pathlen`); a REUC entry likewise
//!   carries its path.
//! - [`GitIndexWriter`] (`git_indexwriter`) is mirrored: C embeds it on its
//!   stack (merge, revert, cherry-pick, rebase, apply).
//! - [`GitTreeCache`](crate::tree_cache::GitTreeCache) is `tree_cache`'s.
//!
//! # References
//!
//! C can reach an index between any two of its calls, and during the C
//! calls the index makes: path validation (configuration), object checks
//! (the odb's backends), blob creation (filters and attributes, which read
//! the repository's index), diffs (which iterate over it), submodule
//! lookups, tree walks, and the callers' callbacks, which may call the
//! index API again. So the port forms a `&mut GitIndex` only in code that
//! makes no foreign call ([`GitIndex`]'s methods), through
//! [`IndexPtr::get`], and drops it before every foreign call; the
//! operations that call out (in [`ops`]) re-derive it afterwards. Entries
//! are reached through raw pointers, with short-lived references in code
//! that makes no foreign call.

pub mod api;
pub mod ffi;
pub mod foreign;
pub(crate) mod map;
pub mod ondisk;
pub mod ops;

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::{self, NonNull};

use crate::oid::{Oid, OidType, git_oid_t};
use crate::tree_cache::{self, GitTreeCache};
use crate::util::alloc;
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::filebuf::GitFilebuf;
use crate::util::futils::Filestamp;
use crate::util::pool::GitPool;
use crate::util::thread::{Atomic32, GitRefcount};
use crate::util::tsort::tsort;
use crate::util::util::bsearch;
use crate::util::vector::{GitVector, VectorCmp};
use crate::{git_assert, git_error_set};

use map::EntryMap;

pub use api::{
    ConflictEntries, Index, IndexAddOption, IndexCapability, IndexConflicts, IndexEntries,
    IndexEntry, IndexTime, Stage,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// `GIT_INDEX_ENTRY_NAMEMASK`.
pub const GIT_INDEX_ENTRY_NAMEMASK: u16 = 0x0fff;
/// `GIT_INDEX_ENTRY_STAGEMASK`.
pub const GIT_INDEX_ENTRY_STAGEMASK: u16 = 0x3000;
/// `GIT_INDEX_ENTRY_STAGESHIFT`.
pub const GIT_INDEX_ENTRY_STAGESHIFT: u16 = 12;
/// `GIT_INDEX_ENTRY_EXTENDED`.
pub const GIT_INDEX_ENTRY_EXTENDED: u16 = 0x4000;
/// `GIT_INDEX_ENTRY_VALID`.
pub const GIT_INDEX_ENTRY_VALID: u16 = 0x8000;
/// `GIT_INDEX_ENTRY_INTENT_TO_ADD`.
pub const GIT_INDEX_ENTRY_INTENT_TO_ADD: u16 = 1 << 13;
/// `GIT_INDEX_ENTRY_SKIP_WORKTREE`.
pub const GIT_INDEX_ENTRY_SKIP_WORKTREE: u16 = 1 << 14;
/// `GIT_INDEX_ENTRY_EXTENDED_FLAGS`.
pub const GIT_INDEX_ENTRY_EXTENDED_FLAGS: u16 =
    GIT_INDEX_ENTRY_INTENT_TO_ADD | GIT_INDEX_ENTRY_SKIP_WORKTREE;
/// `GIT_INDEX_ENTRY_UPTODATE`.
pub const GIT_INDEX_ENTRY_UPTODATE: u16 = 1 << 2;

/// `GIT_INDEX_CAPABILITY_IGNORE_CASE`.
pub const GIT_INDEX_CAPABILITY_IGNORE_CASE: c_int = 1;
/// `GIT_INDEX_CAPABILITY_NO_FILEMODE`.
pub const GIT_INDEX_CAPABILITY_NO_FILEMODE: c_int = 2;
/// `GIT_INDEX_CAPABILITY_NO_SYMLINKS`.
pub const GIT_INDEX_CAPABILITY_NO_SYMLINKS: c_int = 4;
/// `GIT_INDEX_CAPABILITY_FROM_OWNER`.
pub const GIT_INDEX_CAPABILITY_FROM_OWNER: c_int = -1;

/// `GIT_INDEX_ADD_FORCE`.
pub const GIT_INDEX_ADD_FORCE: c_uint = 1 << 0;
/// `GIT_INDEX_ADD_DISABLE_PATHSPEC_MATCH`.
pub const GIT_INDEX_ADD_DISABLE_PATHSPEC_MATCH: c_uint = 1 << 1;
/// `GIT_INDEX_ADD_CHECK_PATHSPEC`.
pub const GIT_INDEX_ADD_CHECK_PATHSPEC: c_uint = 1 << 2;

/// `GIT_INDEX_STAGE_ANY`.
pub const GIT_INDEX_STAGE_ANY: c_int = -1;
/// `GIT_INDEX_STAGE_ANCESTOR`.
pub const GIT_INDEX_STAGE_ANCESTOR: c_int = 1;

/// `GIT_INDEX_OPTIONS_VERSION`.
pub const GIT_INDEX_OPTIONS_VERSION: c_uint = 1;

/// `INDEX_VERSION_NUMBER_DEFAULT`.
pub const INDEX_VERSION_NUMBER_DEFAULT: c_uint = 2;
/// `INDEX_VERSION_NUMBER_LB`.
pub const INDEX_VERSION_NUMBER_LB: c_uint = 2;
/// `INDEX_VERSION_NUMBER_EXT`.
pub const INDEX_VERSION_NUMBER_EXT: c_uint = 3;
/// `INDEX_VERSION_NUMBER_COMP`.
pub const INDEX_VERSION_NUMBER_COMP: c_uint = 4;
/// `INDEX_VERSION_NUMBER_UB`.
pub const INDEX_VERSION_NUMBER_UB: c_uint = 4;

/// `GIT_INDEX_FILE_MODE`.
pub const GIT_INDEX_FILE_MODE: libc::mode_t = 0o666;

/// `S_IFMT`, `S_IFREG`, `S_IFDIR`, `S_IFLNK`, `S_IFGITLINK`.
const S_IFMT: u32 = 0o170000;
const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;
const S_IFLNK: u32 = 0o120000;
const S_IFGITLINK: u32 = 0o160000;

/// `GIT_FILEMODE_*`.
pub const GIT_FILEMODE_TREE: u32 = 0o040000;
/// `GIT_FILEMODE_BLOB`.
pub const GIT_FILEMODE_BLOB: u32 = 0o100644;
/// `GIT_FILEMODE_BLOB_EXECUTABLE`.
pub const GIT_FILEMODE_BLOB_EXECUTABLE: u32 = 0o100755;
/// `GIT_FILEMODE_LINK`.
pub const GIT_FILEMODE_LINK: u32 = 0o120000;
/// `GIT_FILEMODE_COMMIT`.
pub const GIT_FILEMODE_COMMIT: u32 = 0o160000;

#[inline]
fn s_isreg(m: u32) -> bool {
    m & S_IFMT == S_IFREG
}
#[inline]
fn s_isdir(m: u32) -> bool {
    m & S_IFMT == S_IFDIR
}
#[inline]
fn s_islnk(m: u32) -> bool {
    m & S_IFMT == S_IFLNK
}
#[inline]
fn s_isgitlink(m: u32) -> bool {
    m & S_IFMT == S_IFGITLINK
}

/// `git_index__enforce_unsaved_safety`: C (`settings.c`) writes it as a
/// `bool`; an `AtomicBool` has the same layout.
#[unsafe(no_mangle)]
pub static git_index__enforce_unsaved_safety: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

// ---------------------------------------------------------------------------
// The public-header types
// ---------------------------------------------------------------------------

/// `git_index_time`, mirrored.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct GitIndexTime {
    /// `seconds`.
    pub seconds: i32,
    /// `nanoseconds`.
    pub nanoseconds: u32,
}

/// `git_index_entry`, mirrored.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GitIndexEntry {
    /// `ctime`.
    pub ctime: GitIndexTime,
    /// `mtime`.
    pub mtime: GitIndexTime,
    /// `dev`.
    pub dev: u32,
    /// `ino`.
    pub ino: u32,
    /// `mode`.
    pub mode: u32,
    /// `uid`.
    pub uid: u32,
    /// `gid`.
    pub gid: u32,
    /// `file_size`.
    pub file_size: u32,
    /// `id`.
    pub id: Oid,
    /// `flags`.
    pub flags: u16,
    /// `flags_extended`.
    pub flags_extended: u16,
    /// `path`: NUL-terminated.
    pub path: *const c_char,
}

impl GitIndexEntry {
    /// An all-zero entry (`{{ 0 }}`).
    pub const fn zeroed() -> GitIndexEntry {
        GitIndexEntry {
            ctime: GitIndexTime {
                seconds: 0,
                nanoseconds: 0,
            },
            mtime: GitIndexTime {
                seconds: 0,
                nanoseconds: 0,
            },
            dev: 0,
            ino: 0,
            mode: 0,
            uid: 0,
            gid: 0,
            file_size: 0,
            id: Oid::from_raw_parts(0, [0; crate::oid::GIT_OID_MAX_SIZE]),
            flags: 0,
            flags_extended: 0,
            path: ptr::null(),
        }
    }

    /// `GIT_INDEX_ENTRY_STAGE`.
    #[inline]
    pub fn stage(&self) -> c_int {
        c_int::from((self.flags & GIT_INDEX_ENTRY_STAGEMASK) >> GIT_INDEX_ENTRY_STAGESHIFT)
    }

    /// `GIT_INDEX_ENTRY_STAGE_SET`.
    #[inline]
    pub fn set_stage(&mut self, stage: c_int) {
        self.flags = (self.flags & !GIT_INDEX_ENTRY_STAGEMASK)
            | ((((stage & 0x03) as u16) << GIT_INDEX_ENTRY_STAGESHIFT) & GIT_INDEX_ENTRY_STAGEMASK);
    }
}

/// `git_index_reuc_entry`, mirrored.
#[repr(C)]
#[derive(Debug)]
pub struct GitIndexReucEntry {
    /// `mode`.
    pub mode: [u32; 3],
    /// `oid`.
    pub oid: [Oid; 3],
    /// `path`: NUL-terminated, after the entry (`reuc_entry_internal`).
    pub path: *mut c_char,
}

/// `struct reuc_entry_internal` without its flexible path.
#[repr(C)]
struct ReucHead {
    entry: GitIndexReucEntry,
    pathlen: usize,
}

/// `git_index_name_entry`, mirrored: three `git__strdup`'d strings (or
/// NULL).
#[repr(C)]
#[derive(Debug)]
pub struct GitIndexNameEntry {
    /// `ancestor`.
    pub ancestor: *mut c_char,
    /// `ours`.
    pub ours: *mut c_char,
    /// `theirs`.
    pub theirs: *mut c_char,
}

/// `git_index_options`, mirrored.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GitIndexOptions {
    /// `version`.
    pub version: c_uint,
    /// `oid_type`.
    pub oid_type: git_oid_t,
}

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

/// `struct entry_internal` without its flexible path: the entry, then the
/// length of the path that follows it. `entry.path` points to that path.
#[repr(C)]
#[derive(Debug)]
pub struct EntryHead {
    /// The public entry.
    pub entry: GitIndexEntry,
    /// `pathlen`.
    pub pathlen: usize,
}

/// The path of an entry head: `pathlen` bytes at `entry.path`.
///
/// # Safety
///
/// `e` points to a live [`EntryHead`] whose `entry.path` is readable for
/// `pathlen` bytes, neither written during `'a`.
#[inline(always)]
pub(crate) unsafe fn entry_path_raw<'a>(e: *const EntryHead) -> &'a [u8] {
    // SAFETY: the caller's contract: a live head.
    let h = unsafe { &*e };
    if h.pathlen == 0 {
        return &[];
    }
    // SAFETY: the caller's contract: `pathlen` readable bytes at the path.
    unsafe { core::slice::from_raw_parts(h.entry.path.cast::<u8>(), h.pathlen) }
}

/// The stage of an entry head.
///
/// # Safety
///
/// `e` points to a live [`EntryHead`].
#[inline(always)]
pub(crate) unsafe fn entry_stage_raw(e: *const EntryHead) -> u32 {
    // SAFETY: the caller's contract.
    u32::from(
        (unsafe { (*e).entry.flags } & GIT_INDEX_ENTRY_STAGEMASK) >> GIT_INDEX_ENTRY_STAGESHIFT,
    )
}

/// A pointer to an entry this module allocated (`entry_internal`), which
/// the index (or a caller of [`entry_alloc`]) owns.
pub(crate) type EntryPtr = NonNull<EntryHead>;

/// An index entry's path (its own storage).
///
/// # Safety
///
/// `e` is a live entry made by [`entry_alloc`], not written during `'a`.
#[inline(always)]
pub(crate) unsafe fn epath<'a>(e: EntryPtr) -> &'a [u8] {
    // SAFETY: an `entry_alloc` entry's `entry.path` points to its own
    // `pathlen` bytes.
    unsafe { entry_path_raw(e.as_ptr()) }
}

/// An index entry's path and stage.
///
/// # Safety
///
/// As [`epath`].
#[inline(always)]
pub(crate) unsafe fn eps<'a>(e: EntryPtr) -> (&'a [u8], c_int) {
    // SAFETY: the caller's contract.
    let p = unsafe { epath(e) };
    // SAFETY: as above.
    (p, unsafe { estage(e) })
}

/// An index entry's stage.
///
/// # Safety
///
/// `e` is a live entry.
#[inline(always)]
pub(crate) unsafe fn estage(e: EntryPtr) -> c_int {
    // SAFETY: the caller's contract.
    unsafe { entry_stage_raw(e.as_ptr()) as c_int }
}

/// The allocation of `index_entry_create` (without the path check): a
/// zeroed entry with its own copy of `path`, from `git__allocator`.
pub(crate) fn entry_alloc(path: &[u8]) -> Result<EntryPtr> {
    let alloclen = crate::util::integer::alloc_add(size_of::<EntryHead>(), path.len())
        .and_then(|n| crate::util::integer::alloc_add(n, 1))?;
    let p = alloc::calloc(1, alloclen).cast::<EntryHead>();
    let Some(p) = NonNull::new(p) else {
        return Err(ErrorCode::GENERIC);
    };
    // `p` is a fresh, zeroed block of `alloclen` bytes, 8-aligned (the
    // allocator's), room for the head, the path and its NUL; a zeroed
    // `EntryHead` is valid (all integers, a null pointer).
    let pathp = p.as_ptr().cast::<u8>().wrapping_add(size_of::<EntryHead>());
    // SAFETY: the path's room in the block, disjoint from `path`.
    unsafe { ptr::copy_nonoverlapping(path.as_ptr(), pathp, path.len()) };
    // SAFETY: the fresh, valid head, not otherwise referenced.
    let h = unsafe { &mut *p.as_ptr() };
    h.pathlen = path.len();
    h.entry.path = pathp.cast();
    Ok(p)
}

/// An entry's head, for writing.
///
/// # Safety
///
/// `e` is a live entry that nothing else borrows during `'a`.
#[inline(always)]
pub(crate) unsafe fn ehead<'a>(e: EntryPtr) -> &'a mut EntryHead {
    // SAFETY: the caller's contract.
    unsafe { &mut *e.as_ptr() }
}

/// Invalidate the tree cache along `e`'s path.
///
/// # Safety
///
/// `tree` is a tree (`tree_cache`'s contract) and `e` a live entry.
pub(crate) unsafe fn invalidate_entry(tree: *mut GitTreeCache, e: EntryPtr) {
    // SAFETY: the caller's contract.
    let p = unsafe { epath(e) };
    // SAFETY: as above.
    unsafe { tree_cache::invalidate_path(tree, p) };
}

/// `index_entry_free`: zero the id and free.
///
/// # Safety
///
/// `e` is an entry made by [`entry_alloc`] that nothing uses afterwards.
pub(crate) unsafe fn entry_free(e: EntryPtr) {
    // SAFETY: the caller's contract: a live entry, now ours alone.
    unsafe { (*e.as_ptr()).entry.id = Oid::default() };
    // SAFETY: as above; `entry_alloc`'s block, freed once.
    unsafe { alloc::free(e.as_ptr().cast()) };
}

/// `index_entry_cpy`: copy every field of `src` but the path pointer.
///
/// # Safety
///
/// `tgt` is a live entry not otherwise borrowed.
pub(crate) unsafe fn entry_cpy(tgt: EntryPtr, src: &GitIndexEntry) {
    // SAFETY: the caller's contract.
    let t = unsafe { &mut (*tgt.as_ptr()).entry };
    let path = t.path;
    *t = *src;
    t.path = path;
}

/// `index_entry_cpy_nocache`.
///
/// # Safety
///
/// As [`entry_cpy`].
pub(crate) unsafe fn entry_cpy_nocache(tgt: EntryPtr, src: &GitIndexEntry) {
    // SAFETY: the caller's contract.
    unsafe {
        let t = &mut (*tgt.as_ptr()).entry;
        t.id = src.id;
        t.mode = src.mode;
        t.flags = src.flags;
        t.flags_extended = src.flags_extended & GIT_INDEX_ENTRY_EXTENDED_FLAGS;
    }
}

/// `index_entry_adjust_namemask`.
pub(crate) fn adjust_namemask(entry: &mut GitIndexEntry, path_length: usize) {
    entry.flags &= !GIT_INDEX_ENTRY_NAMEMASK;
    if path_length < usize::from(GIT_INDEX_ENTRY_NAMEMASK) {
        entry.flags |= (path_length as u16) & GIT_INDEX_ENTRY_NAMEMASK;
    } else {
        entry.flags |= GIT_INDEX_ENTRY_NAMEMASK;
    }
}

/// `git_index__create_mode`.
pub fn create_mode(mode: u32) -> u32 {
    if s_islnk(mode) {
        return S_IFLNK;
    }
    if s_isdir(mode) || (mode & S_IFMT) == (S_IFLNK | S_IFDIR) {
        return S_IFLNK | S_IFDIR;
    }
    S_IFREG | if mode & 0o100 != 0 { 0o755 } else { 0o644 }
}

/// `git_index_entry__init_from_stat`.
pub fn init_from_stat(entry: &mut GitIndexEntry, st: &libc::stat, trust_mode: bool) {
    entry.ctime.seconds = st.st_ctime as i32;
    entry.mtime.seconds = st.st_mtime as i32;
    entry.mtime.nanoseconds = st.st_mtime_nsec as u32;
    entry.ctime.nanoseconds = st.st_ctime_nsec as u32;
    entry.dev = st.st_rdev as u32;
    entry.ino = st.st_ino as u32;
    entry.mode = if !trust_mode && s_isreg(st.st_mode) {
        create_mode(0o666)
    } else {
        create_mode(st.st_mode)
    };
    entry.uid = st.st_uid;
    entry.gid = st.st_gid;
    entry.file_size = st.st_size as u32;
}

/// `strcmp`/`strcasecmp` (the C locale) of two NUL-free paths.
#[inline(always)]
fn path_cmp(a: &[u8], b: &[u8], icase: bool) -> c_int {
    if !icase {
        match a.cmp(b) {
            core::cmp::Ordering::Less => -1,
            core::cmp::Ordering::Equal => 0,
            core::cmp::Ordering::Greater => 1,
        }
    } else {
        let n = a.len().min(b.len());
        for i in 0..n {
            let (x, y) = (a[i].to_ascii_lowercase(), b[i].to_ascii_lowercase());
            if x != y {
                return c_int::from(x) - c_int::from(y);
            }
        }
        match a.len().cmp(&b.len()) {
            core::cmp::Ordering::Less => -1,
            core::cmp::Ordering::Equal => 0,
            core::cmp::Ordering::Greater => 1,
        }
    }
}

/// `git_index_entry_cmp` / `git_index_entry_icmp` of two index entries.
///
/// # Safety
///
/// Both are live entries made by [`entry_alloc`].
#[inline(always)]
pub(crate) unsafe fn entry_cmp(a: EntryPtr, b: EntryPtr, icase: bool) -> c_int {
    // SAFETY: the caller's contract.
    let (pa, sa) = unsafe { eps(a) };
    // SAFETY: as above.
    let (pb, sb) = unsafe { eps(b) };
    let d = path_cmp(pa, pb, icase);
    if d != 0 { d } else { sa - sb }
}

/// `git_index_entry_srch` / `isrch`: the order of the key (`path`,
/// `stage`) against an index entry. `strncasecmp` over the common length,
/// then the shorter first, then the stage unless it is
/// `GIT_INDEX_STAGE_ANY`.
///
/// # Safety
///
/// `e` is a live entry made by [`entry_alloc`].
#[inline(always)]
pub(crate) unsafe fn entry_srch(path: &[u8], stage: c_int, e: EntryPtr, icase: bool) -> c_int {
    // SAFETY: the caller's contract.
    let (epath, estage) = unsafe { eps(e) };
    let len = path.len().min(epath.len());
    let cmp = if icase {
        let mut c = 0;
        for i in 0..len {
            let (x, y) = (path[i].to_ascii_lowercase(), epath[i].to_ascii_lowercase());
            if x != y {
                c = c_int::from(x) - c_int::from(y);
                break;
            }
        }
        c
    } else {
        match path[..len].cmp(&epath[..len]) {
            core::cmp::Ordering::Less => -1,
            core::cmp::Ordering::Equal => 0,
            core::cmp::Ordering::Greater => 1,
        }
    };
    if cmp != 0 {
        return cmp;
    }
    if path.len() < epath.len() {
        return -1;
    }
    if path.len() > epath.len() {
        return 1;
    }
    if stage != GIT_INDEX_STAGE_ANY {
        return stage - estage;
    }
    0
}

/// A vector element as an entry: the index's vectors hold only non-null
/// entries.
#[inline(always)]
pub(crate) fn as_entry(p: *mut c_void) -> EntryPtr {
    NonNull::new(p.cast::<EntryHead>()).expect("an index entry")
}

/// Sort a vector of index entries as `git_vector_sort` with
/// `git_index_entry_cmp`/`icmp` would (timsort, unless flagged sorted).
pub(crate) fn sort_entries(v: &mut GitVector, icase: bool) {
    if v.is_sorted() || v.cmp_fn().is_none() {
        return;
    }
    if v.len() > 1 {
        // SAFETY: the index's entry vectors hold live entries (the index's
        // invariant), not written during the sort.
        tsort(v.as_mut_slice(), |a, b| unsafe {
            entry_cmp(as_entry(a), as_entry(b), icase)
        });
    }
    v.set_sorted(true);
}

/// `index_find_in_entries`, the vector already sorted: `Ok(pos)` or
/// `Err(insertion pos)` (`GIT_ENOTFOUND`, no error message).
pub(crate) fn find_in_entries(
    v: &GitVector,
    path: &[u8],
    stage: c_int,
    icase: bool,
) -> core::result::Result<usize, usize> {
    // SAFETY: as in `sort_entries`.
    bsearch(v.as_slice(), |&e| unsafe {
        entry_srch(path, stage, as_entry(e), icase)
    })
}

/// The entry comparison function C sees on an index's vector.
pub(crate) fn entry_cmp_fn(icase: bool) -> VectorCmp {
    if icase {
        ffi::git_index_entry_icmp
    } else {
        ffi::git_index_entry_cmp
    }
}

// ---------------------------------------------------------------------------
// REUC and NAME entries
// ---------------------------------------------------------------------------

/// The path of a REUC entry (its own copy).
///
/// # Safety
///
/// `r` is a live REUC entry made by [`reuc_alloc`].
unsafe fn reuc_path<'a>(r: *const GitIndexReucEntry) -> &'a [u8] {
    // SAFETY: a `reuc_alloc` entry starts a `ReucHead`, whose path follows.
    let h = unsafe { &*r.cast::<ReucHead>() };
    // SAFETY: as above: `pathlen` bytes at `path`.
    unsafe { core::slice::from_raw_parts(h.entry.path.cast::<u8>(), h.pathlen) }
}

/// `reuc_entry_alloc`.
fn reuc_alloc(path: &[u8]) -> Option<NonNull<GitIndexReucEntry>> {
    let alloclen = size_of::<ReucHead>()
        .checked_add(path.len())?
        .checked_add(1)?;
    let p = NonNull::new(alloc::calloc(1, alloclen).cast::<ReucHead>())?;
    // a fresh zeroed block with room for the head and the path; a zeroed
    // head is valid
    let pathp = p.as_ptr().cast::<u8>().wrapping_add(size_of::<ReucHead>());
    // SAFETY: the path's room in the block, disjoint from `path`.
    unsafe { ptr::copy_nonoverlapping(path.as_ptr(), pathp, path.len()) };
    // SAFETY: the fresh head, not otherwise referenced.
    let h = unsafe { &mut *p.as_ptr() };
    h.pathlen = path.len();
    h.entry.path = pathp.cast();
    Some(p.cast())
}

/// `index_entry_reuc_free`.
///
/// # Safety
///
/// `r` is NULL or a REUC entry this module allocated, unused afterwards.
unsafe fn reuc_free(r: *mut c_void) {
    // SAFETY: the caller's contract.
    unsafe { alloc::free(r) };
}

/// `reuc_cmp` / `reuc_icmp` / `reuc_srch` of paths.
fn reuc_path_cmp(a: &[u8], b: &[u8], icase: bool) -> c_int {
    path_cmp(a, b, icase)
}

/// `reuc_cmp`/`reuc_icmp` of two REUC entries.
///
/// # Safety
///
/// Both are live REUC entries made by [`reuc_alloc`].
unsafe fn reuc_cmp_raw(a: *const c_void, b: *const c_void, icase: bool) -> c_int {
    // SAFETY: the caller's contract.
    let pa = unsafe { reuc_path(a.cast()) };
    // SAFETY: as above.
    let pb = unsafe { reuc_path(b.cast()) };
    reuc_path_cmp(pa, pb, icase)
}

unsafe extern "C" fn reuc_cmp(a: *const c_void, b: *const c_void) -> c_int {
    // SAFETY: the vector's elements are REUC entries.
    unsafe { reuc_cmp_raw(a, b, false) }
}

unsafe extern "C" fn reuc_icmp(a: *const c_void, b: *const c_void) -> c_int {
    // SAFETY: as above.
    unsafe { reuc_cmp_raw(a, b, true) }
}

/// The bytes of a C string field, or `None` for NULL.
///
/// # Safety
///
/// `p` is NULL or a live NUL-terminated string.
unsafe fn opt_cstr<'a>(p: *const c_char) -> Option<&'a [u8]> {
    // SAFETY: the caller's contract.
    unsafe { crate::util::export::cstr(p) }.map(CStr::to_bytes)
}

/// `conflict_name_cmp`.
///
/// # Safety
///
/// Both are live name entries.
unsafe fn name_cmp(a: *const GitIndexNameEntry, b: *const GitIndexNameEntry) -> c_int {
    // SAFETY: the caller's contract.
    let a = unsafe { &*a };
    // SAFETY: as above.
    let b = unsafe { &*b };
    let f = |p: *const c_char| {
        // SAFETY: the fields are NULL or strings (name entries' invariant).
        unsafe { opt_cstr(p) }
    };
    match (f(a.ancestor), f(b.ancestor)) {
        (Some(_), None) => return 1,
        (None, Some(_)) => return -1,
        (Some(x), Some(y)) => return crate::util::util::strcmp(x, y),
        (None, None) => {}
    }
    match (f(a.ours), f(b.ours)) {
        (Some(x), Some(y)) => crate::util::util::strcmp(x, y),
        _ => 0,
    }
}

unsafe extern "C" fn conflict_name_cmp(a: *const c_void, b: *const c_void) -> c_int {
    // SAFETY: the vector's elements are name entries.
    unsafe { name_cmp(a.cast(), b.cast()) }
}

/// `index_name_entry_free`.
///
/// # Safety
///
/// `ne` is NULL or a name entry this module allocated, unused afterwards.
unsafe fn name_free(ne: *mut GitIndexNameEntry) {
    if ne.is_null() {
        return;
    }
    // SAFETY: the caller's contract: a live name entry, ours alone.
    let n = unsafe { &*ne };
    for s in [n.ancestor, n.ours, n.theirs] {
        // SAFETY: NULL or a `git__allocator` string the entry owns.
        unsafe { alloc::free(s.cast()) };
    }
    // SAFETY: the entry's block, freed once.
    unsafe { alloc::free(ne.cast()) };
}

// ---------------------------------------------------------------------------
// Extensions
// ---------------------------------------------------------------------------

/// An extension the index does not understand, kept to be written back.
#[derive(Clone, Debug)]
pub struct Extension {
    /// The signature.
    pub signature: [u8; 4],
    /// The data.
    pub data: Vec<u8>,
}

/// `strncmp(a, b, 4)` where `a` ends at its first NUL or its end.
fn sig_cmp(a: &[u8], b: &[u8; 4]) -> c_int {
    for (i, &y) in b.iter().enumerate() {
        let x = a.get(i).copied().unwrap_or(0);
        if x != y {
            return c_int::from(x) - c_int::from(y);
        }
        if x == 0 {
            return 0;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// The index
// ---------------------------------------------------------------------------

/// Bits of `GitIndex::bits` (C bitfields, the first declared lowest).
mod bit {
    use core::ffi::c_uint;
    pub const ON_DISK: c_uint = 1 << 0;
    pub const IGNORE_CASE: c_uint = 1 << 1;
    pub const DISTRUST_FILEMODE: c_uint = 1 << 2;
    pub const NO_SYMLINKS: c_uint = 1 << 3;
    pub const DIRTY: c_uint = 1 << 4;
}

/// `git_index` (see the [module documentation](self)): the C-visible
/// prefix, then the Rust-private state.
///
/// Invariant: `entries`, `deleted`, and the map hold live entries made by
/// [`entry_alloc`] that the index owns (each in `entries` or `deleted`,
/// once); the map's keys are entries of `entries`; `reuc` holds REUC
/// entries and `names` name entries this module allocated; `tree` is NULL
/// or a tree in `tree_pool`; `oid_type` is SHA-1 or SHA-256.
#[repr(C)]
pub struct GitIndex {
    /// `rc`.
    pub rc: GitRefcount,
    /// `index_file_path`: NULL (in memory) or a `git__allocator` string.
    pub index_file_path: *mut c_char,
    /// `stamp`.
    pub stamp: Filestamp,
    /// `checksum`.
    pub checksum: [u8; crate::util::hash::MAX_SIZE],
    /// `entries`.
    pub entries: GitVector,
    /// `oid_type`.
    pub oid_type: git_oid_t,
    /// `on_disk`, `ignore_case`, `distrust_filemode`, `no_symlinks`,
    /// `dirty` (bits 0 to 4).
    pub bits: c_uint,
    /// `tree`.
    pub tree: *mut GitTreeCache,
    /// `tree_pool`.
    pub tree_pool: GitPool,
    /// `names`.
    pub names: GitVector,
    /// `reuc`.
    pub reuc: GitVector,

    // --- not in the header ---
    /// `entries_map`.
    pub(crate) entries_map: EntryMap,
    /// `deleted`: entries removed while readers (snapshots) were active.
    pub(crate) deleted: Vec<EntryPtr>,
    /// `readers`: the active snapshots.
    pub(crate) readers: Atomic32,
    /// `extensions`, sorted by signature.
    pub(crate) extensions: Vec<Extension>,
    /// `version`.
    pub(crate) version: c_uint,
}

impl core::fmt::Debug for GitIndex {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GitIndex")
            .field("entries", &self.entries.len())
            .field("bits", &self.bits)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

macro_rules! bit_accessors {
    ($($get:ident, $set:ident, $b:expr;)*) => {
        $(
            #[doc = concat!("The `", stringify!($get), "` bit.")]
            #[inline]
            pub fn $get(&self) -> bool {
                self.bits & $b != 0
            }
            #[doc = concat!("Set the `", stringify!($get), "` bit.")]
            #[inline]
            pub fn $set(&mut self, v: bool) {
                if v { self.bits |= $b } else { self.bits &= !$b }
            }
        )*
    };
}

impl GitIndex {
    bit_accessors! {
        on_disk, set_on_disk, bit::ON_DISK;
        ignore_case, set_ignore_case_bit, bit::IGNORE_CASE;
        distrust_filemode, set_distrust_filemode, bit::DISTRUST_FILEMODE;
        no_symlinks, set_no_symlinks, bit::NO_SYMLINKS;
        dirty, set_dirty, bit::DIRTY;
    }

    /// The first part of `git_index_open_ext`: an empty index, refcount 0.
    pub(crate) fn new(oid_type: OidType) -> Result<Box<GitIndex>> {
        let entries = GitVector::with_capacity(32, Some(ffi::git_index_entry_cmp))?;
        let names = GitVector::with_capacity(8, Some(conflict_name_cmp))?;
        let reuc = GitVector::with_capacity(8, Some(reuc_cmp))?;
        let mut tree_pool = GitPool::new();
        tree_pool.init(1)?;
        let mut deleted = Vec::new();
        deleted.try_reserve(8).map_err(|_| errors::oom())?;
        let mut extensions = Vec::new();
        extensions.try_reserve(8).map_err(|_| errors::oom())?;
        Ok(Box::new(GitIndex {
            rc: GitRefcount::new(0),
            index_file_path: ptr::null_mut(),
            stamp: Filestamp::default(),
            checksum: [0; crate::util::hash::MAX_SIZE],
            entries,
            oid_type: oid_type.raw(),
            bits: 0,
            tree: ptr::null_mut(),
            tree_pool,
            names,
            reuc,
            entries_map: EntryMap::default(),
            deleted,
            readers: Atomic32::new(0),
            extensions,
            version: INDEX_VERSION_NUMBER_DEFAULT,
        }))
    }

    /// The index's id type.
    #[inline]
    pub fn oid_kind(&self) -> OidType {
        OidType::from_raw(self.oid_type).unwrap_or(OidType::Sha1)
    }

    /// `INDEX_OWNER`: the repository, or NULL.
    #[inline]
    pub fn owner(&self) -> *mut c_void {
        self.rc.owner()
    }

    /// The index's path, if not in memory.
    pub fn file_path(&self) -> Option<&CStr> {
        // SAFETY: the invariant: NULL or a string the index owns.
        unsafe { crate::util::export::cstr(self.index_file_path) }
    }

    /// `git_vector_sort(&index->entries)`.
    #[inline]
    pub(crate) fn sort(&mut self) {
        let icase = self.ignore_case();
        sort_entries(&mut self.entries, icase);
    }

    /// The entry at `pos`.
    #[inline]
    pub(crate) fn entry_at(&self, pos: usize) -> Option<EntryPtr> {
        self.entries.get(pos).map(as_entry)
    }

    /// `index_find`: sort, then search for (`path`, `stage`).
    pub(crate) fn find(&mut self, path: &[u8], stage: c_int) -> core::result::Result<usize, usize> {
        self.sort();
        find_in_entries(&self.entries, path, stage, self.ignore_case())
    }

    /// `index_free_deleted`.
    pub(crate) fn free_deleted(&mut self) {
        if self.readers.get() > 0 || self.deleted.is_empty() {
            return;
        }
        for e in self.deleted.drain(..) {
            // SAFETY: deferred entries are owned by the index, and no
            // snapshot is left that could read them.
            unsafe { entry_free(e) };
        }
    }

    /// `index_remove_entry`.
    pub(crate) fn remove_entry(&mut self, pos: usize) -> Result<()> {
        let entry = self.entry_at(pos);
        if let Some(e) = entry {
            // SAFETY: `e` is a live entry of the index.
            let p = unsafe { epath(e) };
            // SAFETY: the tree invariant.
            unsafe { tree_cache::invalidate_path(self.tree, p) };
            // SAFETY: `e` is live.
            unsafe { self.entries_map.remove(e) };
        }
        self.entries.remove(pos)?;
        let Some(e) = entry else {
            return Ok(());
        };
        let mut r = Ok(());
        if self.readers.get() > 0 {
            if self.deleted.try_reserve(1).is_ok() {
                self.deleted.push(e);
            } else {
                // C's `git_vector_insert` failed and the entry leaked;
                // keeping it alive is the closest defined behaviour.
                r = Err(errors::oom());
            }
        } else {
            // SAFETY: removed from the vector and the map; the index owned
            // it.
            unsafe { entry_free(e) };
        }
        self.set_dirty(true);
        r
    }

    /// `git_index_clear`.
    pub(crate) fn clear(&mut self) -> Result<()> {
        self.set_dirty(true);
        self.tree = ptr::null_mut();
        self.tree_pool.clear();
        self.entries_map.clear();
        while !self.entries.is_empty() {
            let n = self.entries.len();
            self.remove_entry(n - 1)?;
        }
        self.free_deleted();
        self.extensions.clear();
        self.name_clear();
        self.reuc_clear();
        crate::util::futils::filestamp_set(&mut self.stamp, None);
        Ok(())
    }

    /// `git_index__set_ignore_case`.
    pub(crate) fn set_ignore_case(&mut self, icase: bool) {
        self.set_ignore_case_bit(icase);
        self.entries_map.ignore_case = icase;
        self.entries.set_cmp(Some(entry_cmp_fn(icase)));
        self.sort();
        self.reuc
            .set_cmp(Some(if icase { reuc_icmp } else { reuc_cmp }));
        self.sort_reuc();
    }

    /// `git_vector_sort(&index->reuc)`.
    pub(crate) fn sort_reuc(&mut self) {
        if self.reuc.is_sorted() || self.reuc.cmp_fn().is_none() {
            return;
        }
        let icase = self.reuc_icase();
        if self.reuc.len() > 1 {
            // SAFETY: the vector holds REUC entries (the invariant).
            tsort(self.reuc.as_mut_slice(), |a, b| unsafe {
                reuc_cmp_raw(a, b, icase)
            });
        }
        self.reuc.set_sorted(true);
    }

    /// Whether the REUC vector's comparison is the case-insensitive one.
    fn reuc_icase(&self) -> bool {
        self.reuc.cmp_fn().map(|f| f as usize) == Some(reuc_icmp as VectorCmp as usize)
    }

    /// `git_vector_sort(&index->names)`.
    pub(crate) fn sort_names(&mut self) {
        if self.names.is_sorted() || self.names.cmp_fn().is_none() {
            return;
        }
        if self.names.len() > 1 {
            // SAFETY: the vector holds name entries (the invariant).
            tsort(self.names.as_mut_slice(), |a, b| unsafe {
                name_cmp(a.cast(), b.cast())
            });
        }
        self.names.set_sorted(true);
    }

    /// `git_index_get_bypath`'s lookup (no error set).
    pub(crate) fn get_bypath(&self, path: &[u8], stage: c_int) -> Option<EntryPtr> {
        let mut key = EntryHead {
            entry: GitIndexEntry::zeroed(),
            pathlen: path.len(),
        };
        key.entry.path = path.as_ptr().cast();
        key.entry.set_stage(stage);
        // SAFETY: `key` is a head whose path is `path`, live for the call.
        unsafe { self.entries_map.get(NonNull::from(&mut key)) }
    }

    /// `git_index_find`: the first entry (of any stage) at `path`.
    pub(crate) fn find_path(&mut self, path: &[u8]) -> Option<usize> {
        let icase = self.ignore_case();
        self.sort();
        // `entries_search_path`: strcmp/strcasecmp of the path.
        // SAFETY: as in `sort_entries`.
        let found = bsearch(self.entries.as_slice(), |&e| unsafe {
            path_cmp(path, epath(as_entry(e)), icase)
        });
        let mut pos = found.ok()?;
        while pos > 0 {
            let prev = self.entry_at(pos - 1)?;
            // SAFETY: a live entry of the index.
            if path_cmp(unsafe { epath(prev) }, path, icase) != 0 {
                break;
            }
            pos -= 1;
        }
        Some(pos)
    }

    /// `index_existing_and_best`: the entry at (path, stage) and its
    /// position (or the insertion position), and the best entry to take a
    /// mode from (the "ours" side of a conflict for a stage-0 entry).
    pub(crate) fn existing_and_best(
        &mut self,
        path: &[u8],
        stage: c_int,
    ) -> (Option<EntryPtr>, usize, Option<EntryPtr>) {
        match self.find(path, stage) {
            Ok(pos) => {
                let e = self.entry_at(pos);
                (e, pos, e)
            }
            Err(pos) => {
                let mut best = None;
                if stage == 0 {
                    let icase = self.ignore_case();
                    let mut p = pos;
                    while let Some(e) = self.entry_at(p) {
                        // SAFETY: a live entry of the index.
                        let (ep, es) = unsafe { eps(e) };
                        // `git__strcasecmp` / `git__strcmp`.
                        if path_cmp(path, ep, icase) != 0 {
                            break;
                        }
                        best = Some(e);
                        if es != GIT_INDEX_STAGE_ANCESTOR {
                            break;
                        }
                        p += 1;
                    }
                }
                (None, pos, best)
            }
        }
    }

    /// `index_merge_mode`.
    pub(crate) fn merge_mode(&self, existing: Option<EntryPtr>, mode: u32) -> u32 {
        // SAFETY: `existing` is a live entry of the index.
        let emode = existing.map(|e| unsafe { (*e.as_ptr()).entry.mode });
        if self.no_symlinks() && s_isreg(mode) && emode.is_some_and(s_islnk) {
            return emode.unwrap_or(0);
        }
        if self.distrust_filemode() && s_isreg(mode) {
            return match emode {
                Some(m) if s_isreg(m) => m,
                _ => create_mode(0o666),
            };
        }
        create_mode(mode)
    }

    /// `canonicalize_directory_path`: on a case-insensitive index, give
    /// `entry`'s directories the case they already have in the index.
    ///
    /// `entry` is a live entry that the index does not hold yet.
    pub(crate) fn canonicalize_directory_path(&self, entry: EntryPtr, existing: Option<EntryPtr>) {
        if !self.ignore_case() {
            return;
        }
        // SAFETY: `entry` is live and not in the index (the caller), so
        // this is its only access; `existing` is a live entry of the index,
        // distinct from `entry`.
        let head = unsafe { &*entry.as_ptr() };
        let (path, len) = (head.entry.path.cast_mut().cast::<u8>(), head.pathlen);
        if let Some(ex) = existing {
            // SAFETY: as above.
            let ep = unsafe { epath(ex) };
            // SAFETY: as above; C copies `strlen(existing->path)` bytes,
            // the same length (the paths are equal but for case).
            unsafe { ptr::copy_nonoverlapping(ep.as_ptr(), path, ep.len().min(len)) };
            return;
        }
        // SAFETY: as above.
        let search_full = unsafe { epath(entry) };
        if memchr::memchr(b'/', search_full).is_none() {
            return;
        }
        let search: Vec<u8> = search_full.to_vec();
        let mut best: Option<(EntryPtr, usize)> = None;
        let mut end = search.len();
        while best.is_none() {
            let Some(sep) = memchr::memrchr(b'/', &search[..end]) else {
                break;
            };
            let prefix = &search[..=sep];
            // `git_vector_bsearch2` with `entries_search_path`
            // (`strcasecmp`, the index being case-insensitive).
            // SAFETY: as in `sort_entries`.
            let pos = match bsearch(self.entries.as_slice(), |&e| unsafe {
                path_cmp(prefix, epath(as_entry(e)), true)
            }) {
                Ok(p) | Err(p) => p,
            };
            let mut p = pos;
            while let Some(m) = self.entry_at(p) {
                // SAFETY: a live entry of the index.
                let (mp, ms) = unsafe { eps(m) };
                if ms != 0 {
                    // conflicts do not contribute to canonical paths
                } else if mp.starts_with(prefix) {
                    best = Some((m, prefix.len()));
                    break;
                } else if mp.len() >= prefix.len()
                    && mp[..prefix.len()].eq_ignore_ascii_case(prefix)
                {
                    if best.is_none() {
                        best = Some((m, prefix.len()));
                    }
                } else {
                    break;
                }
                p += 1;
            }
            end = sep;
        }
        if let Some((b, blen)) = best {
            // SAFETY: as above.
            let bp = unsafe { epath(b) };
            // SAFETY: as above; `blen` is within both paths.
            unsafe { ptr::copy_nonoverlapping(bp.as_ptr(), path, blen.min(len)) };
        }
    }

    /// `has_file_name`: remove (or refuse) the entries under `entry`'s
    /// path, as a directory, at its stage.
    fn has_file_name(
        &mut self,
        entry: EntryPtr,
        mut pos: usize,
        ok_to_replace: bool,
    ) -> core::result::Result<(), ()> {
        // SAFETY: `entry` is live and not in the index.
        let (name, stage) = unsafe { eps(entry) };
        let name = name.to_vec();
        let len = name.len();
        while let Some(p) = self.entry_at(pos) {
            pos += 1;
            // SAFETY: a live entry of the index.
            let (pp, ps) = unsafe { eps(p) };
            if len >= pp.len() {
                break;
            }
            if pp[..len] != name[..] {
                break;
            }
            if ps != stage {
                continue;
            }
            if pp[len] != b'/' {
                continue;
            }
            if !ok_to_replace {
                return Err(());
            }
            pos -= 1;
            if self.remove_entry(pos).is_err() {
                break;
            }
        }
        Ok(())
    }

    /// `has_dir_name`: remove (or refuse) the entries at the paths of
    /// `entry`'s parent directories, at its stage.
    fn has_dir_name(
        &mut self,
        entry: EntryPtr,
        ok_to_replace: bool,
    ) -> core::result::Result<(), ()> {
        // SAFETY: `entry` is live and not in the index.
        let (name, stage) = unsafe { eps(entry) };
        let name = name.to_vec();
        let mut slash = name.len();
        loop {
            // find the previous '/' (C stops at the start of the path)
            loop {
                if slash <= 1 {
                    return Ok(());
                }
                slash -= 1;
                if name[slash] == b'/' {
                    break;
                }
            }
            let len = slash;
            match self.find(&name[..len], stage) {
                Ok(pos) => {
                    if !ok_to_replace {
                        return Err(());
                    }
                    if self.remove_entry(pos).is_err() {
                        break;
                    }
                    continue;
                }
                Err(mut pos) => {
                    while let Some(p) = self.entry_at(pos) {
                        // SAFETY: a live entry of the index.
                        let (pp, ps) = unsafe { eps(p) };
                        if pp.len() <= len || pp[len] != b'/' || pp[..len] != name[..len] {
                            break;
                        }
                        if ps == stage {
                            return Ok(());
                        }
                        pos += 1;
                    }
                }
            }
        }
        Ok(())
    }

    /// `check_file_directory_collision`.
    pub(crate) fn check_file_directory_collision(
        &mut self,
        entry: EntryPtr,
        pos: usize,
        ok_to_replace: bool,
    ) -> Result<()> {
        if self.has_file_name(entry, pos, ok_to_replace).is_err()
            || self.has_dir_name(entry, ok_to_replace).is_err()
        {
            // SAFETY: `entry` is live.
            let path = unsafe { epath(entry) };
            return Err(git_error_set!(
                ErrorClass::INDEX,
                "'{}' appears as both a file and a directory",
                Bytes(path)
            ));
        }
        Ok(())
    }

    /// The end of `index_insert`, from the collision check on: insert or
    /// replace. Takes `entry` (freed on failure and when it replaces an
    /// existing entry); the entry now in the index.
    pub(crate) fn insert_finish(
        &mut self,
        entry: EntryPtr,
        replace: bool,
        trust_path: bool,
    ) -> Result<EntryPtr> {
        // SAFETY: `entry` is live and not in the index.
        let (path, stage) = unsafe { eps(entry) };
        let path = path.to_vec();
        // the position (and existing entry) again: C kept them across the
        // object check, which may have run code that changed the index.
        let (existing, position, _) = self.existing_and_best(&path, stage);
        // (the collision check never removes `existing`: it removes only
        // entries at other paths)
        if let Err(e) = self.check_file_directory_collision(entry, position, replace) {
            // SAFETY: the entry is ours, not in the index.
            unsafe { entry_free(entry) };
            return Err(e);
        }
        let result = if let Some(ex) = existing {
            if replace {
                // SAFETY: `ex` is a live entry of the index and `entry` a
                // distinct live one (for this whole block).
                let src = unsafe { (*entry.as_ptr()).entry };
                // SAFETY: as above.
                unsafe { entry_cpy(ex, &src) };
                if trust_path {
                    // SAFETY: as above.
                    let ep = unsafe { epath(entry) };
                    // SAFETY: as above.
                    let xh = unsafe { &*ex.as_ptr() };
                    let (xp, xlen) = (xh.entry.path.cast_mut().cast::<u8>(), xh.pathlen);
                    // SAFETY: as above: within `ex`'s own path.
                    unsafe { ptr::copy_nonoverlapping(ep.as_ptr(), xp, ep.len().min(xlen)) };
                }
            }
            // SAFETY: the entry is ours, not in the index.
            unsafe { entry_free(entry) };
            ex
        } else {
            let icase = self.ignore_case();
            // `git_vector_insert_sorted` with `index_no_dups`
            let r = self.entries.insert_sorted_by(
                entry.as_ptr().cast(),
                // SAFETY: the vector's elements and `entry` are live
                // entries.
                |a, b| unsafe { entry_cmp(as_entry(a), as_entry(b), icase) },
                Some(&mut |_old: &mut *mut c_void, new: *mut c_void| {
                    let e = as_entry(new);
                    // SAFETY: `new` is `entry`, live.
                    let (p, s) = unsafe { eps(e) };
                    let _ = errors::set_fmt(
                        ErrorClass::INDEX,
                        format_args!("'{}' appears multiple times at stage {}", Bytes(p), s),
                    );
                    ErrorCode::EXISTS.raw()
                }),
            );
            if let Err(e) = r {
                // SAFETY: not inserted; ours.
                unsafe { entry_free(entry) };
                return Err(e);
            }
            // SAFETY: `entry` is live and now in `entries`.
            if let Err(e) = unsafe { self.entries_map.put(entry) } {
                // C freed the entry while the vector held it; take it out
                // first (see `rust/DEFECTS.md`).
                if let Ok(p) = self.find(&path, stage) {
                    let _ = self.entries.remove(p);
                }
                // SAFETY: removed again; ours.
                unsafe { entry_free(entry) };
                return Err(e);
            }
            entry
        };
        self.set_dirty(true);
        Ok(result)
    }

    // ----- conflicts -----

    /// `index_conflict__get_byindex`: the conflict at `n` (its ancestor,
    /// ours and theirs entries) and how many entries it spans.
    pub(crate) fn conflict_at(&self, mut n: usize) -> ([Option<EntryPtr>; 3], usize) {
        let icase = self.ignore_case();
        let mut out = [None; 3];
        let mut len = 0;
        let mut path: Option<&[u8]> = None;
        while let Some(e) = self.entry_at(n) {
            // SAFETY: a live entry of the index.
            let (ep, es) = unsafe { eps(e) };
            if let Some(p) = path
                && path_cmp(ep, p, icase) != 0
            {
                break;
            }
            path = Some(ep);
            if (1..=3).contains(&es) {
                out[es as usize - 1] = Some(e);
                len += 1;
            }
            n += 1;
        }
        (out, len)
    }

    /// `git_index_conflict_get`.
    pub(crate) fn conflict_get(&mut self, path: &[u8]) -> Result<[Option<EntryPtr>; 3]> {
        let Some(pos) = self.find_path(path) else {
            let _ = git_error_set!(ErrorClass::INDEX, "index does not contain {}", Bytes(path));
            return Err(ErrorCode::NOT_FOUND);
        };
        let (out, len) = self.conflict_at(pos);
        if len == 0 {
            return Err(ErrorCode::NOT_FOUND);
        }
        Ok(out)
    }

    /// `index_conflict_remove`: the conflict entries at `path` (all of
    /// them when `None`).
    pub(crate) fn conflict_remove(&mut self, path: Option<&[u8]>) -> Result<()> {
        let mut pos = 0;
        if let Some(p) = path {
            match self.find_path(p) {
                Some(x) => pos = x,
                None => {
                    let _ =
                        git_error_set!(ErrorClass::INDEX, "index does not contain {}", Bytes(p));
                    return Err(ErrorCode::NOT_FOUND);
                }
            }
        }
        let icase = self.ignore_case();
        while let Some(e) = self.entry_at(pos) {
            // SAFETY: a live entry of the index.
            let (ep, es) = unsafe { eps(e) };
            if let Some(p) = path
                && path_cmp(ep, p, icase) != 0
            {
                break;
            }
            if es == 0 {
                pos += 1;
                continue;
            }
            self.remove_entry(pos)?;
        }
        Ok(())
    }

    /// `git_index_has_conflicts`.
    pub(crate) fn has_conflicts(&self) -> bool {
        // SAFETY: live entries of the index.
        self.entries
            .iter()
            .any(|e| unsafe { estage(as_entry(e)) } > 0)
    }

    // ----- removal -----

    /// `git_index_remove`.
    pub(crate) fn remove(&mut self, path: &[u8], stage: c_int) -> Result<()> {
        let mut key = EntryHead {
            entry: GitIndexEntry::zeroed(),
            pathlen: path.len(),
        };
        key.entry.path = path.as_ptr().cast();
        key.entry.set_stage(stage);
        // SAFETY: `key` is a head whose path is `path`, live for the call.
        unsafe { self.entries_map.remove(NonNull::from(&mut key)) };
        match self.find(path, stage) {
            Ok(pos) => self.remove_entry(pos),
            Err(_) => {
                let _ = git_error_set!(
                    ErrorClass::INDEX,
                    "index does not contain {} at stage {}",
                    Bytes(path),
                    stage
                );
                Err(ErrorCode::NOT_FOUND)
            }
        }
    }

    /// `git_index_remove_directory`, given the directory with its trailing
    /// slash (`git_fs_path_to_dir`).
    pub(crate) fn remove_directory(&mut self, pfx: &[u8], stage: c_int) -> Result<()> {
        let mut pos = match self.find(pfx, GIT_INDEX_STAGE_ANY) {
            Ok(p) | Err(p) => p,
        };
        while let Some(e) = self.entry_at(pos) {
            // SAFETY: a live entry of the index.
            let (ep, es) = unsafe { eps(e) };
            if !ep.starts_with(pfx) {
                break;
            }
            if es != stage {
                pos += 1;
                continue;
            }
            self.remove_entry(pos)?;
        }
        Ok(())
    }

    /// `git_index_find_prefix`.
    pub(crate) fn find_prefix(&mut self, prefix: &[u8]) -> Option<usize> {
        let pos = match self.find(prefix, GIT_INDEX_STAGE_ANY) {
            Ok(p) | Err(p) => p,
        };
        let e = self.entry_at(pos)?;
        // SAFETY: a live entry of the index.
        if unsafe { epath(e) }.starts_with(prefix) {
            Some(pos)
        } else {
            None
        }
    }

    // ----- NAME entries -----

    /// `git_index_name_add`.
    pub(crate) fn name_add(
        &mut self,
        ancestor: Option<&CStr>,
        ours: Option<&CStr>,
        theirs: Option<&CStr>,
    ) -> Result<()> {
        let ne = alloc::calloc(1, size_of::<GitIndexNameEntry>()).cast::<GitIndexNameEntry>();
        if ne.is_null() {
            return Err(ErrorCode::GENERIC);
        }
        let dup = |s: Option<&CStr>, field: *mut *mut c_char| -> bool {
            match s {
                None => true,
                Some(s) => {
                    let p = alloc::strdup(s);
                    // SAFETY: `field` is a field of the fresh `ne`.
                    unsafe { field.write(p) };
                    !p.is_null()
                }
            }
        };
        // SAFETY: `ne` is a fresh zeroed name entry, ours alone.
        let n = unsafe { &mut *ne };
        let ok = dup(ancestor, &raw mut n.ancestor)
            && dup(ours, &raw mut n.ours)
            && dup(theirs, &raw mut n.theirs)
            && self.names.insert(ne.cast()).is_ok();
        if !ok {
            // SAFETY: ours, not inserted.
            unsafe { name_free(ne) };
            return Err(ErrorCode::GENERIC);
        }
        self.set_dirty(true);
        Ok(())
    }

    /// `git_index_name_clear`.
    pub(crate) fn name_clear(&mut self) {
        for ne in self.names.iter() {
            // SAFETY: the vector's name entries are ours.
            unsafe { name_free(ne.cast()) };
        }
        self.names.clear();
        self.set_dirty(true);
    }

    // ----- REUC entries -----

    /// `git_index_reuc_add`.
    pub(crate) fn reuc_add(
        &mut self,
        path: &[u8],
        modes: [c_int; 3],
        oids: [Option<Oid>; 3],
    ) -> Result<()> {
        let Some(reuc) = reuc_alloc(path) else {
            return Err(errors::oom());
        };
        for i in 0..3 {
            // SAFETY: `reuc` is fresh and ours.
            unsafe { (*reuc.as_ptr()).mode[i] = modes[i] as u32 };
            if modes[i] > 0 {
                let Some(o) = oids[i] else {
                    // SAFETY: ours.
                    unsafe { reuc_free(reuc.as_ptr().cast()) };
                    let e = match i {
                        0 => errors::assert_failed("ancestor_oid"),
                        1 => errors::assert_failed("our_oid"),
                        _ => errors::assert_failed("their_oid"),
                    };
                    return Err(e);
                };
                // SAFETY: as above.
                unsafe { (*reuc.as_ptr()).oid[i] = o };
            }
        }
        // `index_reuc_insert`
        if !self.reuc.is_sorted() {
            // SAFETY: ours.
            unsafe { reuc_free(reuc.as_ptr().cast()) };
            return Err(errors::assert_failed("git_vector_is_sorted(&index->reuc)"));
        }
        let icase = self.reuc_icase();
        let r = self.reuc.insert_sorted_by(
            reuc.as_ptr().cast(),
            // SAFETY: REUC entries.
            |a, b| unsafe { reuc_cmp_raw(a, b, icase) },
            Some(&mut |old: &mut *mut c_void, new: *mut c_void| {
                // `index_reuc_on_dup`
                // SAFETY: the old entry is ours, replaced by `new`.
                unsafe { reuc_free(*old) };
                *old = new;
                ErrorCode::EXISTS.raw()
            }),
        );
        self.set_dirty(true);
        match r {
            Ok(()) => Ok(()),
            Err(e) if e == ErrorCode::EXISTS => Ok(()),
            Err(e) => {
                // SAFETY: not inserted; ours.
                unsafe { reuc_free(reuc.as_ptr().cast()) };
                Err(e)
            }
        }
    }

    /// `git_index_reuc_find`.
    pub(crate) fn reuc_find(&mut self, path: &[u8]) -> core::result::Result<usize, usize> {
        self.sort_reuc();
        // `reuc_search`: strcmp/strcasecmp, by the index's case
        let icase = self.ignore_case();
        // SAFETY: REUC entries.
        bsearch(self.reuc.as_slice(), |&r| unsafe {
            reuc_path_cmp(path, reuc_path(r.cast()), icase)
        })
    }

    /// `git_index_reuc_remove`.
    pub(crate) fn reuc_remove(&mut self, position: usize) -> Result<()> {
        git_assert!(self.reuc.is_sorted(), "git_vector_is_sorted(&index->reuc)");
        let r = self.reuc.get(position);
        let res = self.reuc.remove(position);
        if res.is_ok()
            && let Some(r) = r
        {
            // SAFETY: removed; ours.
            unsafe { reuc_free(r) };
        }
        self.set_dirty(true);
        res
    }

    /// `git_index_reuc_clear`.
    pub(crate) fn reuc_clear(&mut self) {
        for r in self.reuc.as_mut_slice() {
            // SAFETY: ours.
            unsafe { reuc_free(*r) };
            *r = ptr::null_mut();
        }
        self.reuc.clear();
        self.set_dirty(true);
    }

    // ----- extensions -----

    /// `git_index_extension_lookup`'s search.
    pub(crate) fn extension_find(&self, signature: &[u8]) -> core::result::Result<usize, usize> {
        bsearch(&self.extensions, |x| sig_cmp(signature, &x.signature))
    }

    /// `git_index_extension_add`.
    pub(crate) fn extension_add(&mut self, signature: [u8; 4], data: &[u8]) -> Result<()> {
        let mut v = Vec::new();
        v.try_reserve_exact(data.len()).map_err(|_| errors::oom())?;
        v.extend_from_slice(data);
        if let Ok(pos) = self.extension_find(&signature) {
            self.extensions.remove(pos);
        }
        self.extensions.try_reserve(1).map_err(|_| errors::oom())?;
        let pos = match self.extension_find(&signature) {
            Ok(p) | Err(p) => p,
        };
        self.extensions
            .insert(pos, Extension { signature, data: v });
        Ok(())
    }

    /// `git_index_extension_remove`.
    pub(crate) fn extension_remove(&mut self, signature: &[u8]) -> Result<()> {
        match self.extension_find(signature) {
            Ok(pos) => {
                self.extensions.remove(pos);
                Ok(())
            }
            Err(_) => Err(ErrorCode::NOT_FOUND),
        }
    }

    /// `clear_uptodate`.
    pub(crate) fn clear_uptodate(&mut self) {
        for e in self.entries.iter() {
            // SAFETY: a live entry of the index, not otherwise borrowed.
            unsafe { (*as_entry(e).as_ptr()).entry.flags_extended &= !GIT_INDEX_ENTRY_UPTODATE };
        }
    }

    /// The last part of `index_free`: release everything.
    fn dispose(&mut self) {
        let _ = self.clear();
        self.entries_map.dispose();
        self.entries.dispose();
        self.names.dispose();
        self.reuc.dispose();
        self.extensions = Vec::new();
        // deleted entries are freed by `clear` (no reader is left)
        // SAFETY: the index owns its path string.
        unsafe { alloc::free(self.index_file_path.cast()) };
        self.index_file_path = ptr::null_mut();
    }
}

/// A pointer to a live [`GitIndex`], as C hands it around.
///
/// Every `IndexPtr` is made by [`IndexPtr::new`], whose contract is that the
/// index stays live while the handle is used. [`get`](IndexPtr::get) is the
/// one place where a reference to the index is formed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexPtr(NonNull<GitIndex>);

impl IndexPtr {
    /// A handle on `ix`.
    ///
    /// # Safety
    ///
    /// `ix` points to a live index (made by this module) that stays live
    /// while the handle (or a copy) is used, on this thread only.
    #[inline]
    pub unsafe fn new(ix: NonNull<GitIndex>) -> IndexPtr {
        IndexPtr(ix)
    }

    /// The raw pointer.
    #[inline]
    pub fn as_ptr(self) -> *mut GitIndex {
        self.0.as_ptr()
    }

    /// Exclusive access to the index.
    ///
    /// # Safety
    ///
    /// No other reference to the index (from this or any handle) is live
    /// while the result is, and no foreign code (C, callbacks) runs while
    /// it is: the borrow ends before the next call that could reach the
    /// index.
    #[inline(always)]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get<'a>(self) -> &'a mut GitIndex {
        // SAFETY: the caller's contract and `new`'s.
        unsafe { &mut *self.0.as_ptr() }
    }

    /// The owner (`GIT_REFCOUNT_OWNER`), through the atomic: no reference
    /// to the rest of the index.
    #[inline]
    pub fn owner(self) -> *mut c_void {
        self.rc().owner()
    }

    /// The reference count (atomics, which C also reads and writes through
    /// shared access): `rc` is the first field.
    #[inline]
    fn rc<'a>(self) -> &'a GitRefcount {
        // SAFETY: the index is live (`new`); `rc` is its first field
        // (`repr(C)`), only ever accessed through shared references.
        unsafe { &*self.0.as_ptr().cast::<GitRefcount>() }
    }

    /// The readers count (atomic).
    #[inline]
    pub(crate) fn readers<'a>(self) -> &'a Atomic32 {
        // SAFETY: the index is live; only the field's address is taken.
        let p = unsafe { &raw const (*self.0.as_ptr()).readers };
        // SAFETY: as above; the counter is atomic, never borrowed
        // exclusively while a handle reads it (the index's methods that
        // take `&mut self` only read or change it through `&`).
        unsafe { &*p }
    }

    /// `GIT_REFCOUNT_INC`.
    #[inline]
    pub fn incref(self) {
        self.rc().inc()
    }

    /// `git_index_free`: drop a reference, and free the index when it was
    /// the last (and it has no owner).
    ///
    /// # Safety
    ///
    /// The caller gives up its reference: it does not use the handle
    /// afterwards (unless it holds another reference).
    pub unsafe fn decref(self) {
        let free = self.rc().dec();
        if free {
            // SAFETY: the last reference: nothing else uses the index.
            unsafe { index_free(self) };
        }
    }
}

/// `index_free`.
///
/// # Safety
///
/// `ix` is the last reference.
unsafe fn index_free(ix: IndexPtr) {
    // SAFETY: the last reference, so no other access.
    let index = unsafe { ix.get() };
    // index iterators hold a reference, so none is left here
    if index.readers.get() != 0 {
        return;
    }
    index.dispose();
    // SAFETY: indexes are made by `Box::into_raw` (`ops::open_ext`), and
    // this is the last reference.
    drop(unsafe { Box::from_raw(ix.as_ptr()) });
}

/// `git_indexwriter`, mirrored.
#[repr(C)]
pub struct GitIndexWriter {
    /// `index`: a reference to the index, or NULL.
    pub index: *mut GitIndex,
    /// `file`.
    pub file: GitFilebuf,
    /// `should_write:1` (bit 0).
    pub should_write: c_uint,
}

/// `struct git_index_iterator`, Rust-private (the public header only
/// declares it).
pub struct GitIndexIterator {
    pub(crate) index: IndexPtr,
    pub(crate) snap: GitVector,
    pub(crate) cur: usize,
}

/// `struct git_index_conflict_iterator`, Rust-private.
pub struct GitIndexConflictIterator {
    pub(crate) index: IndexPtr,
    pub(crate) cur: usize,
}

crate::c_layout_test! {
    headers: ["index.h", "git2/sys/index.h"],
    types: [
        GitIndexTime => "git_index_time" { seconds, nanoseconds },
        GitIndexEntry => "git_index_entry" { ctime, mtime, dev, ino, mode, uid, gid, file_size, id, flags, flags_extended, path },
        GitIndexReucEntry => "git_index_reuc_entry" { mode, oid, path },
        GitIndexNameEntry => "git_index_name_entry" { ancestor, ours, theirs },
        GitIndexOptions => "git_index_options" { version, oid_type },
        GitIndexWriter => "git_indexwriter" { index, file },
        CPrefix => "git_index" { rc, index_file_path, stamp, checksum, entries, oid_type, tree, tree_pool, names, reuc },
    ],
}

/// The C-visible prefix of [`GitIndex`] alone, for the layout test (the
/// header's `struct git_index`).
#[cfg(test)]
#[repr(C)]
struct CPrefix {
    rc: GitRefcount,
    index_file_path: *mut c_char,
    stamp: Filestamp,
    checksum: [u8; crate::util::hash::MAX_SIZE],
    entries: GitVector,
    oid_type: git_oid_t,
    bits: c_uint,
    tree: *mut GitTreeCache,
    tree_pool: GitPool,
    names: GitVector,
    reuc: GitVector,
}

#[cfg(test)]
mod layout_rust {
    use super::*;

    #[test]
    fn prefix_offsets_match_the_header() {
        macro_rules! same {
            ($($f:ident),*) => {$(
                assert_eq!(core::mem::offset_of!(GitIndex, $f), core::mem::offset_of!(CPrefix, $f));
            )*};
        }
        same!(
            rc,
            index_file_path,
            stamp,
            checksum,
            entries,
            oid_type,
            bits,
            tree,
            tree_pool,
            names,
            reuc
        );
        assert!(core::mem::offset_of!(GitIndex, entries_map) >= size_of::<CPrefix>());
        assert_eq!(core::mem::offset_of!(GitIndex, rc), 0);
        assert_eq!(core::mem::offset_of!(GitIndex, index_file_path), 16);
        assert_eq!(core::mem::offset_of!(GitIndex, stamp), 24);
        assert_eq!(core::mem::offset_of!(GitIndex, checksum), 56);
        assert_eq!(core::mem::offset_of!(GitIndex, entries), 88);
        assert_eq!(core::mem::offset_of!(GitIndex, oid_type), 128);
        assert_eq!(core::mem::offset_of!(GitIndex, bits), 132);
        assert_eq!(core::mem::offset_of!(GitIndex, tree), 136);
        assert_eq!(core::mem::offset_of!(GitIndex, tree_pool), 144);
        assert_eq!(core::mem::offset_of!(GitIndex, names), 168);
        assert_eq!(core::mem::offset_of!(GitIndex, reuc), 208);
        assert_eq!(core::mem::offset_of!(GitIndex, entries_map), 248);
        assert_eq!(size_of::<EntryHead>(), 96);
        assert_eq!(size_of::<ReucHead>(), 120 + 8);
    }
}
