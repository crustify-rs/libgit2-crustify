//! Port of `src/libgit2/refdb_fs.c`: the filesystem reference database
//! backend (loose references under `refs/`, the `packed-refs` file, and the
//! reflogs under `logs/`).
//!
//! The backend is a Rust table that C calls into: a
//! [`CTable`]`<`[`RefdbBackendTable`]`, `[`FsBackend`]`>` whose slots
//! [`impl_cvtable!`](ffibox::impl_cvtable) mints, all shared (`&FsBackend`),
//! since the backend is used re-entrantly (writing a reference looks
//! references up) and from several threads at once
//! (`tests/libgit2/threads/refdb.c`). Its iterator is another table,
//! `CTable<RefIteratorTable, FsIter>`, whose slots are exclusive (an
//! iterator is used by one caller at a time).
//!
//! The state that C kept racily is synchronised:
//!
//! - the packed references (`refcache`) are a [`SortedCache`] (its own
//!   read-write lock);
//! - the `packed-refs` header's traits (`sorted`, `peeling_mode`), which C
//!   wrote from several threads, are atomics;
//! - the `packed-refs` mapping used by the sorted lookup is an `Arc` under
//!   a mutex: C read the mapping after releasing `prlock`, so a concurrent
//!   `packed_write` could unmap it under the reader (DEFECTS.md); here the
//!   reader keeps the mapping alive.
//!
//! The parsers (`packed-refs`, the binary search over a mapping, the
//! glob prefix) are safe Rust and Miri-tested.

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::ffi::CString;
use std::sync::{Arc, Mutex};

use ffibox::vtable::CTable;

use crate::git_error_set;
use crate::oid::{GIT_OID_MAX_SIZE, Oid, OidType};
use crate::refdb::{
    self, GIT_REFDB_BACKEND_INIT_FORCE_HEAD, GIT_REFDB_BACKEND_INIT_IS_WORKTREE,
    GIT_REFDB_BACKEND_VERSION, RefIteratorTable, RefdbBackendTable, RefdbPtr, git_refdb_backend,
    git_reference_iterator,
};
use crate::reflog::{self, GitReflog};
use crate::refs::deps::{self as p, RepoPtr, configmap, git_repository, git_signature};
use crate::refs::{
    self, GIT_HEAD_REF, GIT_PACKEDREFS_FILE, GIT_PACKEDREFS_FILE_MODE, GIT_PACKEDREFS_HEADER,
    GIT_REFERENCE_DIRECT, GIT_REFERENCE_SYMBOLIC, GIT_REFS_DIR, GIT_REFS_FILE_MODE,
    GIT_REFS_HEADS_DIR, GIT_REFS_TAGS_DIR, GIT_SYMREF, GitReference, RefBox, RefView,
};
use crate::sys::iterator::{
    FsIterator, GIT_ITERATOR_DESCEND_SYMLINKS, GIT_ITERATOR_IGNORE_CASE,
    GIT_ITERATOR_PRECOMPOSE_UNICODE,
};
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::filebuf::{GIT_FILEBUF_CREATE_LEADING_DIRS, GIT_FILEBUF_FSYNC, GitFilebuf};
use crate::util::fs_path::{
    self, GIT_FS_PATH_DIR_IGNORE_CASE, GIT_FS_PATH_DIR_PRECOMPOSE_UNICODE, str_cpath,
};
use crate::util::futils::{
    self, GIT_RMDIR_EMPTY_PARENTS, GIT_RMDIR_SKIP_NONEMPTY, GIT_RMDIR_SKIP_ROOT,
};
use crate::util::pool::GitPool;
use crate::util::posix::GitMap;
use crate::util::sortedcache::SortedCache;
use crate::util::str::GitStr;
use crate::util::util::{memmem, memrchr, prefixcmp, prefixncmp, strcmp, suffixcmp};
use crate::util::wildmatch::{WildmatchFlags, wildmatch};

/// `PACKREF_HAS_PEEL`.
pub const PACKREF_HAS_PEEL: u8 = 1;
/// `PACKREF_WAS_LOOSE`.
pub const PACKREF_WAS_LOOSE: u8 = 2;
/// `PACKREF_CANNOT_PEEL`.
pub const PACKREF_CANNOT_PEEL: u8 = 4;
/// `PACKREF_SHADOWED`.
pub const PACKREF_SHADOWED: u8 = 8;

/// `PEELING_NONE`.
pub const PEELING_NONE: u8 = 0;
/// `PEELING_STANDARD`.
pub const PEELING_STANDARD: u8 = 1;
/// `PEELING_FULL`.
pub const PEELING_FULL: u8 = 2;

/// `GIT_REFLOG_DIR`.
pub const GIT_REFLOG_DIR: &[u8] = b"logs/";
/// `GIT_REFLOG_DIR_MODE`.
pub const GIT_REFLOG_DIR_MODE: u32 = 0o777;
/// `GIT_REFLOG_FILE_MODE`.
pub const GIT_REFLOG_FILE_MODE: u32 = 0o666;

/// `GIT_REPOSITORY_INIT_SHARED_UMASK`.
const SHARED_UMASK: u32 = 0;
/// `GIT_REPOSITORY_INIT_SHARED_GROUP`.
const SHARED_GROUP: u32 = 0o002775;
/// `GIT_REPOSITORY_INIT_SHARED_ALL`.
const SHARED_ALL: u32 = 0o002777;

/// `struct packref` but its name (the cache's key): what a packed
/// reference holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PackRef {
    /// The target.
    pub oid: Oid,
    /// The peeled target, if `PACKREF_HAS_PEEL`.
    pub peel: Oid,
    /// `PACKREF_*`.
    pub flags: u8,
}

// ---------------------------------------------------------------------------
// packed-refs parsing (safe, Miri-tested)
// ---------------------------------------------------------------------------

/// The traits of a `packed-refs` header (`# pack-refs with: …`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackedTraits {
    /// `PEELING_*`.
    pub peeling_mode: u8,
    /// Whether ` sorted ` is a trait; `None` without a header (C leaves
    /// the previous value).
    pub sorted: Option<bool>,
}

/// A `packed-refs` file the parser rejects ("corrupted packed references
/// file").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Corrupted;

/// `packed_set_peeling_mode`: the header's traits, and the offset after
/// it (`None` if the header has no end of line: C's NULL).
pub fn packed_header(data: &[u8]) -> (PackedTraits, Option<usize>) {
    const TRAITS: &[u8] = b"# pack-refs with:";
    let mut t = PackedTraits {
        peeling_mode: PEELING_NONE,
        sorted: None,
    };
    if prefixncmp(data, data.len(), TRAITS) != 0 {
        return (t, Some(0));
    }
    let rest = &data[TRAITS.len()..];
    let Some(eol) = memchr::memchr(b'\n', rest) else {
        return (t, None);
    };
    let line = &rest[..eol];
    if memmem(line, b" fully-peeled ").is_some() {
        t.peeling_mode = PEELING_FULL;
    } else if memmem(line, b" peeled ").is_some() {
        t.peeling_mode = PEELING_STANDARD;
    }
    t.sorted = Some(memmem(line, b" sorted ").is_some());
    (t, Some(TRAITS.len() + eol + 1))
}

/// C's `strchr(p, '\n')` on a NUL-terminated buffer: the newline's offset
/// from `from`, unless a NUL (or the end) comes first.
fn strchr_nl(data: &[u8], from: usize) -> Option<usize> {
    let rest = data.get(from..)?;
    let i = memchr::memchr2(b'\n', 0, rest)?;
    (rest[i] == b'\n').then_some(from + i)
}

/// The bytes at `from` that a C reader sees: up to the NUL after the
/// contents (`at` past the end is NUL).
fn at(data: &[u8], i: usize) -> u8 {
    data.get(i).copied().unwrap_or(0)
}

/// `git_oid_from_prefix(&oid, scan, hexsize, type)` on the buffer: reads
/// hex digits up to the first invalid one (the NUL terminator included).
fn oid_at(data: &[u8], from: usize, t: OidType) -> Option<Oid> {
    let n = t.hexsize();
    let end = from.checked_add(n)?;
    let s = data.get(from..end)?;
    Oid::parse_prefix(s, t).ok()
}

/// The body of `packed_reload`: parse `data` (the file, which C reads with
/// a NUL after it), calling `upsert(name)` for each record and filling the
/// item it returns. `Err(Corrupted)` is "corrupted packed references file".
pub fn parse_packed(
    data: &[u8],
    oid_type: OidType,
    mut traits_out: impl FnMut(PackedTraits),
    mut upsert: impl FnMut(&[u8], &mut dyn FnMut(&mut PackRef)) -> Result<()>,
) -> core::result::Result<(), Corrupted> {
    let hexsize = oid_type.hexsize();
    let eof = data.len();
    let (traits, start) = packed_header(data);
    traits_out(traits);
    let mut scan = start.ok_or(Corrupted)?;
    while scan < eof && data[scan] == b'#' {
        let eol = strchr_nl(data, scan).ok_or(Corrupted)?;
        scan = eol + 1;
    }
    while scan < eof {
        let oid = oid_at(data, scan, oid_type).ok_or(Corrupted)?;
        scan += hexsize;
        if at(data, scan) != b' ' {
            return Err(Corrupted);
        }
        scan += 1;
        let eol = strchr_nl(data, scan).ok_or(Corrupted)?;
        let mut name_end = eol;
        if eol > 0 && data[eol - 1] == b'\r' {
            // `eol[-1] = '\0'`: only if it is within the name.
            if eol > scan {
                name_end = eol - 1;
            }
        }
        let name = &data[scan..name_end];
        scan = eol + 1;
        // C upserts before it parses the peel line; a failure there
        // empties the cache anyway, so the record is applied whole.
        let mut peel = None;
        if at(data, scan) == b'^' {
            peel = Some(oid_at(data, scan + 1, oid_type).ok_or(Corrupted)?);
            scan += hexsize + 1;
            if scan < eof {
                let eol = strchr_nl(data, scan).ok_or(Corrupted)?;
                scan = eol + 1;
            }
        }
        let cannot_peel = peel.is_none()
            && (traits.peeling_mode == PEELING_FULL
                || (traits.peeling_mode == PEELING_STANDARD
                    && prefixcmp(name, GIT_REFS_TAGS_DIR) == 0));
        upsert(name, &mut |item: &mut PackRef| {
            item.oid = oid;
            if let Some(p) = peel {
                item.peel = p;
                item.flags |= PACKREF_HAS_PEEL;
            } else if cannot_peel {
                item.flags |= PACKREF_CANNOT_PEEL;
            }
        })
        .map_err(|_| Corrupted)?;
    }
    Ok(())
}

/// `start_of_record(buf, p)`: the start of the record (with its peel
/// line) that the byte `p` (an offset) is in.
fn start_of_record(data: &[u8], lo: usize, p: usize) -> usize {
    let mut nl = p;
    loop {
        let Some(i) = memrchr(&data[lo..nl], b'\n') else {
            return lo;
        };
        let i = lo + i;
        if at(data, i + 1) == b'^' && i > lo {
            nl = i - 1 + 1;
            // C: `--nl` then searches `[buf, nl)`: the byte before the
            // newline is excluded from the next search.
            nl -= 1;
            continue;
        }
        return i + 1;
    }
}

/// `end_of_record(p, end)`: the start of the record after the one at `p`.
fn end_of_record(data: &[u8], p: usize, end: usize) -> usize {
    let mut p = p;
    loop {
        let Some(i) = memchr::memchr(b'\n', &data[p..end]) else {
            return end;
        };
        p += i + 1;
        if p < end && data[p] == b'^' {
            p += 1;
        } else {
            return p;
        }
    }
}

/// `cmp_record_to_refname`: the record at `rec` against `ref_name`.
fn cmp_record_to_refname(data: &[u8], rec: usize, ref_name: &[u8], hexsize: usize) -> c_int {
    let data_end = data.len() - rec;
    if data_end < hexsize + 3 {
        return -1;
    }
    let start = rec + hexsize + 1;
    let mut len = data_end - (hexsize + 1);
    if let Some(e) = memchr::memchr(b'\n', &data[start..start + len]) {
        len = e;
    }
    let n = ref_name.len().min(len);
    let c = match data[start..start + n].cmp(&ref_name[..n]) {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    };
    if c == 0 && len != ref_name.len() {
        return if len > ref_name.len() { 1 } else { -1 };
    }
    c
}

/// The result of [`packed_search`].
#[derive(Debug, PartialEq, Eq)]
pub enum Searched {
    /// The reference's id and peel.
    Found(Oid, Option<Oid>),
    /// Not in the file.
    NotFound,
}

/// The binary search of `packed_lookup` over a sorted `packed-refs`
/// mapping. `Err(Corrupted)` is "corrupted packed references file".
pub fn packed_search(
    data: &[u8],
    ref_name: &[u8],
    oid_type: OidType,
) -> core::result::Result<Searched, Corrupted> {
    let hexsize = oid_type.hexsize();
    let data_end = data.len();
    let mut left = 0usize;
    let mut right = data_end;
    while left < right && data[left] == b'#' {
        let i = memchr::memchr(b'\n', &data[left..data_end]).ok_or(Corrupted)?;
        left += i + 1;
    }
    while left < right {
        let mid = left + (right - left) / 2;
        let rec = start_of_record(data, left, mid);
        let compare = cmp_record_to_refname(data, rec, ref_name, hexsize);
        if compare < 0 {
            left = end_of_record(data, mid, right);
        } else if compare > 0 {
            right = rec;
        } else {
            if data_end - rec < hexsize {
                return Err(Corrupted);
            }
            let oid =
                Oid::parse_prefix(&data[rec..rec + hexsize], oid_type).map_err(|_| Corrupted)?;
            let mut r = rec + hexsize + 1;
            let eol = r + memchr::memchr(b'\n', data.get(r..data_end).ok_or(Corrupted)?)
                .ok_or(Corrupted)?;
            let mut peel = None;
            if eol + 1 < data_end {
                r = eol + 1;
                if data[r] == b'^' {
                    r += 1;
                    if data_end - r < hexsize {
                        return Err(Corrupted);
                    }
                    peel = Some(
                        Oid::parse_prefix(&data[r..r + hexsize], oid_type)
                            .map_err(|_| Corrupted)?,
                    );
                }
            }
            return Ok(Searched::Found(oid, peel));
        }
    }
    Ok(Searched::NotFound)
}

/// `ref_is_available(old_ref, new_ref, this_ref)`: whether `new_ref` does
/// not collide (as a directory or a file) with `this_ref`, which may be
/// `old_ref` (being renamed).
pub fn ref_is_available(old_ref: Option<&[u8]>, new_ref: &[u8], this_ref: &[u8]) -> bool {
    if old_ref.is_none_or(|o| strcmp(o, this_ref) != 0) {
        let cmplen = this_ref.len().min(new_ref.len());
        let lead = if this_ref.len() < new_ref.len() {
            new_ref
        } else {
            this_ref
        };
        if new_ref[..cmplen] == this_ref[..cmplen] && at(lead, cmplen) == b'/' {
            return false;
        }
    }
    true
}

/// Whether `new_ref` collides with one of the sorted names `name_at(0..len)`
/// (none of which contains a NUL), skipping `old_ref`: what
/// `reference_path_available`'s scan of every packed reference with
/// [`ref_is_available`] finds, in `O(log n)`: a name that is a directory
/// of `new_ref` (looked up with `contains`), or a name under `new_ref/`
/// (a range of the sorted names).
pub fn path_collides<'a>(
    new_ref: &[u8],
    old_ref: Option<&[u8]>,
    len: usize,
    name_at: impl Fn(usize) -> &'a [u8],
    contains: impl Fn(&[u8]) -> bool,
) -> bool {
    let skip = |n: &[u8]| old_ref.is_some_and(|o| o == n);
    for (i, &c) in new_ref.iter().enumerate() {
        if c == b'/' {
            let dir = &new_ref[..i];
            if !skip(dir) && contains(dir) {
                return true;
            }
        }
    }
    let mut prefix = Vec::with_capacity(new_ref.len() + 1);
    prefix.extend_from_slice(new_ref);
    prefix.push(b'/');
    let (mut lo, mut hi) = (0, len);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if name_at(mid) < prefix.as_slice() {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    (lo..len)
        .map(&name_at)
        .take_while(|n| n.starts_with(&prefix))
        .any(|n| !skip(n))
}

/// `iter_load_optimize_prefix`: the directory part of a glob before its
/// first wildcard (with the slash), or `None` (then `refs/`).
pub fn glob_prefix(glob: &[u8]) -> Option<&[u8]> {
    let mut last_sep = None;
    for (i, &c) in glob.iter().enumerate() {
        match c {
            b'?' | b'*' | b'[' | b'\\' => break,
            b'/' => last_sep = Some(i),
            _ => {}
        }
    }
    last_sep.map(|i| &glob[..=i])
}

/// The namespace path of `setup_namespace`: `refs/namespaces/<a>/…` for
/// each component of `namespace`, then `refs/namespaces/<last>/refs`.
pub fn namespace_path(namespace: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let parts: Vec<&[u8]> = namespace.split(|&c| c == b'/').collect();
    let (last, init): (&[u8], &[&[u8]]) = match parts.split_last() {
        Some((l, i)) => (l, i),
        None => (b"", &[]),
    };
    for p in init {
        out.extend_from_slice(b"refs/namespaces/");
        out.extend_from_slice(p);
        out.push(b'/');
    }
    out.extend_from_slice(b"refs/namespaces/");
    out.extend_from_slice(last);
    out.extend_from_slice(b"/refs");
    out
}

// ---------------------------------------------------------------------------
// The packed-refs mapping
// ---------------------------------------------------------------------------

/// A read-only mapping of `packed-refs`, unmapped when the last user drops
/// it, with the `sorted` trait of its own header (`None`: no header, so the
/// trait last seen applies, as in C).
struct Mapping(GitMap, Option<bool>);

// SAFETY: a private read-only mapping that this process never writes;
// unmapped once, in `drop`, when no reference into it remains.
unsafe impl Send for Mapping {}
// SAFETY: as above.
unsafe impl Sync for Mapping {}

impl Mapping {
    fn bytes(&self) -> &[u8] {
        if self.0.data.is_null() || self.0.len == 0 {
            return &[];
        }
        // SAFETY: a live mapping of `len` readable bytes, unmapped only
        // when the last `Arc` drops, after this borrow. This process never
        // writes the file in place (`packed_write` renames a new file over
        // it). That no other process does either is an assumption, the
        // contract git and libgit2 keep (they replace `packed-refs` by a
        // rename), as for packs: an external in-place rewrite is outside
        // it (in C it gives mixed reads; a truncation is a SIGBUS in both).
        unsafe { core::slice::from_raw_parts(self.0.data.cast::<u8>(), self.0.len) }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: our mapping, from `mmap_ro`; no borrow of it outlives
        // the last `Arc`.
        unsafe { crate::util::posix::munmap(&mut self.0) };
    }
}

#[derive(Default)]
struct PackedMap {
    map: Option<Arc<Mapping>>,
    stamp: futils::Filestamp,
}

// ---------------------------------------------------------------------------
// Lock files
// ---------------------------------------------------------------------------

/// A locked file (`git_filebuf`), cleaned up (the lock removed unless
/// committed) on drop.
pub struct LockFile(Option<Box<GitFilebuf>>);

impl LockFile {
    fn open(path: &[u8], flags: c_int, mode: u32) -> Result<LockFile> {
        let mut f = Box::new(GitFilebuf::new());
        // SAFETY: the box keeps the filebuf in place until it is cleaned
        // up (`Drop`, or the commit); only the box pointer ever moves.
        unsafe { f.open(path, flags, mode) }?;
        Ok(LockFile(Some(f)))
    }

    fn file(&mut self) -> &mut GitFilebuf {
        self.0.as_mut().expect("a lock file handed to C")
    }

    /// The target path.
    pub fn original_path(&self) -> Option<&CStr> {
        self.0.as_ref().and_then(|f| f.original_path())
    }

    fn printf(&mut self, data: &[u8]) -> Result<()> {
        self.file().printf_formatted(Some(data))
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        self.file().write(data)
    }

    fn commit(&mut self) -> Result<()> {
        self.file().commit()
    }

    /// Hand the lock to C as `lock`'s payload.
    fn into_payload(mut self) -> *mut c_void {
        let f = self.0.take().expect("a lock file handed to C");
        Box::into_raw(f).cast()
    }

    /// Adopt a payload `into_payload` made.
    ///
    /// # Safety
    ///
    /// `p` came from `into_payload` and is not used again.
    unsafe fn from_payload(p: *mut c_void) -> LockFile {
        // SAFETY: per the contract: a `Box<GitFilebuf>` we gave away.
        LockFile(Some(unsafe { Box::from_raw(p.cast::<GitFilebuf>()) }))
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        if let Some(f) = self.0.as_mut() {
            f.cleanup();
        }
    }
}

// ---------------------------------------------------------------------------
// The backend
// ---------------------------------------------------------------------------

/// The state of a filesystem backend (`refdb_fs_backend` but its table).
pub struct FsInner {
    repo: RepoPtr,
    /// The (namespaced) git directory, with its trailing slash.
    gitpath: Option<Vec<u8>>,
    /// The (namespaced) common directory, with its trailing slash.
    commonpath: Option<Vec<u8>>,
    oid_type: c_int,
    sorted: AtomicBool,
    peeling_mode: AtomicU8,
    refcache: SortedCache<PackRef>,
    packed: Mutex<PackedMap>,
}

/// The state behind a filesystem backend's table.
pub struct FsBackend(Arc<FsInner>);

fn cpath(s: &GitStr) -> std::borrow::Cow<'_, CStr> {
    str_cpath(s)
}

fn cbytes(b: &[u8]) -> CString {
    fs_path::to_cstring(b)
}

impl FsInner {
    fn oid_kind(&self) -> Result<OidType> {
        OidType::from_raw(self.oid_type as u32)
            .ok_or_else(|| git_error_set!(ErrorClass::INVALID, "unknown object id type"))
    }

    fn gitpath(&self) -> &[u8] {
        self.gitpath.as_deref().unwrap_or_default()
    }

    fn commonpath(&self) -> &[u8] {
        self.commonpath.as_deref().unwrap_or_default()
    }

    fn set_traits(&self, t: PackedTraits) {
        self.peeling_mode.store(t.peeling_mode, Ordering::Relaxed);
        if let Some(s) = t.sorted {
            self.sorted.store(s, Ordering::Relaxed);
        }
    }

    fn configmap_flag(&self, item: c_int) -> bool {
        matches!(self.repo.configmap(item), Ok(t) if t != 0)
    }

    /// `refdb_fs_get_iterator_flags`.
    fn iterator_flags(&self) -> c_uint {
        let mut flags = GIT_ITERATOR_DESCEND_SYMLINKS;
        if self.configmap_flag(configmap::IGNORECASE) {
            flags |= GIT_ITERATOR_IGNORE_CASE;
        }
        if self.configmap_flag(configmap::PRECOMPOSE) {
            flags |= GIT_ITERATOR_PRECOMPOSE_UNICODE;
        }
        flags
    }

    /// `refdb_fs_get_direach_flags`.
    fn direach_flags(&self) -> u32 {
        let mut flags = 0;
        if self.configmap_flag(configmap::IGNORECASE) {
            flags |= GIT_FS_PATH_DIR_IGNORE_CASE;
        }
        if self.configmap_flag(configmap::PRECOMPOSE) {
            flags |= GIT_FS_PATH_DIR_PRECOMPOSE_UNICODE;
        }
        flags
    }

    /// `refdb_fs_should_fsync`.
    fn should_fsync(&self) -> bool {
        self.configmap_flag(configmap::FSYNCOBJECTFILES) || p::fsync_gitdir()
    }

    /// `reflog_path`: `<gitdir or commondir>/logs/<refname>`.
    fn reflog_path(&self, refname: &[u8]) -> Result<GitStr> {
        let base = if refname == GIT_HEAD_REF {
            self.repo.gitdir()
        } else {
            self.repo.commondir()
        };
        let mut out = GitStr::new();
        out.joinpath(base.as_deref().unwrap_or_default(), GIT_REFLOG_DIR)?;
        out.joinpath_self(refname)?;
        Ok(out)
    }

    /// `packed_reload`: refresh the packed references if the file changed.
    fn packed_reload(&self) -> Result<()> {
        if self.gitpath.is_none() {
            return Ok(());
        }
        let loaded = match self.refcache.lock_and_load() {
            Ok(None) => return Ok(()),
            Ok(Some(l)) => l,
            Err(e) if e.raw_code() == ErrorCode::NOT_FOUND => {
                self.refcache.write().clear();
                errors::clear();
                return Ok(());
            }
            Err(e) => return Err(e.raw_code()),
        };
        let (mut guard, contents) = loaded;
        guard.clear();
        let oid_type = self.oid_kind()?;
        let r = parse_packed(
            &contents,
            oid_type,
            |t| self.set_traits(t),
            |name, f| {
                let item = guard
                    .upsert_with(name, PackRef::default)
                    .map_err(|e| e.raw_code())?;
                f(item);
                Ok(())
            },
        );
        if r.is_err() {
            let _ = git_error_set!(ErrorClass::REFERENCE, "corrupted packed references file");
            guard.clear();
            return Err(ErrorCode::GENERIC);
        }
        Ok(())
    }

    /// `loose_lookup`: the loose reference `name`, from the git directory
    /// if it is per-worktree, else from the common directory.
    fn loose_lookup(&self, name: &[u8], want: bool) -> Result<Option<RefBox>> {
        let dir = if refs::is_per_worktree_ref(name) {
            self.gitpath()
        } else {
            self.commonpath()
        };
        refs::lookup_loose(dir, name, self.oid_type, want)
    }

    /// `packed_map_check`: the current mapping of `packed-refs` (remapped
    /// if the file changed), or `None` if there is none or it is empty.
    fn packed_map_check(&self) -> Result<Option<Arc<Mapping>>> {
        let mut st = self.packed.lock().unwrap_or_else(|e| e.into_inner());
        let path = cbytes(self.refcache.path());
        if st.map.is_some() && futils::filestamp_check(Some(&mut st.stamp), &path) == 0 {
            return Ok(st.map.clone());
        }
        packed_map_free(&mut st);
        let fd = match futils::open_ro(&path) {
            Ok(fd) => fd,
            Err(ErrorCode::NOT_FOUND) => {
                errors::clear();
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        let stat = match crate::util::posix::fstat(fd) {
            Ok(s) => s,
            Err(_) => {
                // SAFETY: our descriptor, closed once.
                let _ = unsafe { crate::util::posix::close(fd) };
                return Err(git_error_set!(
                    ErrorClass::OS,
                    "unable to stat packed-refs '{}'",
                    Bytes(path.to_bytes())
                ));
            }
        };
        if stat.st_size == 0 {
            // SAFETY: our descriptor, closed once.
            let _ = unsafe { crate::util::posix::close(fd) };
            return Ok(None);
        }
        st.stamp = futils::Filestamp::from_stat(&stat);
        let map = futils::mmap_ro(fd, 0, stat.st_size as usize);
        // SAFETY: our descriptor, closed once.
        let _ = unsafe { crate::util::posix::close(fd) };
        let mut map = Mapping(map?, None);
        let (t, _) = packed_header(map.bytes());
        map.1 = t.sorted;
        self.set_traits(t);
        let map = Arc::new(map);
        st.map = Some(Arc::clone(&map));
        Ok(Some(map))
    }

    /// `packed_unsorted_lookup`.
    fn packed_unsorted_lookup(&self, name: &[u8]) -> Result<RefBox> {
        self.packed_reload()?;
        let g = self.refcache.read();
        match g.get(name) {
            None => Err(ref_error_notfound(name)),
            Some(e) => RefBox::new_direct(name, &e.oid, Some(&e.peel)),
        }
    }

    /// `packed_lookup`.
    fn packed_lookup(&self, name: &[u8]) -> Result<RefBox> {
        let map = self.packed_map_check()?;
        // The trait of the mapping searched, not one a concurrent reload
        // of another file set (review l-refs-r1, LR-4).
        let sorted = map
            .as_deref()
            .and_then(|m| m.1)
            .unwrap_or_else(|| self.sorted.load(Ordering::Relaxed));
        if !sorted {
            return self.packed_unsorted_lookup(name);
        }
        let data = map.as_deref().map_or(&[][..], Mapping::bytes);
        match packed_search(data, name, self.oid_kind()?) {
            Ok(Searched::Found(oid, peel)) => RefBox::new_direct(name, &oid, peel.as_ref()),
            Ok(Searched::NotFound) => Err(ref_error_notfound(name)),
            Err(Corrupted) => Err(git_error_set!(
                ErrorClass::REFERENCE,
                "corrupted packed references file"
            )),
        }
    }

    /// `refdb_fs_backend__lookup`.
    fn lookup(&self, name: &[u8]) -> Result<RefBox> {
        match self.loose_lookup(name, true) {
            Ok(Some(r)) => Ok(r),
            Ok(None) => Err(ErrorCode::GENERIC),
            Err(ErrorCode::NOT_FOUND) => {
                errors::clear();
                self.packed_lookup(name)
            }
            Err(e) => Err(e),
        }
    }

    /// `refdb_fs_backend__exists`.
    fn exists(&self, name: &[u8]) -> Result<bool> {
        let mut path = GitStr::new();
        path.joinpath(self.gitpath(), name)?;
        if fs_path::isfile(&cpath(&path)) {
            return Ok(true);
        }
        self.packed_reload()?;
        Ok(self.refcache.read().get(name).is_some())
    }

    /// `loose_lock`: lock the loose file of `name`.
    fn loose_lock(&self, name: &[u8]) -> Result<LockFile> {
        // SAFETY: the backend's repository is live while the backend is
        // (C's contract with `git_refdb_backend_fs`).
        let valid = unsafe {
            crate::sys::path::is_valid_bytes(
                self.repo.as_ptr(),
                name,
                0,
                fs_path::GIT_FS_PATH_REJECT_FILESYSTEM_DEFAULTS,
            )
        };
        if !valid {
            let _ = git_error_set!(
                ErrorClass::INVALID,
                "invalid reference name '{}'",
                Bytes(name)
            );
            return Err(ErrorCode::INVALID_SPEC);
        }
        let basedir = if refs::is_per_worktree_ref(name) {
            self.gitpath()
        } else {
            self.commonpath()
        };
        futils::rmdir_r(name, Some(basedir), GIT_RMDIR_SKIP_NONEMPTY)?;
        let mut path = GitStr::new();
        path.joinpath(basedir, name)?;
        let mut flags = GIT_FILEBUF_CREATE_LEADING_DIRS;
        if self.should_fsync() {
            flags |= GIT_FILEBUF_FSYNC;
        }
        match LockFile::open(path.as_bytes(), flags, GIT_REFS_FILE_MODE) {
            Err(ErrorCode::DIRECTORY) => {
                let _ = git_error_set!(
                    ErrorClass::REFERENCE,
                    "cannot lock ref '{}', there are refs beneath that folder",
                    Bytes(name)
                );
                Err(ErrorCode::DIRECTORY)
            }
            r => r,
        }
    }

    /// `reference_path_available`.
    fn reference_path_available(
        &self,
        new_ref: &[u8],
        old_ref: Option<&[u8]>,
        force: bool,
    ) -> Result<()> {
        self.packed_reload()?;
        if !force && self.exists(new_ref)? {
            let _ = git_error_set!(
                ErrorClass::REFERENCE,
                "failed to write reference '{}': a reference with that name already exists.",
                Bytes(new_ref)
            );
            return Err(ErrorCode::EXISTS);
        }
        // C scans every packed reference with `ref_is_available`; the same
        // answer from the sorted cache.
        let g = self.refcache.read();
        let collides = path_collides(
            new_ref,
            old_ref,
            g.len(),
            |i| g.entry(i).map_or(&[][..], |(n, _)| n),
            |n| g.get(n).is_some(),
        );
        drop(g);
        if collides {
            return Err(git_error_set!(
                ErrorClass::REFERENCE,
                "path to reference '{}' collides with existing one",
                Bytes(new_ref)
            ));
        }
        Ok(())
    }

    /// `cmp_old_ref`: 0 if `name` has the old value (or none is given).
    fn cmp_old_ref(
        &self,
        name: &[u8],
        old_id: Option<&Oid>,
        old_target: Option<&[u8]>,
    ) -> Result<c_int> {
        if old_id.is_none() && old_target.is_none() {
            return Ok(0);
        }
        let old = match self.lookup(name) {
            Ok(r) => r,
            Err(ErrorCode::NOT_FOUND) if old_id.is_some_and(Oid::is_zero) => return Ok(0),
            Err(e) => return Err(e),
        };
        let v = old.view();
        if old_id.is_some() && v.raw_kind() != GIT_REFERENCE_DIRECT {
            return Ok(-1);
        }
        if old_target.is_some() && v.raw_kind() != GIT_REFERENCE_SYMBOLIC {
            return Ok(1);
        }
        let mut cmp = 0;
        if let Some(id) = old_id
            && v.raw_kind() == GIT_REFERENCE_DIRECT
        {
            cmp = id.cmp_raw(v.oid_field());
        }
        if let Some(t) = old_target
            && v.raw_kind() == GIT_REFERENCE_SYMBOLIC
        {
            cmp = strcmp(t, v.symbolic_target().unwrap_or_default());
        }
        Ok(cmp)
    }

    /// `maybe_append_head`: append to HEAD's reflog when HEAD points to
    /// `r`.
    fn maybe_append_head(
        &self,
        r: RefView<'_>,
        who: *const git_signature,
        message: Option<&CStr>,
    ) -> Result<()> {
        let db = self.repo.refdb()?;
        // SAFETY: the count `git_repository_refdb` gave us.
        let counted = unsafe { ffibox::CShared::from_raw(db.cast::<refdb::GitRefdb>()) }
            .ok_or(ErrorCode::GENERIC)?;
        // SAFETY: we hold a count on the live database.
        let dbp = unsafe { RefdbPtr::new(NonNull::from(&*counted)) };
        if !dbp.should_write_head_reflog(r)? {
            return Ok(());
        }
        let old_id = match refs::name_to_id(self.repo, r.name()) {
            Ok(id) => id,
            Err(_) => oid_clear(self.repo.oid_type()),
        };
        let head = refs::lookup(self.repo, GIT_HEAD_REF)?;
        self.reflog_append(head.view(), Some(&old_id), r.target(), who, message)
    }

    /// `reflog_append`: append an entry to the reflog of `r`; must be
    /// called under the reference's lock.
    fn reflog_append(
        &self,
        r: RefView<'_>,
        old: Option<&Oid>,
        new: Option<&Oid>,
        who: *const git_signature,
        message: Option<&CStr>,
    ) -> Result<()> {
        let is_symbolic = r.raw_kind() == GIT_REFERENCE_SYMBOLIC;
        if is_symbolic && r.name() != GIT_HEAD_REF && !(old.is_some() && new.is_some()) {
            return Ok(());
        }
        let mut old_id = oid_clear(self.oid_type);
        match old {
            Some(o) => old_id = *o,
            None => match refs::name_to_id(self.repo, r.name()) {
                Ok(id) => old_id = id,
                Err(ErrorCode::NOT_FOUND) => {}
                Err(e) => return Err(e),
            },
        }
        let new_id = match new {
            Some(n) => *n,
            None if !is_symbolic => *r.oid_field(),
            None => {
                let id = match refs::name_to_id(self.repo, r.symbolic_target().unwrap_or_default())
                {
                    Ok(id) => id,
                    // Detaching HEAD does not create an entry.
                    Err(ErrorCode::NOT_FOUND) => return Ok(()),
                    Err(e) => return Err(e),
                };
                errors::clear();
                id
            }
        };
        let mut buf = GitStr::new();
        reflog::serialize_entry(&mut buf, &old_id, &new_id, who, message.map(CStr::to_bytes))?;
        let path = self.reflog_path(r.name())?;
        match futils::mkpath2file(path.as_bytes(), 0o777) {
            Ok(()) | Err(ErrorCode::EXISTS) => {}
            Err(e) => return Err(e),
        }
        let cp = cpath(&path);
        if fs_path::isdir(&cp) {
            match futils::rmdir_r(path.as_bytes(), None, GIT_RMDIR_SKIP_NONEMPTY) {
                Err(ErrorCode::NOT_FOUND) => {}
                Err(e) => return Err(e),
                Ok(()) => {
                    if fs_path::isdir(&cp) {
                        let _ = git_error_set!(
                            ErrorClass::REFERENCE,
                            "cannot create reflog at '{}', there are reflogs beneath that folder",
                            Bytes(r.name())
                        );
                        return Err(ErrorCode::DIRECTORY);
                    }
                }
            }
        }
        let mut flags = libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND;
        if self.should_fsync() {
            flags |= futils::O_FSYNC;
        }
        futils::writebuffer(buf.as_bytes(), &cp, flags, GIT_REFLOG_FILE_MODE)
    }

    /// `refdb_fs_backend__write_tail`: check the old value, append to the
    /// reflogs, and commit `file`; the lock is released either way.
    #[allow(clippy::too_many_arguments)]
    fn write_tail(
        &self,
        r: RefView<'_>,
        file: LockFile,
        update_reflog: bool,
        old_id: Option<&Oid>,
        old_target: Option<&[u8]>,
        who: *const git_signature,
        message: Option<&CStr>,
    ) -> Result<()> {
        let mut file = file;
        let cmp = self.cmp_old_ref(r.name(), old_id, old_target)?;
        if cmp != 0 {
            let _ = git_error_set!(ErrorClass::REFERENCE, "old reference value does not match");
            return Err(ErrorCode::MODIFIED);
        }
        let (new_id, new_target) = if r.raw_kind() == GIT_REFERENCE_SYMBOLIC {
            (None, Some(r.symbolic_target().unwrap_or_default()))
        } else {
            (Some(r.oid_field()), None)
        };
        match self.cmp_old_ref(r.name(), new_id, new_target) {
            // The same value: nothing to do.
            Ok(0) => return Ok(()),
            Ok(_) | Err(ErrorCode::NOT_FOUND) => {}
            Err(e) => return Err(e),
        }
        if update_reflog {
            let db = RefdbPtr::of_repo(self.repo)?;
            if db.should_write_reflog(r)? {
                self.reflog_append(r, None, None, who, message)?;
                self.maybe_append_head(r, who, message)?;
            }
        }
        loose_commit(&mut file, r)
    }

    /// `refdb_fs_backend__write`.
    fn write(
        &self,
        r: RefView<'_>,
        force: bool,
        who: *const git_signature,
        message: Option<&CStr>,
        old_id: Option<&Oid>,
        old_target: Option<&[u8]>,
    ) -> Result<()> {
        self.reference_path_available(r.name(), None, force)?;
        let file = self.loose_lock(r.name())?;
        self.write_tail(r, file, true, old_id, old_target, who, message)
    }

    /// `refdb_fs_backend__prune_refs`: remove the empty directories left
    /// above `ref_name` (under `prefix`) in `refs/heads`, `refs/tags` or
    /// `refs/remotes`.
    fn prune_refs(&self, ref_name: &[u8], prefix: Option<&[u8]>) -> Result<()> {
        let mut relative = GitStr::new();
        relative.sets(ref_name)?;
        fs_path::squash_slashes(&mut relative);
        let mut commonlen = None;
        for dir in [&b"refs/heads/"[..], b"refs/tags/", b"refs/remotes/"] {
            let n = fs_path::common_dirlen(dir, relative.as_bytes());
            if n == dir.len() {
                commonlen = Some(n);
                break;
            }
        }
        let Some(commonlen) = commonlen else {
            return Ok(());
        };
        relative.truncate(commonlen);
        let mut base = GitStr::new();
        match prefix {
            Some(prefix) => base.join3(b'/', self.commonpath(), prefix, relative.as_bytes())?,
            None => base.joinpath(self.commonpath(), relative.as_bytes())?,
        }
        // SAFETY: no repository (`NULL`), as C passes.
        if !unsafe {
            crate::sys::path::is_valid_bytes(
                core::ptr::null_mut::<core::ffi::c_void>(),
                base.as_bytes(),
                0,
                fs_path::GIT_FS_PATH_REJECT_LONG_PATHS,
            )
        } {
            return Err(git_error_set!(
                ErrorClass::FILESYSTEM,
                "path too long: '{}'",
                Bytes(base.as_bytes())
            ));
        }
        let rest = ref_name.get(commonlen..).unwrap_or_default();
        match futils::rmdir_r(
            rest,
            Some(base.as_bytes()),
            GIT_RMDIR_EMPTY_PARENTS | GIT_RMDIR_SKIP_ROOT,
        ) {
            Err(ErrorCode::NOT_FOUND) => Ok(()),
            r => r,
        }
    }

    /// `loose_delete`: unlink the loose file of `name` (in the common
    /// directory, as C).
    fn loose_delete(&self, name: &[u8]) -> Result<()> {
        let mut path = GitStr::new();
        path.joinpath(self.commonpath(), name)?;
        match crate::util::posix::unlink(&cpath(&path)) {
            Ok(()) => Ok(()),
            Err(e) if e.0 == libc::ENOENT => Err(ErrorCode::NOT_FOUND),
            Err(_) => Err(ErrorCode::GENERIC),
        }
    }

    /// `packed_delete`: remove `name` from `packed-refs` (rewriting it) if
    /// it is there.
    fn packed_delete(&self, name: &[u8]) -> Result<()> {
        self.packed_reload()?;
        let found = {
            let mut g = self.refcache.write();
            match g.index_of(name) {
                Some(pos) => {
                    g.remove(pos).map_err(|e| e.raw_code())?;
                    true
                }
                None => false,
            }
        };
        if found {
            self.packed_write()?;
        }
        Ok(())
    }

    /// `refdb_fs_backend__delete_tail`.
    fn delete_tail(
        &self,
        file: LockFile,
        name: &[u8],
        old_id: Option<&Oid>,
        old_target: Option<&[u8]>,
    ) -> Result<()> {
        let r = self.delete_tail_inner(name, old_id, old_target);
        drop(file);
        r?;
        self.prune_refs(name, Some(b""))
    }

    fn delete_tail_inner(
        &self,
        name: &[u8],
        old_id: Option<&Oid>,
        old_target: Option<&[u8]>,
    ) -> Result<()> {
        let cmp = self.cmp_old_ref(name, old_id, old_target)?;
        if cmp != 0 {
            let _ = git_error_set!(ErrorClass::REFERENCE, "old reference value does not match");
            return Err(ErrorCode::MODIFIED);
        }
        let packed_deleted = match self.packed_delete(name) {
            Ok(()) => true,
            Err(ErrorCode::NOT_FOUND) => false,
            Err(e) => return Err(e),
        };
        match self.loose_delete(name) {
            Ok(()) => Ok(()),
            Err(ErrorCode::NOT_FOUND) => {
                if packed_deleted {
                    Ok(())
                } else {
                    Err(ref_error_notfound(name))
                }
            }
            Err(e) => Err(e),
        }
    }

    /// `refdb_fs_backend__delete`.
    fn delete(&self, name: &[u8], old_id: Option<&Oid>, old_target: Option<&[u8]>) -> Result<()> {
        let file = self.loose_lock(name)?;
        self.reflog_delete(name)?;
        self.delete_tail(file, name, old_id, old_target)
    }

    /// `refdb_fs_backend__rename`.
    fn rename(
        &self,
        old_name: &[u8],
        new_name: &[u8],
        force: bool,
        who: *const git_signature,
        message: Option<&CStr>,
    ) -> Result<RefBox> {
        self.reference_path_available(new_name, Some(old_name), force)?;
        let old = self.lookup(old_name)?;
        let file = self.loose_lock(old.view().name())?;
        let new = old.rename(new_name)?;
        self.delete_tail(file, old_name, None, None)?;
        let mut file = self.loose_lock(new_name)?;
        // Rename the reflog; it is fine if there is none. (C ignores the
        // other failures of the rename too.)
        match self.reflog_rename(old_name, new_name) {
            Ok(()) | Err(ErrorCode::NOT_FOUND) => {
                let target = new.view().target().copied();
                self.reflog_append(new.view(), target.as_ref(), None, who, message)?;
            }
            Err(_) => {}
        }
        loose_commit(&mut file, new.view())?;
        Ok(new)
    }

    /// `packed_find_peel`: cache the peel of a packed tag.
    fn packed_find_peel(&self, r: &mut PackRef) -> Result<()> {
        if r.flags & (PACKREF_HAS_PEEL | PACKREF_CANNOT_PEEL) != 0 {
            return Ok(());
        }
        // C: `if (git_object_lookup(…) < 0) return -1;`: the code is folded.
        let obj = p::ObjBox::lookup(self.repo, &r.oid, -2).map_err(|_| ErrorCode::GENERIC)?;
        if let Some(target) = obj.tag_target_id() {
            r.peel = target;
            r.flags |= PACKREF_HAS_PEEL;
        }
        Ok(())
    }

    /// `packed_remove_loose`: after `packed-refs` is written, remove the
    /// loose files that were packed, unless they changed meanwhile.
    fn packed_remove_loose(&self, entries: &[(Vec<u8>, PackRef)]) -> Result<()> {
        for (name, r) in entries {
            if r.flags & PACKREF_WAS_LOOSE == 0 {
                continue;
            }
            let lock = match self.loose_lock(name) {
                Ok(l) => l,
                Err(ErrorCode::EXISTS | ErrorCode::NOT_FOUND) => continue,
                Err(e) => {
                    let _ = git_error_set!(
                        ErrorClass::REFERENCE,
                        "failed to lock loose reference '{}'",
                        Bytes(name)
                    );
                    return Err(e);
                }
            };
            let original = lock.original_path().unwrap_or(c"").to_owned();
            let mut content = GitStr::new();
            if let Err(ErrorCode::NOT_FOUND) = futils::readbuffer(&mut content, &original) {
                continue;
            }
            if prefixcmp(content.as_bytes(), GIT_SYMREF) == 0 {
                continue;
            }
            let Ok(current) =
                refs::loose_parse_oid(original.to_bytes(), content.as_bytes(), self.oid_type)
            else {
                continue;
            };
            if current != r.oid {
                continue;
            }
            let _ = crate::util::posix::unlink(&original);
            drop(lock);
        }
        Ok(())
    }

    /// `packed_write`: write the packed references to `packed-refs`, then
    /// remove the loose files that were packed.
    fn packed_write(&self) -> Result<()> {
        {
            let mut st = self.packed.lock().unwrap_or_else(|e| e.into_inner());
            packed_map_free(&mut st);
        }
        let mut g = self.refcache.write();
        let flags = if self.should_fsync() {
            GIT_FILEBUF_FSYNC
        } else {
            0
        };
        let mut file = LockFile::open(self.refcache.path(), flags, GIT_PACKEDREFS_FILE_MODE)?;
        let mut header = GIT_PACKEDREFS_HEADER.to_vec();
        header.push(b'\n');
        file.printf(&header)?;
        let n = g.len();
        // The packed loose references, for `packed_remove_loose` (which C
        // finds by walking every entry for `PACKREF_WAS_LOOSE`).
        let mut entries: Vec<(Vec<u8>, PackRef)> = Vec::new();
        let mut line = Vec::with_capacity(256);
        for i in 0..n {
            let Some((_, item)) = g.entry(i) else {
                return Err(errors::assert_failed("ref"));
            };
            let mut item = *item;
            if item.flags & (PACKREF_HAS_PEEL | PACKREF_CANNOT_PEEL) == 0 {
                self.packed_find_peel(&mut item)?;
                let name = g.entry(i).map(|(n, _)| n.to_vec()).unwrap_or_default();
                if let Some(stored) = g.get_mut(&name) {
                    *stored = item;
                }
            }
            let Some((name, _)) = g.entry(i) else {
                return Err(errors::assert_failed("ref"));
            };
            packed_write_ref(&mut file, &mut line, name, &item)?;
            if item.flags & PACKREF_WAS_LOOSE != 0 {
                entries.try_reserve(1).map_err(|_| errors::oom())?;
                entries.push((name.to_vec(), item));
            }
        }
        file.commit()?;
        self.packed_remove_loose(&entries)?;
        g.updated();
        Ok(())
    }

    /// `loose_lookup_to_packfile`: add the loose reference `name` to the
    /// packed references.
    fn loose_lookup_to_packfile(&self, name: &[u8]) -> Result<()> {
        let mut path = GitStr::new();
        let mut file = GitStr::new();
        let read = path
            .joinpath(self.gitpath(), name)
            .and_then(|()| futils::readbuffer(&mut file, &cpath(&path)));
        if read.is_err() {
            errors::clear();
            return Ok(());
        }
        if prefixcmp(file.as_bytes(), GIT_SYMREF) == 0 {
            return Ok(());
        }
        let oid = refs::loose_parse_oid(name, file.as_bytes(), self.oid_type)?;
        let mut g = self.refcache.write();
        let item = g
            .upsert_with(name, PackRef::default)
            .map_err(|e| e.raw_code())?;
        item.oid = oid;
        item.flags = PACKREF_WAS_LOOSE;
        Ok(())
    }

    /// `packed_loadloose`: add every loose reference to the packed ones.
    fn packed_loadloose(&self) -> Result<()> {
        let mut path = GitStr::new();
        path.joinpath(self.gitpath(), GIT_REFS_DIR)?;
        let flags = self.direach_flags();
        let gitlen = self.gitpath().len();
        let mut cb = |p: &mut GitStr| -> c_int { self.dirent_loose_load(p, flags, gitlen) };
        fs_path::direach(&mut path, flags, &mut cb)
    }

    /// `_dirent_loose_load`.
    fn dirent_loose_load(&self, full: &mut GitStr, flags: u32, gitlen: usize) -> c_int {
        if suffixcmp(full.as_bytes(), b".lock") == 0 {
            return 0;
        }
        if fs_path::isdir(&cpath(full)) {
            let mut cb = |p: &mut GitStr| -> c_int { self.dirent_loose_load(p, flags, gitlen) };
            return match fs_path::direach(full, flags, &mut cb) {
                Ok(()) => 0,
                Err(ErrorCode::NOT_FOUND) => {
                    errors::clear();
                    0
                }
                Err(e) => e.raw(),
            };
        }
        let name = full.as_bytes().get(gitlen..).unwrap_or_default().to_vec();
        match self.loose_lookup_to_packfile(&name) {
            Ok(()) => 0,
            Err(e) => e.raw(),
        }
    }

    /// `refdb_fs_backend__compress`.
    fn compress(&self) -> Result<()> {
        self.packed_reload()?;
        self.packed_loadloose()?;
        self.packed_write()
    }

    /// `refdb_fs_backend__init`.
    fn init(&self, head_target: Option<&CStr>, mode: u32, flags: u32) -> c_int {
        let mut path = GitStr::new();
        if flags & GIT_REFDB_BACKEND_INIT_IS_WORKTREE == 0 {
            let dmode = match mode {
                SHARED_UMASK => 0o777,
                SHARED_GROUP => 0o775 | libc::S_ISGID,
                SHARED_ALL => 0o777 | libc::S_ISGID,
                m => m,
            };
            for dir in [GIT_REFS_HEADS_DIR, GIT_REFS_TAGS_DIR] {
                if let Err(e) = path.joinpath(self.gitpath(), dir) {
                    return e.raw();
                }
                // C assigns `git_futils_mkdir(…) < 0` to `error`: a failure
                // returns 1 (see DEFECTS.md).
                if futils::mkdir(path.as_bytes(), dmode, 0).is_err() {
                    return 1;
                }
            }
        }
        let Some(head_target) = head_target else {
            return 0;
        };
        if let Err(e) = path.joinpath(self.gitpath(), GIT_HEAD_REF) {
            return e.raw();
        }
        let cp = cpath(&path);
        if flags & GIT_REFDB_BACKEND_INIT_FORCE_HEAD == 0 && fs_path::exists(&cp) {
            return 0;
        }
        let mut content = GIT_SYMREF.to_vec();
        content.extend_from_slice(head_target.to_bytes());
        content.push(b'\n');
        match futils::writebuffer(&content, &cp, 0, mode) {
            Ok(()) => 0,
            Err(e) => e.raw(),
        }
    }

    /// `refdb_reflog_fs__has_log`.
    fn has_log(&self, name: &[u8]) -> bool {
        match self.reflog_path(name) {
            Ok(p) => fs_path::isfile(&cpath(&p)),
            Err(_) => false,
        }
    }

    /// `refdb_reflog_fs__ensure_log`.
    fn ensure_log(&self, name: &[u8]) -> Result<()> {
        let p = self.reflog_path(name)?;
        create_new_reflog_file(&p)
    }

    /// `refdb_reflog_fs__read`.
    fn reflog_read(&self, name: &CStr) -> Result<NonNull<GitReflog>> {
        let log = reflog::Reflog::alloc(name, self.oid_type)?;
        let path = self.reflog_path(name.to_bytes())?;
        let mut file = GitStr::new();
        match futils::readbuffer(&mut file, &cpath(&path)) {
            Ok(()) => {}
            Err(ErrorCode::NOT_FOUND) => create_new_reflog_file(&path)?,
            Err(e) => return Err(e),
        }
        let mut log = log;
        log.parse(file.as_bytes())?;
        Ok(log.into_raw())
    }

    /// `lock_reflog`.
    fn lock_reflog(&self, refname: &[u8]) -> Result<LockFile> {
        // SAFETY: as in `loose_lock`.
        let valid = unsafe {
            crate::sys::path::is_valid_bytes(
                self.repo.as_ptr(),
                refname,
                0,
                fs_path::GIT_FS_PATH_REJECT_FILESYSTEM_DEFAULTS,
            )
        };
        if !valid {
            let _ = git_error_set!(
                ErrorClass::INVALID,
                "invalid reference name '{}'",
                Bytes(refname)
            );
            return Err(ErrorCode::INVALID_SPEC);
        }
        let path = self.reflog_path(refname)?;
        if !fs_path::isfile(&cpath(&path)) {
            return Err(git_error_set!(
                ErrorClass::INVALID,
                "log file for reference '{}' doesn't exist",
                Bytes(refname)
            ));
        }
        LockFile::open(path.as_bytes(), 0, GIT_REFLOG_FILE_MODE)
    }

    /// `refdb_reflog_fs__write`.
    ///
    /// # Safety
    ///
    /// `log` is a live reflog (from any backend, or `transaction.c`'s
    /// pool), not changed during the call.
    unsafe fn reflog_write(&self, log: *mut GitReflog) -> Result<()> {
        // SAFETY: per the contract.
        let name = unsafe { reflog::name_of(log) };
        let Ok(mut file) = self.lock_reflog(name.to_bytes()) else {
            return Err(ErrorCode::GENERIC);
        };
        let mut buf = GitStr::new();
        // SAFETY: per the contract.
        for e in unsafe { reflog::entries_of(log) } {
            // SAFETY: the reflog's entries are live entries.
            let e = unsafe { &*e };
            // SAFETY: an entry's message is NULL or a string.
            let msg = unsafe { crate::util::export::cstr(e.msg) }.map(CStr::to_bytes);
            if reflog::serialize_entry(&mut buf, &e.oid_old, &e.oid_cur, e.committer, msg).is_err()
            {
                // C jumps to its cleanup with `error` still 0.
                return Ok(());
            }
            file.write(buf.as_bytes())?;
        }
        file.commit()
    }

    /// `refdb_reflog_fs__rename`.
    fn reflog_rename(&self, old_name: &[u8], new_name: &[u8]) -> Result<()> {
        let mut normalized = GitStr::new();
        refs::normalize_name(
            Some(&mut normalized),
            new_name,
            refs::GIT_REFERENCE_FORMAT_ALLOW_ONELEVEL,
        )?;
        let mut temp = GitStr::new();
        temp.joinpath(
            self.repo.gitdir().as_deref().unwrap_or_default(),
            GIT_REFLOG_DIR,
        )?;
        let mut old_path = GitStr::new();
        old_path.joinpath(temp.as_bytes(), old_name)?;
        let mut new_path = GitStr::new();
        new_path.joinpath(temp.as_bytes(), normalized.as_bytes())?;
        if !fs_path::exists(&cpath(&old_path)) {
            return Err(ErrorCode::NOT_FOUND);
        }
        temp.joinpath_self(b"temp_reflog")?;
        let base = temp.as_bytes().to_vec();
        let fd = futils::mktmp(&mut temp, &base, GIT_REFLOG_FILE_MODE)
            .map_err(|_| ErrorCode::GENERIC)?;
        // SAFETY: our descriptor, closed once.
        let _ = unsafe { crate::util::posix::close(fd) };
        let rename_error = || {
            git_error_set!(
                ErrorClass::OS,
                "failed to rename reflog for {}",
                Bytes(new_name)
            )
        };
        if crate::util::posix::rename(&cpath(&old_path), &cpath(&temp)).is_err() {
            return Err(rename_error());
        }
        if fs_path::isdir(&cpath(&new_path))
            && futils::rmdir_r(new_path.as_bytes(), None, GIT_RMDIR_SKIP_NONEMPTY).is_err()
        {
            return Err(ErrorCode::GENERIC);
        }
        if futils::mkpath2file(new_path.as_bytes(), GIT_REFLOG_DIR_MODE).is_err() {
            return Err(ErrorCode::GENERIC);
        }
        if crate::util::posix::rename(&cpath(&temp), &cpath(&new_path)).is_err() {
            return Err(rename_error());
        }
        Ok(())
    }

    /// `refdb_reflog_fs__delete`.
    fn reflog_delete(&self, name: &[u8]) -> Result<()> {
        let path = self.reflog_path(name)?;
        let cp = cpath(&path);
        if !fs_path::isfile(&cp) {
            return Ok(());
        }
        if crate::util::posix::unlink(&cp).is_err() {
            return Err(ErrorCode::GENERIC);
        }
        self.prune_refs(name, Some(GIT_REFLOG_DIR))
    }

    /// `refdb_fs_backend__iterator`.
    fn iterator(self: &Arc<Self>, glob: Option<&CStr>) -> Result<FsIter> {
        let mut it = FsIter {
            backend: Arc::clone(self),
            glob: None,
            pool: GitPool::with_item_size(1)?,
            loose: Vec::new(),
            packed: Vec::new(),
            loose_pos: 0,
            packed_pos: 0,
        };
        if let Some(g) = glob {
            it.glob = Some(g.to_owned());
        }
        self.iter_load_loose_paths(&mut it)?;
        self.packed_reload()?;
        let g = self.refcache.read();
        it.packed.try_reserve(g.len()).map_err(|_| errors::oom())?;
        for (name, item) in g.iter() {
            let n = it.pool.strndup(name)?;
            it.packed.push((n, *item));
        }
        Ok(it)
    }

    /// `iter_load_loose_paths`.
    fn iter_load_loose_paths(&self, it: &mut FsIter) -> Result<()> {
        let Some(common) = self.commonpath.as_deref() else {
            return Ok(());
        };
        let glob = it.glob.clone();
        let prefix = glob
            .as_deref()
            .and_then(|g| glob_prefix(g.to_bytes()))
            .unwrap_or(GIT_REFS_DIR)
            .to_vec();
        self.iter_load_paths(it, &prefix, common, false)?;
        if self.repo.is_worktree() {
            self.iter_load_paths(it, &prefix, self.gitpath(), true)?;
        }
        Ok(())
    }

    /// `iter_load_paths`.
    fn iter_load_paths(
        &self,
        it: &mut FsIter,
        prefix: &[u8],
        root: &[u8],
        worktree: bool,
    ) -> Result<()> {
        let mut path = root.to_vec();
        path.extend_from_slice(prefix);
        let mut fsit = match FsIterator::new(&cbytes(&path), self.iterator_flags(), self.oid_type) {
            Ok(f) => f,
            Err(ErrorCode::NOT_FOUND) if worktree || it.glob.is_some() => return Ok(()),
            Err(e) => return Err(e),
        };
        let is_worktree = !worktree && self.repo.is_worktree();
        let mut name = Vec::with_capacity(prefix.len() + 64);
        while let Some(entry) = fsit.advance() {
            name.clear();
            name.extend_from_slice(prefix);
            name.extend_from_slice(entry.to_bytes());
            if worktree {
                if !refs::is_per_worktree_ref(&name) {
                    continue;
                }
            } else if is_worktree && refs::is_per_worktree_ref(&name) {
                continue;
            }
            if suffixcmp(&name, b".lock") == 0 {
                continue;
            }
            if let Some(g) = &it.glob
                && !wildmatch(g.to_bytes(), &name, WildmatchFlags::default()).is_match()
            {
                continue;
            }
            let dup = it.pool.strndup(&name)?;
            it.loose.try_reserve(1).map_err(|_| errors::oom())?;
            it.loose.push(dup);
        }
        Ok(())
    }
}

/// `packed_map_free`.
fn packed_map_free(st: &mut PackedMap) {
    if st.map.take().is_some() {
        futils::filestamp_set(&mut st.stamp, None);
    }
}

/// `ref_error_notfound`.
fn ref_error_notfound(name: &[u8]) -> ErrorCode {
    let _ = git_error_set!(
        ErrorClass::REFERENCE,
        "reference '{}' not found",
        Bytes(name)
    );
    ErrorCode::NOT_FOUND
}

/// `git_oid_clear(&id, type)`.
pub fn oid_clear(oid_type: c_int) -> Oid {
    Oid::from_raw_parts(oid_type as u8, [0; GIT_OID_MAX_SIZE])
}

/// `loose_commit`: write `r` into its lock file and commit it.
fn loose_commit(file: &mut LockFile, r: RefView<'_>) -> Result<()> {
    let mut line = Vec::with_capacity(80);
    match r.raw_kind() {
        GIT_REFERENCE_DIRECT => {
            line.extend_from_slice(r.oid_field().hex().as_bytes());
        }
        GIT_REFERENCE_SYMBOLIC => {
            line.extend_from_slice(GIT_SYMREF);
            line.extend_from_slice(r.symbolic_target().unwrap_or_default());
        }
        _ => return Err(errors::assert_failed("0")),
    }
    line.push(b'\n');
    // C ignores the printf's result: a failure surfaces in the commit.
    let _ = file.printf(&line);
    file.commit()
}

/// `packed_write_ref`.
fn packed_write_ref(
    file: &mut LockFile,
    line: &mut Vec<u8>,
    name: &[u8],
    r: &PackRef,
) -> Result<()> {
    line.clear();
    line.extend_from_slice(r.oid.hex().as_bytes());
    line.push(b' ');
    line.extend_from_slice(name);
    line.push(b'\n');
    if r.flags & PACKREF_HAS_PEEL != 0 {
        line.push(b'^');
        line.extend_from_slice(r.peel.hex().as_bytes());
        line.push(b'\n');
    }
    file.printf(line)
}

/// `create_new_reflog_file`.
fn create_new_reflog_file(path: &GitStr) -> Result<()> {
    futils::mkpath2file(path.as_bytes(), GIT_REFLOG_DIR_MODE)?;
    let fd = crate::util::posix::open(
        &cpath(path),
        libc::O_WRONLY | libc::O_CREAT,
        GIT_REFLOG_FILE_MODE,
    )
    .map_err(|_| ErrorCode::GENERIC)?;
    // SAFETY: our descriptor, closed once.
    unsafe { crate::util::posix::close(fd) }.map_err(|_| ErrorCode::GENERIC)
}

/// `setup_namespace(repo, in)`: `in`, or the root of the namespace under
/// it (made if needed); `None` (C's NULL) on failure.
fn setup_namespace(repo: RepoPtr, dir: &[u8]) -> Option<Vec<u8>> {
    let mut path = GitStr::new();
    path.puts(dir).ok()?;
    let Some(ns) = repo.namespace() else {
        return Some(path.as_bytes().to_vec());
    };
    path.puts(&namespace_path(&ns)).ok()?;
    let mut opts = futils::MkdirOpts::default();
    futils::mkdir_relative(
        path.as_bytes(),
        Some(dir),
        0o777,
        futils::GIT_MKDIR_PATH,
        &mut opts,
    )
    .ok()?;
    path.rtruncate_at_char(b'/');
    path.putc(b'/').ok()?;
    Some(path.as_bytes().to_vec())
}

/// `git_refdb_backend_fs`: a filesystem backend for `repo`, as a table C
/// owns (and frees through its `free` slot).
pub fn backend_fs(repo: RepoPtr) -> Result<*mut git_refdb_backend> {
    let gitpath = match repo.gitdir() {
        Some(d) => Some(setup_namespace(repo, &d).ok_or(ErrorCode::GENERIC)?),
        None => None,
    };
    let commonpath = match repo.commondir() {
        Some(d) => Some(setup_namespace(repo, &d).ok_or(ErrorCode::GENERIC)?),
        None => None,
    };
    let mut packed = GitStr::new();
    packed.joinpath(
        commonpath.as_deref().unwrap_or_default(),
        GIT_PACKEDREFS_FILE,
    )?;
    let refcache = SortedCache::new(Some(packed.as_bytes())).map_err(|e| e.raw_code())?;
    let inner = FsInner {
        repo,
        gitpath,
        commonpath,
        oid_type: repo.oid_type(),
        sorted: AtomicBool::new(false),
        peeling_mode: AtomicU8::new(PEELING_NONE),
        refcache,
        packed: Mutex::new(PackedMap::default()),
    };
    let table = CTable::<RefdbBackendTable, FsBackend>::new(FsBackend(Arc::new(inner)));
    Ok(table.into_c().as_ptr())
}

// ---------------------------------------------------------------------------
// The iterator
// ---------------------------------------------------------------------------

/// The state behind a filesystem backend's iterator (`refdb_fs_iter`).
pub struct FsIter {
    backend: Arc<FsInner>,
    glob: Option<CString>,
    /// The names handed out (`git_pool`, as C): valid until the iterator
    /// is freed.
    pool: GitPool,
    loose: Vec<NonNull<c_char>>,
    /// The packed references at the iterator's creation, in name order.
    packed: Vec<(NonNull<c_char>, PackRef)>,
    loose_pos: usize,
    packed_pos: usize,
}

// SAFETY: the names are blocks of the iterator's own pool, read-only
// strings; nothing ties them to a thread.
unsafe impl Send for FsIter {}
// SAFETY: as above; shared access reads only.
unsafe impl Sync for FsIter {}

/// A NUL-terminated name of the pool.
fn pool_str<'a>(p: NonNull<c_char>) -> &'a CStr {
    // SAFETY: a string the iterator's pool holds until it is dropped,
    // which outlives every use (the iterator's `&mut self`).
    unsafe { CStr::from_ptr(p.as_ptr()) }
}

impl FsIter {
    fn shadow(&mut self, name: &[u8]) {
        if let Ok(i) = self
            .packed
            .binary_search_by(|(n, _)| pool_str(*n).to_bytes().cmp(name))
        {
            self.packed[i].1.flags |= PACKREF_SHADOWED;
        }
    }

    fn glob_matches(&self, name: &[u8]) -> bool {
        self.glob
            .as_ref()
            .is_none_or(|g| wildmatch(g.to_bytes(), name, WildmatchFlags::default()).is_match())
    }

    /// `refdb_fs_backend__iterator_next`.
    fn next(&mut self) -> Result<RefBox> {
        while self.loose_pos < self.loose.len() {
            let path = pool_str(self.loose[self.loose_pos]).to_bytes();
            self.loose_pos += 1;
            if let Ok(Some(r)) = self.backend.loose_lookup(path, true) {
                self.shadow(path);
                return Ok(r);
            }
            errors::clear();
        }
        while self.packed_pos < self.packed.len() {
            let (n, r) = self.packed[self.packed_pos];
            self.packed_pos += 1;
            let name = pool_str(n).to_bytes();
            if r.flags & PACKREF_SHADOWED != 0 || !self.glob_matches(name) {
                continue;
            }
            return RefBox::new_direct(name, &r.oid, Some(&r.peel));
        }
        Err(ErrorCode::ITER_OVER)
    }

    /// `refdb_fs_backend__iterator_next_name`.
    fn next_name(&mut self) -> Result<NonNull<c_char>> {
        while self.loose_pos < self.loose.len() {
            let p = self.loose[self.loose_pos];
            let path = pool_str(p).to_bytes();
            self.loose_pos += 1;
            if let Ok(None) = self.backend.loose_lookup(path, false) {
                self.shadow(path);
                return Ok(p);
            }
            errors::clear();
        }
        while self.packed_pos < self.packed.len() {
            let (n, r) = self.packed[self.packed_pos];
            self.packed_pos += 1;
            if r.flags & PACKREF_SHADOWED != 0 || !self.glob_matches(pool_str(n).to_bytes()) {
                continue;
            }
            return Ok(n);
        }
        Err(ErrorCode::ITER_OVER)
    }
}

// ---------------------------------------------------------------------------
// The tables
// ---------------------------------------------------------------------------

fn busy() -> c_int {
    let _ = git_error_set!(
        ErrorClass::REFERENCE,
        "the reference database is being freed"
    );
    -1
}

fn panicked() -> c_int {
    errors::set_panic_error();
    -1
}

fn st(r: Result<()>) -> c_int {
    crate::util::export::status(r)
}

/// The name argument of a slot.
///
/// # Safety
///
/// `p` is a NUL-terminated string (C's contract for the slot).
unsafe fn name<'a>(p: *const c_char) -> &'a CStr {
    // SAFETY: per the contract (C dereferences NULL).
    unsafe { CStr::from_ptr(p) }
}

/// An optional string argument of a slot.
///
/// # Safety
///
/// `p` is NULL or a NUL-terminated string.
unsafe fn opt<'a>(p: *const c_char) -> Option<&'a CStr> {
    // SAFETY: per the contract.
    unsafe { crate::util::export::cstr(p) }
}

/// The reference argument of a slot.
///
/// # Safety
///
/// `p` is a live reference for the call.
unsafe fn refarg<'a>(p: *const GitReference) -> RefView<'a> {
    // SAFETY: per the contract (C dereferences NULL).
    unsafe { RefView::from_raw(p) }
}

/// An optional oid argument of a slot.
///
/// # Safety
///
/// `p` is NULL or a valid oid for the call.
unsafe fn oidarg<'a>(p: *const Oid) -> Option<&'a Oid> {
    // SAFETY: per the contract.
    unsafe { p.as_ref() }
}

ffibox::impl_cvtable! {
    // SAFETY: libgit2 passes each slot of a `git_refdb_backend` its
    // backend in the argument marked `@` (sys/refdb_backend.h), and calls
    // only the slots set here, `free` once; the slots C may leave NULL are
    // all set. The slots' other arguments keep the header's contract.
    unsafe impl CVtable<FsBackend> for RefdbBackendTable {}
    base = git_refdb_backend { version: GIT_REFDB_BACKEND_VERSION, ..Default::default() };
    fn init(@backend, head_target: *const c_char, mode: libc::mode_t, flags: u32) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        // SAFETY: `head_target` is NULL or a string.
        |s: &FsBackend| s.0.init(unsafe { opt(head_target) }, mode, flags),
    }
    fn exists(out: *mut c_int, @backend, ref_name: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &FsBackend| {
            // SAFETY: `out` is writable (the slot's contract).
            unsafe { *out = 0 };
            // SAFETY: `ref_name` is a string.
            match s.0.exists(unsafe { name(ref_name) }.to_bytes()) {
                Ok(e) => {
                    // SAFETY: as above.
                    unsafe { *out = c_int::from(e) };
                    0
                }
                Err(e) => e.raw(),
            }
        },
    }
    fn lookup(out: *mut *mut GitReference, @backend, ref_name: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        // SAFETY: `ref_name` is a string.
        |s: &FsBackend| match s.0.lookup(unsafe { name(ref_name) }.to_bytes()) {
            Ok(r) => {
                // SAFETY: `out` is writable.
                unsafe { *out = r.into_raw() };
                0
            }
            Err(e) => e.raw(),
        },
    }
    fn iterator(out: *mut *mut git_reference_iterator, @backend, glob: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        // SAFETY: `glob` is NULL or a string.
        |s: &FsBackend| match s.0.iterator(unsafe { opt(glob) }) {
            Ok(it) => {
                let t = CTable::<RefIteratorTable, FsIter>::new(it);
                // SAFETY: `out` is writable.
                unsafe { *out = t.into_c().as_ptr() };
                0
            }
            Err(e) => e.raw(),
        },
    }
    fn write(
        @backend,
        r: *const GitReference,
        force: c_int,
        who: *const git_signature,
        message: *const c_char,
        old_id: *const Oid,
        old_target: *const c_char,
    ) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &FsBackend| {
            // SAFETY: `r` is a live reference.
            let v = unsafe { refarg(r) };
            // SAFETY: NULL or a string.
            let msg = unsafe { opt(message) };
            // SAFETY: NULL or a valid oid.
            let old = unsafe { oidarg(old_id) };
            // SAFETY: NULL or a string.
            let old_t = unsafe { opt(old_target) };
            st(s.0.write(v, force != 0, who, msg, old, old_t.map(CStr::to_bytes)))
        },
    }
    fn rename(
        out: *mut *mut GitReference,
        @backend,
        old_name: *const c_char,
        new_name: *const c_char,
        force: c_int,
        who: *const git_signature,
        message: *const c_char,
    ) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &FsBackend| {
            // SAFETY: the names are strings, `message` NULL or one.
            let (o, n, msg) = (unsafe { name(old_name) }, unsafe { name(new_name) }, unsafe { opt(message) });
            let r = s.0.rename(o.to_bytes(), n.to_bytes(), force != 0, who, msg);
            match r {
                Ok(r) => {
                    if !out.is_null() {
                        // SAFETY: `out` is NULL or writable.
                        unsafe { *out = r.into_raw() };
                    }
                    0
                }
                Err(e) => e.raw(),
            }
        },
    }
    fn del(@backend, ref_name: *const c_char, old_id: *const Oid, old_target: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &FsBackend| {
            // SAFETY: `ref_name` is a string, the others NULL or valid.
            let (n, old, old_t) = (unsafe { name(ref_name) }, unsafe { oidarg(old_id) }, unsafe { opt(old_target) });
            st(s.0.delete(n.to_bytes(), old, old_t.map(CStr::to_bytes)))
        },
    }
    fn compress(@backend) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &FsBackend| st(s.0.compress()),
    }
    fn has_log(@backend, ref_name: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &FsBackend| {
            if ref_name.is_null() {
                let _ = errors::assert_arg_failed("name");
                return -1;
            }
            // SAFETY: a string.
            c_int::from(s.0.has_log(unsafe { name(ref_name) }.to_bytes()))
        },
    }
    fn ensure_log(@backend, ref_name: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &FsBackend| {
            if ref_name.is_null() {
                let _ = errors::assert_arg_failed("_backend && name");
                return -1;
            }
            // SAFETY: a string.
            st(s.0.ensure_log(unsafe { name(ref_name) }.to_bytes()))
        },
    }
    fn reflog_read(out: *mut *mut GitReflog, @backend, ref_name: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &FsBackend| {
            if out.is_null() {
                let _ = errors::assert_arg_failed("out");
                return -1;
            }
            if ref_name.is_null() {
                let _ = errors::assert_arg_failed("name");
                return -1;
            }
            // SAFETY: a string.
            match s.0.reflog_read(unsafe { name(ref_name) }) {
                Ok(log) => {
                    // SAFETY: `out` is writable.
                    unsafe { *out = log.as_ptr() };
                    0
                }
                Err(e) => e.raw(),
            }
        },
    }
    fn reflog_write(@backend, log: *mut GitReflog) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &FsBackend| {
            if log.is_null() {
                let _ = errors::assert_arg_failed("reflog");
                return -1;
            }
            // SAFETY: a live reflog for the call.
            st(unsafe { s.0.reflog_write(log) })
        },
    }
    fn reflog_rename(@backend, old_name: *const c_char, new_name: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &FsBackend| {
            if old_name.is_null() {
                let _ = errors::assert_arg_failed("old_name");
                return -1;
            }
            if new_name.is_null() {
                let _ = errors::assert_arg_failed("new_name");
                return -1;
            }
            // SAFETY: strings.
            let (o, n) = (unsafe { name(old_name) }, unsafe { name(new_name) });
            st(s.0.reflog_rename(o.to_bytes(), n.to_bytes()))
        },
    }
    fn reflog_delete(@backend, ref_name: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &FsBackend| {
            if ref_name.is_null() {
                let _ = errors::assert_arg_failed("name");
                return -1;
            }
            // SAFETY: a string.
            st(s.0.reflog_delete(unsafe { name(ref_name) }.to_bytes()))
        },
    }
    fn lock(out: *mut *mut c_void, @backend, refname: *const c_char) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        // SAFETY: a string.
        |s: &FsBackend| match s.0.loose_lock(unsafe { name(refname) }.to_bytes()) {
            Ok(l) => {
                // SAFETY: `out` is writable.
                unsafe { *out = l.into_payload() };
                0
            }
            Err(e) => e.raw(),
        },
    }
    fn unlock(
        @backend,
        payload: *mut c_void,
        success: c_int,
        update_reflog: c_int,
        r: *const GitReference,
        sig: *const git_signature,
        message: *const c_char,
    ) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &FsBackend| {
            // SAFETY: the payload our `lock` made, which C hands back once.
            let lock = unsafe { LockFile::from_payload(payload) };
            let res = if success == 2 {
                // SAFETY: `r` is a live reference.
                let v = unsafe { refarg(r) };
                s.0.delete_tail(lock, v.name(), None, None)
            } else if success != 0 {
                // SAFETY: `r` is a live reference; `message` NULL or a
                // string.
                let (v, msg) = (unsafe { refarg(r) }, unsafe { opt(message) });
                s.0.write_tail(v, lock, update_reflog != 0, None, None, sig, msg)
            } else {
                drop(lock);
                Ok(())
            };
            st(res)
        },
    }
    release free;
}

ffibox::impl_cvtable! {
    // SAFETY: libgit2 passes each slot of a `git_reference_iterator` the
    // iterator, as declared, and calls `next`, `next_name` and `free`
    // (once), all set here.
    unsafe impl CVtable<FsIter> for RefIteratorTable {}
    base = git_reference_iterator::default();
    fn next(out: *mut *mut GitReference, @iter) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &mut FsIter| match s.next() {
            Ok(r) => {
                // SAFETY: `out` is writable.
                unsafe { *out = r.into_raw() };
                0
            }
            Err(e) => e.raw(),
        },
    }
    fn next_name(out: *mut *const c_char, @iter) -> c_int {
        on panic: panicked(),
        on busy: busy(),
        |s: &mut FsIter| match s.next_name() {
            Ok(n) => {
                // SAFETY: `out` is writable; the name lives in the
                // iterator's pool until the iterator is freed.
                unsafe { *out = n.as_ptr() };
                0
            }
            Err(e) => e.raw(),
        },
    }
    release free;
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `refdb_fs.c`.
pub mod ffi {
    use super::*;
    use crate::util::export::barrier;

    /// `git_refdb_backend_fs`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `repo` a live repository that outlives the
    /// backend.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_refdb_backend_fs(
        out: *mut *mut git_refdb_backend,
        repo: *mut git_repository,
    ) -> c_int {
        barrier(|| {
            let Some(r) = NonNull::new(repo) else {
                return errors::assert_arg_failed("repository").raw();
            };
            // SAFETY: per the contract.
            match backend_fs(unsafe { RepoPtr::new(r) }) {
                Ok(b) => {
                    // SAFETY: `out` is writable.
                    unsafe { *out = b };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_reference__lookup_loose`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; the strings are NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_reference__lookup_loose(
        out: *mut *mut GitReference,
        ref_dir: *const c_char,
        ref_name: *const c_char,
        oid_type: c_int,
    ) -> c_int {
        barrier(|| {
            if !out.is_null() {
                // SAFETY: writable.
                unsafe { *out = core::ptr::null_mut() };
            }
            // SAFETY: strings (C dereferences NULL).
            let dir = unsafe { crate::util::export::cstr_bytes(ref_dir) };
            // SAFETY: as above.
            let n = unsafe { crate::util::export::cstr_bytes(ref_name) };
            match refs::lookup_loose(dir, n, oid_type, !out.is_null()) {
                Ok(Some(r)) => {
                    // SAFETY: writable (`want` is `!out.is_null()`).
                    unsafe { *out = r.into_raw() };
                    0
                }
                Ok(None) => 0,
                Err(e) => e.raw(),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";

    type Parsed = (Vec<(Vec<u8>, PackRef)>, PackedTraits);

    fn parse(data: &[u8]) -> core::result::Result<Parsed, Corrupted> {
        let mut items: Vec<(Vec<u8>, PackRef)> = Vec::new();
        let mut traits = None;
        parse_packed(
            data,
            OidType::Sha1,
            |t| traits = Some(t),
            |name, f| {
                let i = match items.iter().position(|(n, _)| n == name) {
                    Some(i) => i,
                    None => {
                        items.push((name.to_vec(), PackRef::default()));
                        items.len() - 1
                    }
                };
                f(&mut items[i].1);
                Ok(())
            },
        )?;
        Ok((items, traits.unwrap()))
    }

    #[test]
    fn header_traits() {
        let (t, off) = packed_header(b"# pack-refs with: peeled fully-peeled sorted \nx");
        assert_eq!(t.peeling_mode, PEELING_FULL);
        assert_eq!(t.sorted, Some(true));
        assert_eq!(off, Some(46));
        let (t, off) = packed_header(b"# pack-refs with: peeled \n");
        assert_eq!(t.peeling_mode, PEELING_STANDARD);
        assert_eq!(t.sorted, Some(false));
        assert_eq!(off, Some(26));
        let (t, off) = packed_header(b"# pack-refs with: sorted ");
        assert_eq!(off, None);
        assert_eq!(t.sorted, None);
        let (t, off) = packed_header(b"abc");
        assert_eq!(
            (t.peeling_mode, t.sorted, off),
            (PEELING_NONE, None, Some(0))
        );
        let (_, off) = packed_header(b"# pack");
        assert_eq!(off, Some(0));
    }

    #[test]
    fn parse_records() {
        let data = format!(
            "# pack-refs with: peeled \n{A} refs/heads/master\n{B} refs/tags/v1\n^{A}\n{B} refs/tags/v2\r\n"
        );
        let (items, t) = parse(data.as_bytes()).unwrap();
        assert_eq!(t.peeling_mode, PEELING_STANDARD);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].0, b"refs/heads/master");
        assert_eq!(items[0].1.flags, 0);
        assert_eq!(items[1].1.flags, PACKREF_HAS_PEEL);
        assert_eq!(items[1].1.peel.hex().as_str(), A);
        assert_eq!(items[2].0, b"refs/tags/v2");
        assert_eq!(items[2].1.flags, PACKREF_CANNOT_PEEL);
    }

    #[test]
    fn parse_failures() {
        assert!(parse(b"1234 refs/heads/x\n").is_err());
        assert!(parse(format!("{A}xrefs/heads/x\n").as_bytes()).is_err());
        assert!(parse(format!("{A} refs/heads/x").as_bytes()).is_err());
        assert!(parse(format!("{A} refs/he\0ads/x\n").as_bytes()).is_err());
        assert!(parse(b"# comment without newline").is_err());
        assert!(parse(format!("{A} r\n^12").as_bytes()).is_err());
        // A peel line at the end without a newline is fine.
        let (items, _) = parse(format!("{A} r\n^{B}").as_bytes()).unwrap();
        assert_eq!(items[0].1.flags, PACKREF_HAS_PEEL);
        assert!(parse(b"").unwrap().0.is_empty());
    }

    #[test]
    fn search_sorted() {
        let mut data = String::from("# pack-refs with: peeled fully-peeled sorted \n");
        let names = [
            "refs/heads/a",
            "refs/heads/b",
            "refs/heads/b/c",
            "refs/tags/t",
            "refs/tags/u",
        ];
        for (i, n) in names.iter().enumerate() {
            data.push_str(&format!("{A} {n}\n"));
            if i == 3 {
                data.push_str(&format!("^{B}\n"));
            }
        }
        let d = data.as_bytes();
        for n in names {
            assert!(
                matches!(
                    packed_search(d, n.as_bytes(), OidType::Sha1),
                    Ok(Searched::Found(..))
                ),
                "{n}"
            );
        }
        match packed_search(d, b"refs/tags/t", OidType::Sha1) {
            Ok(Searched::Found(_, Some(p))) => assert_eq!(p.hex().as_str(), B),
            r => panic!("{r:?}"),
        }
        for n in [
            "refs/heads/0",
            "refs/heads/ab",
            "refs/heads/b/",
            "refs/z",
            "a",
        ] {
            assert_eq!(
                packed_search(d, n.as_bytes(), OidType::Sha1),
                Ok(Searched::NotFound),
                "{n}"
            );
        }
        assert_eq!(
            packed_search(b"", b"x", OidType::Sha1),
            Ok(Searched::NotFound)
        );
        assert_eq!(packed_search(b"#abc", b"x", OidType::Sha1), Err(Corrupted));
    }

    #[test]
    fn availability() {
        assert!(!ref_is_available(None, b"refs/heads/a/b", b"refs/heads/a"));
        assert!(!ref_is_available(None, b"refs/heads/a", b"refs/heads/a/b"));
        assert!(ref_is_available(None, b"refs/heads/ab", b"refs/heads/a"));
        assert!(ref_is_available(
            Some(b"refs/heads/a"),
            b"refs/heads/a/b",
            b"refs/heads/a"
        ));
        assert!(ref_is_available(None, b"refs/heads/a", b"refs/heads/a"));
    }

    #[test]
    fn collisions_match_the_scan() {
        let mut state = 12345u64;
        let mut rnd = |n: u64| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) % n
        };
        let alphabet = b"ab/";
        for _ in 0..300 {
            let mut names: Vec<Vec<u8>> = (0..rnd(12))
                .map(|_| (0..rnd(6)).map(|_| alphabet[rnd(3) as usize]).collect())
                .collect();
            names.sort();
            names.dedup();
            for _ in 0..20 {
                let new: Vec<u8> = (0..rnd(6)).map(|_| alphabet[rnd(3) as usize]).collect();
                let old = if rnd(2) == 0 && !names.is_empty() {
                    Some(names[rnd(names.len() as u64) as usize].clone())
                } else {
                    None
                };
                let scan = names
                    .iter()
                    .any(|n| !ref_is_available(old.as_deref(), &new, n));
                let fast = path_collides(
                    &new,
                    old.as_deref(),
                    names.len(),
                    |i| names[i].as_slice(),
                    |n| names.binary_search_by(|x| x.as_slice().cmp(n)).is_ok(),
                );
                assert_eq!(scan, fast, "{names:?} {new:?} {old:?}");
            }
        }
    }

    #[test]
    fn prefixes() {
        assert_eq!(glob_prefix(b"refs/heads/*"), Some(&b"refs/heads/"[..]));
        assert_eq!(glob_prefix(b"refs/*/x"), Some(&b"refs/"[..]));
        assert_eq!(glob_prefix(b"refs/heads/master"), Some(&b"refs/heads/"[..]));
        assert_eq!(glob_prefix(b"*"), None);
        assert_eq!(glob_prefix(b"HEAD"), None);
        assert_eq!(namespace_path(b"foo"), b"refs/namespaces/foo/refs");
        assert_eq!(
            namespace_path(b"a/b"),
            b"refs/namespaces/a/refs/namespaces/b/refs"
        );
    }
}
