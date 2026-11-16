//! The index's operations that call foreign code (C, callbacks): each
//! borrows the index only between those calls (see the [index
//! module](super)). These are the bodies of the exports and of the safe
//! API.

use core::ffi::{CStr, c_char, c_int, c_uint};
use core::ptr::{self, NonNull};

use super::foreign::{self, Diff, Paths, Pathspec, Repo};
use super::ondisk;
use super::{
    EntryPtr, GIT_FILEMODE_BLOB, GIT_FILEMODE_BLOB_EXECUTABLE, GIT_FILEMODE_COMMIT,
    GIT_FILEMODE_LINK, GIT_INDEX_ADD_CHECK_PATHSPEC, GIT_INDEX_ADD_DISABLE_PATHSPEC_MATCH,
    GIT_INDEX_ADD_FORCE, GIT_INDEX_CAPABILITY_FROM_OWNER, GIT_INDEX_CAPABILITY_IGNORE_CASE,
    GIT_INDEX_CAPABILITY_NO_FILEMODE, GIT_INDEX_CAPABILITY_NO_SYMLINKS, GIT_INDEX_ENTRY_UPTODATE,
    GIT_INDEX_FILE_MODE, GitIndex, GitIndexEntry, GitIndexWriter, IndexPtr, as_entry, create_mode,
    entry_alloc, entry_cmp, entry_cpy, entry_cpy_nocache, entry_free, epath, find_in_entries,
    init_from_stat, s_isgitlink, sort_entries,
};
use crate::oid::OidType;
use crate::sys::diff as d;
use crate::sys::path::{GIT_PATH_REJECT_INDEX_DEFAULTS, GIT_PATH_REJECT_WORKDIR_DEFAULTS};
use crate::tree_cache;
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::filebuf::{GIT_FILEBUF_HASH_SHA1, GIT_FILEBUF_HASH_SHA256};
use crate::util::str::GitStr;
use crate::util::vector::GitVector;
use crate::git_error_set;

/// A `git_index_matched_path_cb`, as the operations call it: the path and
/// the matched pathspec (NULL for none), both C strings valid for the call;
/// 0 to go on, > 0 to skip, < 0 to stop.
pub type MatchedPathCb<'a> = &'a mut dyn FnMut(*const c_char, *const c_char) -> c_int;

/// `create_index_error`.
fn index_error(msg: &str) -> ErrorCode {
    errors::set_str(ErrorClass::INDEX, msg)
}

/// The index's owner.
fn owner(ix: IndexPtr) -> Option<Repo> {
    // SAFETY: an index's owner is NULL or the repository that owns it,
    // which outlives it (it frees the index).
    unsafe { Repo::new(ix.owner()) }
}

/// A NUL-terminated copy of `b` (paths from C have no NUL within).
fn c_copy(b: &[u8]) -> Result<alloc_cstring::CString> {
    alloc_cstring::CString::new(b)
}

mod alloc_cstring {
    //! A small owned C string (fallible allocation).
    use core::ffi::CStr;

    use crate::util::errors::{self, Result};

    /// An owned, NUL-terminated byte string.
    pub struct CString(Vec<u8>);

    impl CString {
        /// A copy of `b` with a NUL appended (`b` should have none).
        pub fn new(b: &[u8]) -> Result<CString> {
            let mut v = Vec::new();
            v.try_reserve_exact(b.len() + 1).map_err(|_| errors::oom())?;
            v.extend_from_slice(b);
            v.push(0);
            Ok(CString(v))
        }

        /// As a `CStr` (up to its first NUL).
        pub fn as_c_str(&self) -> &CStr {
            CStr::from_bytes_until_nul(&self.0).unwrap_or(c"")
        }

        /// The bytes without the NUL.
        pub fn as_bytes(&self) -> &[u8] {
            &self.0[..self.0.len() - 1]
        }
    }
}

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

/// `index_entry_create`: check `path` (always rejecting `.git` and
/// traversal; with `from_workdir`, the working directory's rules too),
/// then allocate the entry.
pub(crate) fn entry_create(repo: Option<Repo>, path: &[u8], st_mode: Option<u32>, from_workdir: bool) -> Result<EntryPtr> {
    let mut flags = GIT_PATH_REJECT_INDEX_DEFAULTS;
    if from_workdir {
        flags |= GIT_PATH_REJECT_WORKDIR_DEFAULTS;
    }
    // C: `uint16_t mode = st->st_mode`
    let mode = st_mode.map_or(0, |m| m as u16);
    if !foreign::path_is_valid(repo, path, mode, flags) {
        return Err(git_error_set!(ErrorClass::INDEX, "invalid path: '{}'", Bytes(path)));
    }
    entry_alloc(path)
}

/// `index_entry_dup`.
pub(crate) fn entry_dup(repo: Option<Repo>, src: &GitIndexEntry, src_path: &[u8]) -> Result<EntryPtr> {
    let e = entry_create(repo, src_path, None, false)?;
    // SAFETY: a fresh entry of ours.
    unsafe { entry_cpy(e, src) };
    Ok(e)
}

/// The path of a caller's entry (`entry->path`, a C string).
///
/// # Safety
///
/// `e.path` is NULL or a string live for `'a`.
pub(crate) unsafe fn src_path<'a>(e: &GitIndexEntry) -> &'a [u8] {
    // SAFETY: the caller's contract.
    unsafe { crate::util::export::cstr_bytes(e.path) }
}

/// `index_insert`: insert `entry` (which it takes: freed on failure, and
/// when it replaces an existing one); the entry now in the index.
pub(crate) fn insert(
    ix: IndexPtr,
    entry: EntryPtr,
    replace: bool,
    trust_path: bool,
    trust_mode: bool,
    trust_id: bool,
) -> Result<EntryPtr> {
    let (mode, id) = {
        // SAFETY: no foreign call in this block.
        let index = unsafe { ix.get() };
        // SAFETY: `entry` is ours, not in the index.
        let e = unsafe { &mut (*entry.as_ptr()).entry };
        // SAFETY: as above.
        let pathlen = unsafe { (*entry.as_ptr()).pathlen };
        super::adjust_namemask(e, pathlen);
        // This entry is now up-to-date and should not be checked for
        // raciness
        e.flags_extended |= GIT_INDEX_ENTRY_UPTODATE;
        let stage = e.stage();
        // SAFETY: as above.
        let path = unsafe { epath(entry) };
        let (_existing, _pos, best) = index.existing_and_best(path, stage);
        // SAFETY: as above (`e` was re-borrowed by `epath`).
        let e = unsafe { &mut (*entry.as_ptr()).entry };
        e.mode = if trust_mode { create_mode(e.mode) } else { index.merge_mode(best, e.mode) };
        let (mode, id) = (e.mode, e.id);
        if !trust_path {
            index.canonicalize_directory_path(entry, best);
        }
        (mode, id)
    };
    // Ensure that the given id exists (unless it's a submodule)
    if !trust_id
        && let Some(repo) = owner(ix)
        && (mode & GIT_FILEMODE_COMMIT) != GIT_FILEMODE_COMMIT
        && !foreign::object_is_valid(repo, &id, foreign::type_from_filemode(mode))
    {
        // SAFETY: ours, not in the index.
        unsafe { entry_free(entry) };
        return Err(ErrorCode::GENERIC);
    }
    // SAFETY: no foreign call in `insert_finish`.
    unsafe { ix.get() }.insert_finish(entry, replace, trust_path)
}

/// `git_index__conflict_to_reuc`: move the conflict at `path` to the REUC.
pub(crate) fn conflict_to_reuc(index: &mut GitIndex, path: &[u8]) -> Result<()> {
    let conflict = index.conflict_get(path)?;
    let mut modes = [0 as c_int; 3];
    let mut oids = [None; 3];
    for i in 0..3 {
        if let Some(e) = conflict[i] {
            // SAFETY: a live entry of the index.
            let ent = unsafe { &(*e.as_ptr()).entry };
            modes[i] = ent.mode as c_int;
            oids[i] = Some(ent.id);
        }
    }
    index.reuc_add(path, modes, oids)?;
    index.conflict_remove(Some(path))
}

/// The `GIT_ENOTFOUND` of a result, as C's `ret != GIT_ENOTFOUND` tests.
fn not_found(r: &Result<()>) -> bool {
    matches!(r, Err(e) if *e == ErrorCode::NOT_FOUND)
}

/// `git_index_add`.
pub(crate) fn add(ix: IndexPtr, source: &GitIndexEntry, path: &[u8]) -> Result<()> {
    if !valid_filemode(source.mode) {
        return Err(git_error_set!(ErrorClass::INDEX, "invalid entry mode"));
    }
    let entry = entry_dup(owner(ix), source, path)?;
    let entry = insert(ix, entry, true, true, true, false)?;
    // SAFETY: no foreign call; `entry` is a live entry of the index.
    let index = unsafe { ix.get() };
    // SAFETY: as above.
    unsafe { tree_cache::invalidate_path(index.tree, epath(entry)) };
    Ok(())
}

/// `is_file_or_link`.
fn is_file_or_link(m: u32) -> bool {
    m == GIT_FILEMODE_BLOB || m == GIT_FILEMODE_BLOB_EXECUTABLE || m == GIT_FILEMODE_LINK
}

/// `valid_filemode`.
fn valid_filemode(m: u32) -> bool {
    is_file_or_link(m) || m == GIT_FILEMODE_COMMIT
}

/// `git_index_add_from_buffer`.
/// `buf` is `Err(len)` for a buffer too large to look at.
pub(crate) fn add_from_buffer(
    ix: IndexPtr,
    source: &GitIndexEntry,
    path: &[u8],
    buf: core::result::Result<&[u8], usize>,
) -> Result<()> {
    let Some(repo) = owner(ix) else {
        return Err(index_error(
            "could not initialize index entry. Index is not backed up by an existing repository.",
        ));
    };
    if !is_file_or_link(source.mode) {
        return Err(git_error_set!(ErrorClass::INDEX, "invalid filemode"));
    }
    let buf = match buf {
        Ok(b) if b.len() <= u32::MAX as usize => b,
        _ => return Err(git_error_set!(ErrorClass::INDEX, "buffer is too large")),
    };
    let entry = entry_dup(Some(repo), source, path)?;
    let id = match foreign::blob_from_buffer(repo, buf) {
        Ok(id) => id,
        Err(e) => {
            // SAFETY: ours, not in the index.
            unsafe { entry_free(entry) };
            return Err(e);
        }
    };
    // SAFETY: ours, not in the index.
    unsafe {
        (*entry.as_ptr()).entry.id = id;
        (*entry.as_ptr()).entry.file_size = buf.len() as u32;
    }
    let entry = insert(ix, entry, true, true, true, true)?;
    // SAFETY: no foreign call below; `entry` is live until the conflict
    // removal (which may remove it, when the source had a stage: the path
    // is copied first, where C read the freed entry).
    let index = unsafe { ix.get() };
    // SAFETY: as above.
    let p = c_copy(unsafe { epath(entry) })?;
    let r = conflict_to_reuc(index, p.as_bytes());
    if r.is_err() && !not_found(&r) {
        return r;
    }
    // SAFETY: the tree invariant.
    unsafe { tree_cache::invalidate_path(index.tree, p.as_bytes()) };
    Ok(())
}

/// `index_entry_init`: the entry for the working-directory file at
/// `rel_path`, its blob written.
fn entry_init(ix: IndexPtr, rel_path: &CStr) -> Result<EntryPtr> {
    let Some(repo) = owner(ix) else {
        return Err(index_error(
            "could not initialize index entry. Index is not backed up by an existing repository.",
        ));
    };
    foreign::ensure_not_bare(repo, "create blob from file")?;
    let full = foreign::workdir_path(repo, rel_path).map_err(|_| ErrorCode::GENERIC)?;
    let st = crate::util::fs_path::lstat(full.as_c_str().unwrap_or(c""));
    drop(full);
    let mut st = st?;
    let entry = entry_create(Some(repo), rel_path.to_bytes(), Some(st.st_mode), true)
        .map_err(|_| ErrorCode::GENERIC)?;
    // write the blob to disk and get the oid and stat info
    let id = match foreign::blob_from_workdir(repo, rel_path, &mut st) {
        Ok(id) => id,
        Err(e) => {
            // SAFETY: ours.
            unsafe { entry_free(entry) };
            return Err(e);
        }
    };
    // SAFETY: no foreign call; `entry` is ours.
    let trust = !unsafe { ix.get() }.distrust_filemode();
    // SAFETY: as above.
    let e = unsafe { &mut (*entry.as_ptr()).entry };
    e.id = id;
    init_from_stat(e, &st, trust);
    Ok(entry)
}

/// `add_repo_as_submodule`.
fn add_repo_as_submodule(ix: IndexPtr, path: &CStr) -> Result<EntryPtr> {
    let repo = owner(ix).ok_or(ErrorCode::GENERIC)?;
    let abspath = foreign::workdir_path(repo, path)?;
    let abs = abspath.as_c_str().unwrap_or(c"");
    let st = match crate::util::posix::stat(abs) {
        Ok(st) => st,
        Err(_) => return Err(git_error_set!(ErrorClass::OS, "failed to stat repository dir")),
    };
    let entry = entry_create(Some(repo), path.to_bytes(), Some(st.st_mode), true)
        .map_err(|_| ErrorCode::GENERIC)?;
    // SAFETY: no foreign call; `entry` is ours.
    let trust = !unsafe { ix.get() }.distrust_filemode();
    // SAFETY: as above.
    init_from_stat(unsafe { &mut (*entry.as_ptr()).entry }, &st, trust);
    let id = match foreign::head_target_of(abs) {
        Ok(id) => id,
        Err(e) => {
            // SAFETY: ours (C leaked it).
            unsafe { entry_free(entry) };
            return Err(e);
        }
    };
    // SAFETY: as above.
    unsafe {
        (*entry.as_ptr()).entry.id = id;
        (*entry.as_ptr()).entry.mode = GIT_FILEMODE_COMMIT;
    }
    Ok(entry)
}

/// `git_index_add_bypath`.
pub(crate) fn add_bypath(ix: IndexPtr, path: &CStr) -> Result<()> {
    let mut ret = entry_init(ix, path).and_then(|e| insert(ix, e, true, false, false, true));
    // If we were given a directory, let's see if it's a submodule
    if let Err(e) = ret
        && e != ErrorCode::DIRECTORY
    {
        return Err(e);
    }
    if let Err(e) = ret {
        debug_assert_eq!(e, ErrorCode::DIRECTORY);
        let saved = errors::save();
        let repo = owner(ix).ok_or(ErrorCode::GENERIC)?;
        match foreign::submodule_lookup(repo, path) {
            Err(e) if e == ErrorCode::NOT_FOUND => {
                errors::restore(saved);
                return Err(ErrorCode::DIRECTORY);
            }
            Err(e) if e == ErrorCode::EXISTS => {
                drop(saved);
                // EEXISTS means that there is a repository at that path,
                // but it's not known as a submodule. We add its HEAD as an
                // entry and don't register it.
                let entry = add_repo_as_submodule(ix, path)?;
                ret = Ok(insert(ix, entry, true, false, false, true)?);
            }
            Err(e) => {
                drop(saved);
                return Err(e);
            }
            Ok(sm) => {
                drop(saved);
                return foreign::submodule_add_to_index(&sm);
            }
        }
    }
    let entry = ret?;
    // SAFETY: no foreign call below; `entry` is a live stage-0 entry of
    // the index (the conflict removal leaves it).
    let index = unsafe { ix.get() };
    // Adding implies conflict was resolved, move conflict entries to REUC
    let r = conflict_to_reuc(index, path.to_bytes());
    if r.is_err() && !not_found(&r) {
        return r;
    }
    // SAFETY: as above.
    unsafe { tree_cache::invalidate_path(index.tree, epath(entry)) };
    Ok(())
}

/// `git_index_remove_bypath`.
pub(crate) fn remove_bypath(index: &mut GitIndex, path: &[u8]) -> Result<()> {
    let r = index.remove(path, 0);
    if r.is_err() && !not_found(&r) {
        return r;
    }
    let r = conflict_to_reuc(index, path);
    if r.is_err() && !not_found(&r) {
        return r;
    }
    if not_found(&r) {
        errors::clear();
    }
    Ok(())
}

/// `git_index__fill`: add copies of `source` entries (no checks of
/// collisions or duplicates).
pub(crate) fn fill(ix: IndexPtr, source: &[*const GitIndexEntry]) -> Result<()> {
    if source.is_empty() {
        return Ok(());
    }
    {
        // SAFETY: no foreign call in this block.
        let index = unsafe { ix.get() };
        index.entries.size_hint(source.len())?;
        index.entries_map.resize((source.len() as f64 * 1.3) as usize)?;
    }
    let repo = owner(ix);
    let mut error = Ok(());
    for &src in source {
        // SAFETY: the caller passes live entries with string paths.
        let (src, path) = unsafe { (&*src, src_path(&*src)) };
        let entry = match entry_dup(repo, src, path) {
            Ok(e) => e,
            Err(e) => {
                error = Err(e);
                break;
            }
        };
        // SAFETY: no foreign call in this block; `entry` is ours.
        let index = unsafe { ix.get() };
        // SAFETY: as above.
        unsafe {
            let e = &mut (*entry.as_ptr()).entry;
            let n = (*entry.as_ptr()).pathlen;
            super::adjust_namemask(e, n);
            e.flags_extended |= GIT_INDEX_ENTRY_UPTODATE;
            e.mode = create_mode(e.mode);
        }
        if let Err(e) = index.entries.insert(entry.as_ptr().cast()) {
            // SAFETY: not inserted; ours (C leaked it).
            unsafe { entry_free(entry) };
            error = Err(e);
            break;
        }
        // SAFETY: the entry is now the index's.
        if let Err(e) = unsafe { index.entries_map.put(entry) } {
            error = Err(e);
            break;
        }
        index.set_dirty(true);
    }
    if error.is_ok() {
        // SAFETY: no foreign call.
        unsafe { ix.get() }.sort();
    }
    error
}

/// `git_index_conflict_add`.
pub(crate) fn conflict_add(ix: IndexPtr, sides: [Option<(&GitIndexEntry, &[u8])>; 3]) -> Result<()> {
    let repo = owner(ix);
    let mut entries: [Option<EntryPtr>; 3] = [None; 3];
    let free_all = |entries: &mut [Option<EntryPtr>; 3]| {
        for e in entries.iter_mut().filter_map(Option::take) {
            // SAFETY: ours, not in the index.
            unsafe { entry_free(e) };
        }
    };
    for i in 0..3 {
        if let Some((src, path)) = sides[i] {
            match entry_dup(repo, src, path) {
                Ok(e) => entries[i] = Some(e),
                Err(err) => {
                    free_all(&mut entries);
                    return Err(err);
                }
            }
        }
    }
    // Validate entries
    for i in 0..3 {
        // SAFETY: ours.
        if let Some(e) = entries[i]
            && !valid_filemode(unsafe { (*e.as_ptr()).entry.mode })
        {
            free_all(&mut entries);
            return Err(git_error_set!(ErrorClass::INDEX, "invalid filemode for stage {} entry", i + 1));
        }
    }
    // Remove existing index entries for each path
    for i in 0..3 {
        let Some(e) = entries[i] else { continue };
        // SAFETY: no foreign call; `e` is ours.
        let r = unsafe { ix.get() }.remove(unsafe { epath(e) }, 0);
        if r.is_err() {
            if !not_found(&r) {
                free_all(&mut entries);
                return r;
            }
            errors::clear();
        }
    }
    // Add the conflict entries
    for i in 0..3 {
        let Some(e) = entries[i].take() else { continue };
        // Make sure stage is correct
        // SAFETY: ours.
        unsafe { (*e.as_ptr()).entry.set_stage(i as c_int + 1) };
        if let Err(err) = insert(ix, e, true, true, true, false) {
            free_all(&mut entries);
            return Err(err);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Opening, reading, writing
// ---------------------------------------------------------------------------

/// `git_index_open_ext`: a new index (reference count 1), read from
/// `path` when given.
pub(crate) fn open_ext(path: Option<&CStr>, oid_type: OidType) -> Result<IndexPtr> {
    let mut index = GitIndex::new(oid_type)?;
    if let Some(p) = path {
        let s = crate::util::alloc::strdup(p);
        if s.is_null() {
            index.dispose_box();
            return Err(ErrorCode::GENERIC);
        }
        index.index_file_path = s;
        // Check if index file is stored on disk already
        if crate::util::fs_path::exists(p) {
            index.set_on_disk(true);
        }
    }
    let raw = NonNull::from(Box::leak(index));
    // SAFETY: a fresh index, live until its last reference goes.
    let ix = unsafe { IndexPtr::new(raw) };
    if path.is_some()
        && let Err(e) = read(ix, true)
    {
        // SAFETY: the index has no reference yet: `git_index_free` of a
        // count of 0 frees it.
        unsafe { ix.decref() };
        return Err(e);
    }
    ix.incref();
    Ok(ix)
}

impl GitIndex {
    /// Free a boxed index that never got a reference.
    fn dispose_box(mut self: Box<Self>) {
        self.dispose();
    }
}

/// `compare_checksum`: 1 if the file's checksum differs from the index's,
/// 0 if not.
fn compare_checksum(path: &CStr, checksum: &[u8]) -> Result<c_int> {
    let fd = crate::util::posix::open(path, libc::O_RDONLY, 0).map_err(|_| ErrorCode::GENERIC)?;
    // SAFETY: `fd` is open; `lseek` has no memory preconditions.
    if unsafe { libc::lseek(fd, -(checksum.len() as libc::off_t), libc::SEEK_END) } < 0 {
        // SAFETY: our descriptor, closed once.
        let _ = unsafe { crate::util::posix::close(fd) };
        return Err(git_error_set!(ErrorClass::OS, "failed to seek to end of file"));
    }
    let mut buf = [0u8; crate::util::hash::MAX_SIZE];
    let r = crate::util::posix::read(fd, &mut buf[..checksum.len()]);
    // SAFETY: our descriptor, closed once.
    let _ = unsafe { crate::util::posix::close(fd) };
    match r {
        Ok(n) if n >= checksum.len() => Ok(c_int::from(buf[..checksum.len()] != *checksum)),
        _ => Err(ErrorCode::GENERIC),
    }
}

/// `git_index_read`. `Ok(1)` is C's (odd) result when the file vanished
/// between its existence check and its `stat`.
pub(crate) fn read(ix: IndexPtr, force: bool) -> Result<c_int> {
    let (path, mut stamp, checksum) = {
        // SAFETY: no foreign call in this block.
        let index = unsafe { ix.get() };
        let Some(p) = index.file_path() else {
            return Err(index_error("failed to read index: The index is in-memory only"));
        };
        let path = c_copy(p.to_bytes())?;
        let n = ondisk_checksum_size(index.oid_kind());
        let mut c = [0u8; crate::util::hash::MAX_SIZE];
        c[..n].copy_from_slice(&index.checksum[..n]);
        (path, index.stamp, (c, n))
    };
    let path_c = path.as_c_str();
    let on_disk = crate::util::fs_path::exists(path_c);
    {
        // SAFETY: no foreign call in this block.
        let index = unsafe { ix.get() };
        index.set_on_disk(on_disk);
        if !on_disk {
            if force {
                index.clear()?;
            }
            index.set_dirty(false);
            return Ok(0);
        }
    }
    let failed = || {
        git_error_set!(
            ErrorClass::INDEX,
            "failed to read index: '{}' no longer exists",
            Bytes(path.as_bytes())
        )
    };
    // C: `(updated = git_futils_filestamp_check(...) < 0) || ...`: the
    // stamp's result itself is lost; only the checksum decides.
    if crate::util::futils::filestamp_check(Some(&mut stamp), path_c) < 0 {
        let _ = failed();
        return Ok(1);
    }
    let updated = match compare_checksum(path_c, &checksum.0[..checksum.1]) {
        Ok(u) => u,
        Err(e) => {
            let _ = failed();
            return Err(e);
        }
    };
    if updated == 0 && !force {
        return Ok(0);
    }
    let mut buffer = GitStr::new();
    crate::util::futils::readbuffer(&mut buffer, path_c)?;

    let (mut st, repo) = {
        // SAFETY: no foreign call in this block.
        let index = unsafe { ix.get() };
        index.tree = ptr::null_mut();
        index.tree_pool.clear();
        index.clear()?;
        (index.take_parse_state(), owner(ix))
    };
    let mut validate =
        |p: &[u8]| foreign::path_is_valid(repo, p, 0, GIT_PATH_REJECT_INDEX_DEFAULTS);
    let r = ondisk::parse(&mut st, buffer.as_bytes(), &mut validate);
    // SAFETY: no foreign call.
    let index = unsafe { ix.get() };
    index.put_parse_state(st);
    r?;
    crate::util::futils::filestamp_set(&mut index.stamp, Some(&stamp));
    index.set_dirty(false);
    Ok(0)
}

/// The checksum size of an id type.
fn ondisk_checksum_size(t: OidType) -> usize {
    t.size()
}

/// `git_index_read_safely`.
pub(crate) fn read_safely(ix: IndexPtr) -> Result<c_int> {
    // SAFETY: no foreign call.
    let dirty = unsafe { ix.get() }.dirty();
    if super::git_index__enforce_unsaved_safety.load(core::sync::atomic::Ordering::Relaxed) && dirty {
        let _ = git_error_set!(
            ErrorClass::INDEX,
            "the index has unsaved changes that would be overwritten by this operation"
        );
        return Err(ErrorCode::INDEX_DIRTY);
    }
    read(ix, false)
}

/// `git_index_entry_newer_than_index`.
pub(crate) fn newer_than_index(entry: &GitIndexEntry, index: &GitIndex) -> bool {
    // If we never read the index, we can't have this race either
    if index.stamp.mtime.tv_sec == 0 {
        return false;
    }
    let sec = index.stamp.mtime.tv_sec as i32;
    if sec < entry.mtime.seconds {
        true
    } else if sec > entry.mtime.seconds {
        false
    } else {
        (index.stamp.mtime.tv_nsec as u32) <= entry.mtime.nanoseconds
    }
}

/// `truncate_racily_clean`: smudge (size 0) the racily clean entries that
/// differ from the working directory.
fn truncate_racily_clean(ix: IndexPtr) -> Result<()> {
    let Some(repo) = owner(ix) else { return Ok(()) };
    if !foreign::has_workdir(repo) {
        return Ok(());
    }
    let flags = d::GIT_DIFF_INCLUDE_TYPECHANGE | d::GIT_DIFF_IGNORE_SUBMODULES | d::GIT_DIFF_DISABLE_PATHSPEC_MATCH;
    let mut paths: Vec<*mut c_char> = Vec::new();
    {
        // SAFETY: no foreign call in this block.
        let index = unsafe { ix.get() };
        for e in index.entries.iter() {
            // SAFETY: a live entry of the index.
            let ent = unsafe { &(*as_entry(e).as_ptr()).entry };
            let racy = !s_isgitlink(ent.mode) && newer_than_index(ent, index);
            if ent.flags_extended & GIT_INDEX_ENTRY_UPTODATE == 0 && racy && paths.try_reserve(1).is_ok() {
                paths.push(ent.path.cast_mut());
            }
        }
    }
    if paths.is_empty() {
        return Ok(());
    }
    // SAFETY: no reference into the index is live; the paths are entries'
    // paths, which the diff (it only reads the index) leaves live.
    let diff = unsafe { Diff::index_to_workdir(repo, ix.as_ptr(), flags, Some((paths.as_mut_ptr(), paths.len()))) }?;
    for i in 0..diff.len() {
        let Some(path) = diff.old_path(i) else { continue };
        // SAFETY: no foreign call in this block.
        let index = unsafe { ix.get() };
        // Ensure that we have a stage 0 for this file (ie, it's not a
        // conflict), otherwise smudging it is quite pointless.
        match index.get_bypath(&path, 0) {
            Some(e) => {
                // SAFETY: a live entry of the index.
                unsafe { (*e.as_ptr()).entry.file_size = 0 };
                index.set_dirty(true);
            }
            None => {
                let _ = git_error_set!(ErrorClass::INDEX, "index does not contain '{}'", Bytes(&path));
            }
        }
    }
    Ok(())
}

/// `git_index_write`.
pub(crate) fn write(ix: IndexPtr) -> Result<()> {
    let _ = truncate_racily_clean(ix);
    let mut writer = GitIndexWriter {
        index: ptr::null_mut(),
        file: crate::util::filebuf::GitFilebuf::new(),
        should_write: 0,
    };
    let mut r = writer_init(&mut writer, ix);
    if r.is_ok() {
        r = writer_commit(&mut writer);
        if r.is_ok() {
            // SAFETY: no foreign call.
            unsafe { ix.get() }.set_dirty(false);
        }
    }
    writer_cleanup(&mut writer);
    r
}

/// `git_indexwriter_init`.
pub(crate) fn writer_init(writer: &mut GitIndexWriter, ix: IndexPtr) -> Result<()> {
    ix.incref();
    writer.index = ix.as_ptr();
    // SAFETY: no foreign call in this block.
    let index = unsafe { ix.get() };
    let filebuf_hash = match index.oid_kind() {
        OidType::Sha1 => GIT_FILEBUF_HASH_SHA1,
        OidType::Sha256 => GIT_FILEBUF_HASH_SHA256,
    };
    let Some(path) = index.file_path() else {
        return Err(index_error("failed to write index: The index is in-memory only"));
    };
    // SAFETY: no deflate level in the flags, so zlib keeps no address of
    // the filebuf (and the writer, C's or `write`'s local, does not move
    // before its cleanup either).
    if let Err(e) = unsafe { writer.file.open(path.to_bytes(), filebuf_hash, GIT_INDEX_FILE_MODE) } {
        if e == ErrorCode::LOCKED {
            let _ = git_error_set!(
                ErrorClass::INDEX,
                "the index is locked; this might be due to a concurrent or crashed process"
            );
        }
        return Err(e);
    }
    writer.should_write = 1;
    Ok(())
}

/// `GIT_CHECKOUT_DONT_WRITE_INDEX`.
const GIT_CHECKOUT_DONT_WRITE_INDEX: c_uint = 1 << 23;

/// `git_indexwriter_init_for_operation`.
pub(crate) fn writer_init_for_operation(writer: &mut GitIndexWriter, repo: Repo, checkout_strategy: &mut c_uint) -> Result<()> {
    let index = foreign::repository_index(repo)?;
    // SAFETY: the repository's index, live while the repository is (and
    // the writer takes a reference).
    writer_init(writer, unsafe { IndexPtr::new(index) })?;
    writer.should_write = c_uint::from(*checkout_strategy & GIT_CHECKOUT_DONT_WRITE_INDEX == 0);
    *checkout_strategy |= GIT_CHECKOUT_DONT_WRITE_INDEX;
    Ok(())
}

/// `git_indexwriter_commit`.
pub(crate) fn writer_commit(writer: &mut GitIndexWriter) -> Result<()> {
    if writer.should_write & 1 == 0 {
        return Ok(());
    }
    let Some(ixp) = NonNull::new(writer.index) else {
        return Err(errors::assert_arg_failed("writer->index"));
    };
    // SAFETY: the writer holds a reference to its index.
    let ix = unsafe { IndexPtr::new(ixp) };
    // SAFETY: no foreign call in this block (the writer's filebuf and the
    // index are distinct objects).
    let index = unsafe { ix.get() };
    index.sort();
    index.sort_reuc();
    let checksum = match ondisk::write_index(index, &mut writer.file) {
        Ok(c) => c,
        Err(e) => {
            writer_cleanup(writer);
            return Err(e);
        }
    };
    writer.file.commit()?;
    let path = index.file_path().map(|p| c_copy(p.to_bytes())).transpose()?;
    let path = path.as_ref().map_or(c"", |p| p.as_c_str());
    if crate::util::futils::filestamp_check(Some(&mut index.stamp), path) < 0 {
        return Err(git_error_set!(ErrorClass::OS, "could not read index timestamp"));
    }
    index.set_dirty(false);
    index.set_on_disk(true);
    let n = index.oid_kind().size();
    index.checksum[..n].copy_from_slice(&checksum[..n]);
    writer.index = ptr::null_mut();
    // SAFETY: the writer's reference, given up.
    unsafe { ix.decref() };
    Ok(())
}

/// `git_indexwriter_cleanup`.
pub(crate) fn writer_cleanup(writer: &mut GitIndexWriter) {
    writer.file.cleanup();
    if let Some(p) = NonNull::new(writer.index) {
        writer.index = ptr::null_mut();
        // SAFETY: the writer's reference, given up.
        unsafe { IndexPtr::new(p).decref() };
    }
}

// ---------------------------------------------------------------------------
// Capabilities, trees
// ---------------------------------------------------------------------------

/// `git_index_set_caps`.
pub(crate) fn set_caps(ix: IndexPtr, caps: c_int) -> Result<()> {
    // SAFETY: no foreign call.
    let old_ignore_case = unsafe { ix.get() }.ignore_case();
    if caps == GIT_INDEX_CAPABILITY_FROM_OWNER {
        let Some(repo) = owner(ix) else {
            return Err(index_error("cannot access repository to set index caps"));
        };
        let icase = foreign::configmap_lookup(repo, foreign::CONFIGMAP_IGNORECASE);
        let filemode = foreign::configmap_lookup(repo, foreign::CONFIGMAP_FILEMODE);
        let symlinks = foreign::configmap_lookup(repo, foreign::CONFIGMAP_SYMLINKS);
        // SAFETY: no foreign call.
        let index = unsafe { ix.get() };
        if let Ok(v) = icase {
            index.set_ignore_case_bit(v != 0);
        }
        if let Ok(v) = filemode {
            index.set_distrust_filemode(v == 0);
        }
        if let Ok(v) = symlinks {
            index.set_no_symlinks(v == 0);
        }
    } else {
        // SAFETY: no foreign call.
        let index = unsafe { ix.get() };
        index.set_ignore_case_bit(caps & GIT_INDEX_CAPABILITY_IGNORE_CASE != 0);
        index.set_distrust_filemode(caps & GIT_INDEX_CAPABILITY_NO_FILEMODE != 0);
        index.set_no_symlinks(caps & GIT_INDEX_CAPABILITY_NO_SYMLINKS != 0);
    }
    // SAFETY: no foreign call.
    let index = unsafe { ix.get() };
    if old_ignore_case != index.ignore_case() {
        let icase = index.ignore_case();
        index.set_ignore_case(icase);
    }
    Ok(())
}

/// `git_index_read_tree`.
pub(crate) fn read_tree(ix: IndexPtr, tree: foreign::TreeRef<'_>) -> Result<()> {
    let icase = {
        // SAFETY: no foreign call in this block.
        let index = unsafe { ix.get() };
        index.tree = ptr::null_mut();
        index.tree_pool.clear();
        index.sort();
        index.ignore_case()
    };
    let repo = owner(ix);
    // `git_vector_set_cmp(&entries, index->entries._cmp)` on an empty,
    // unsorted vector
    let mut entries = GitVector::new();
    entries.set_cmp(Some(super::entry_cmp_fn(icase)));
    let walk = foreign::tree_walk_post(tree, &mut |root, te| {
        // `read_tree_cb`
        if te.attr & 0o170000 == 0o040000 {
            return 0;
        }
        let mut path = GitStr::new();
        if path.joinpath(root, te.name).is_err() {
            return -1;
        }
        let Ok(entry) = entry_create(repo, path.as_bytes(), None, false) else {
            return -1;
        };
        // SAFETY: ours.
        let e = unsafe { &mut (*entry.as_ptr()).entry };
        e.mode = te.attr;
        e.id = te.id;
        {
            // look for corresponding old entry and copy data to new entry
            // SAFETY: no foreign call in this block (the walk is paused
            // in this callback, holding no reference).
            let index = unsafe { ix.get() };
            if let Ok(pos) = find_in_entries(&index.entries, path.as_bytes(), 0, icase)
                && let Some(old) = index.entry_at(pos)
            {
                // SAFETY: a live entry of the index, distinct from ours.
                let old = unsafe { (*old.as_ptr()).entry };
                if e.mode == old.mode && e.id == old.id {
                    // SAFETY: ours.
                    unsafe { entry_cpy(entry, &old) };
                    // SAFETY: as above.
                    unsafe { (*entry.as_ptr()).entry.flags_extended = 0 };
                }
            }
        }
        // SAFETY: ours.
        super::adjust_namemask(unsafe { &mut (*entry.as_ptr()).entry }, path.len());
        if entries.insert(entry.as_ptr().cast()).is_err() {
            // SAFETY: not inserted; ours.
            unsafe { entry_free(entry) };
            return -1;
        }
        0
    });
    let free_entries = |v: &mut GitVector| {
        for e in v.iter() {
            // SAFETY: ours (C leaked them on these failures).
            unsafe { entry_free(as_entry(e)) };
        }
        v.clear();
    };
    if let Err(e) = walk {
        free_entries(&mut entries);
        return Err(e);
    }
    // the C's local map: case-sensitive, whatever the index's case
    let mut map = super::map::EntryMap::default();
    if let Err(e) = map.resize(entries.len()) {
        free_entries(&mut entries);
        return Err(e);
    }
    let mut put_err = None;
    for e in entries.iter() {
        // SAFETY: our entries, which the map holds no longer than them.
        if let Err(err) = unsafe { map.put(as_entry(e)) } {
            put_err = Some(err);
            break;
        }
    }
    if let Some(err) = put_err {
        let _ = git_error_set!(ErrorClass::INDEX, "failed to insert entry into map");
        free_entries(&mut entries);
        return Err(err);
    }
    sort_entries(&mut entries, icase);
    let mut pool = {
        // SAFETY: no foreign call in this block.
        let index = unsafe { ix.get() };
        if let Err(e) = index.clear() {
            free_entries(&mut entries);
            return Err(e);
        }
        entries.set_cmp(index.entries.cmp_fn());
        index.entries.swap(&mut entries);
        index.entries_map.swap(&mut map);
        index.set_dirty(true);
        // the tree cache is read into the index's pool, which is lent out
        // while the tree lookups (C) run
        core::mem::take(&mut index.tree_pool)
    };
    entries.dispose();
    map.dispose();
    // SAFETY: the caller's tree is live (`TreeRef`).
    let (src, root) = unsafe { foreign::CTrees::borrow(tree.as_ptr()) };
    // SAFETY: no foreign call.
    let oid_type = unsafe { ix.get() }.oid_type;
    let r = tree_cache::read_tree(&src, &root, oid_type, &mut pool);
    // SAFETY: no foreign call.
    let index = unsafe { ix.get() };
    index.tree_pool = pool;
    if let Ok(t) = r {
        index.tree = t;
    }
    r.map(|_| ())
}

/// `git_index_read_index`: make `ix`'s entries those of `new_ix`, keeping
/// the cached stat data of the entries that did not change
/// (`git_index_read_iterator` over the new index's iterator, which yields
/// its entries case-sensitively sorted, conflicts included).
pub(crate) fn read_index(ix: IndexPtr, new_ix: IndexPtr) -> Result<()> {
    // The C iterators took a snapshot of each index: a reference, a reader
    // (so that removals are deferred), and a sort.
    let snap = |p: IndexPtr| -> Result<Vec<EntryPtr>> {
        p.incref();
        // SAFETY: no foreign call in this block.
        let index = unsafe { p.get() };
        index.readers.inc();
        index.sort();
        let mut v = Vec::new();
        if v.try_reserve_exact(index.entries.len()).is_err() {
            index.readers.dec();
            // SAFETY: the reference just taken.
            unsafe { p.decref() };
            return Err(errors::oom());
        }
        v.extend(index.entries.iter().map(as_entry));
        // SAFETY: live entries of the index.
        v.sort_by(|&a, &b| unsafe { entry_cmp(a, b, false) }.cmp(&0));
        Ok(v)
    };
    let release = |p: IndexPtr| {
        // SAFETY: no foreign call.
        unsafe { p.get() }.readers.dec();
        // SAFETY: the reference `snap` took.
        unsafe { p.decref() };
    };
    let (hint, old_len, icase) = {
        // SAFETY: no foreign call in this block.
        let (n, o) = unsafe { (new_ix.get().entries.len(), ix.get().entries.len()) };
        // SAFETY: as above.
        (n, o, unsafe { ix.get() }.ignore_case())
    };
    let mut new_entries = GitVector::with_capacity(hint, Some(super::entry_cmp_fn(icase)))?;
    let mut remove_entries: Vec<EntryPtr> = Vec::new();
    remove_entries.try_reserve(old_len.max(8)).map_err(|_| errors::oom())?;
    let mut new_map = super::map::EntryMap::default();
    if hint > 0 {
        new_map.resize(hint)?;
    }
    let new_snap = snap(new_ix)?;
    let old_snap = match snap(ix) {
        Ok(s) => s,
        Err(e) => {
            release(new_ix);
            return Err(e);
        }
    };
    let repo = owner(ix);
    let mut created: Vec<EntryPtr> = Vec::new();
    let (mut oi, mut ni) = (0usize, 0usize);
    let mut result = Ok(());
    loop {
        let old_entry = old_snap.get(oi).copied();
        let new_entry = new_snap.get(ni).copied();
        let diff = match (old_entry, new_entry) {
            // SAFETY: live entries (both indexes have a reader).
            (Some(o), Some(n)) => unsafe { entry_cmp(o, n, false) },
            (None, Some(_)) => 1,
            (Some(_), None) => -1,
            (None, None) => break,
        };
        let mut dup_entry = None;
        let mut add_entry = None;
        let mut remove_entry = None;
        if diff < 0 {
            remove_entry = old_entry;
        } else if diff > 0 {
            dup_entry = new_entry;
        } else {
            let (o, n) = (old_entry.unwrap_or(NonNull::dangling()), new_entry.unwrap_or(NonNull::dangling()));
            // SAFETY: live entries.
            let (oe, ne) = unsafe { ((*o.as_ptr()).entry, (*n.as_ptr()).entry) };
            // Path and stage are equal, if the OID is equal, keep it to
            // keep the stat cache data.
            if oe.id == ne.id && oe.mode == ne.mode {
                add_entry = Some(o);
            } else {
                dup_entry = Some(n);
                remove_entry = Some(o);
            }
        }
        if let Some(dup) = dup_entry {
            // SAFETY: a live entry of the new index (it has a reader).
            let (src, path) = unsafe { ((*dup.as_ptr()).entry, epath(dup).to_vec()) };
            match entry_create(repo, &path, None, false) {
                Ok(e) => {
                    // SAFETY: ours.
                    unsafe { entry_cpy_nocache(e, &src) };
                    // SAFETY: as above.
                    let n = unsafe { (*e.as_ptr()).pathlen };
                    // SAFETY: as above.
                    super::adjust_namemask(unsafe { &mut (*e.as_ptr()).entry }, n);
                    if created.try_reserve(1).is_err() {
                        // SAFETY: ours.
                        unsafe { entry_free(e) };
                        result = Err(errors::oom());
                        break;
                    }
                    created.push(e);
                    add_entry = Some(e);
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
            // invalidate this path in the tree cache if this is new (to
            // invalidate the parent trees)
            if remove_entry.is_none() {
                // SAFETY: no foreign call; the tree invariant.
                let index = unsafe { ix.get() };
                // SAFETY: as above.
                unsafe { tree_cache::invalidate_path(index.tree, &path) };
            }
        }
        let mut error = Ok(());
        if let Some(a) = add_entry {
            error = new_entries.insert(a.as_ptr().cast());
            if error.is_ok() {
                // SAFETY: the entry stays live while the map is used.
                error = unsafe { new_map.put(a) };
            }
        }
        if let Some(r) = remove_entry
            && error.is_ok()
        {
            if remove_entries.try_reserve(1).is_ok() {
                remove_entries.push(r);
            } else {
                error = Err(errors::oom());
            }
        }
        if let Err(e) = error {
            let _ = git_error_set!(ErrorClass::INDEX, "failed to insert entry");
            result = Err(e);
            break;
        }
        if diff <= 0 {
            oi += 1;
        }
        if diff >= 0 {
            ni += 1;
        }
    }
    release(new_ix);
    if let Err(e) = result {
        release(ix);
        for e in created {
            // SAFETY: the entries made here, which the index never got.
            unsafe { entry_free(e) };
        }
        new_map.dispose();
        return Err(e);
    }
    {
        // SAFETY: no foreign call in this block.
        let index = unsafe { ix.get() };
        index.name_clear();
        index.reuc_clear();
        index.entries.swap(&mut new_entries);
        index.entries_map.swap(&mut new_map);
        index.readers.dec();
        for &e in &remove_entries {
            // SAFETY: a removed entry, still live.
            unsafe { tree_cache::invalidate_path(index.tree, epath(e)) };
            if index.readers.get() > 0 {
                // another snapshot may read it (C freed it: see DEFECTS.md)
                if index.deleted.try_reserve(1).is_ok() {
                    index.deleted.push(e);
                }
            } else {
                // SAFETY: the index's entry, in no vector or map now.
                unsafe { entry_free(e) };
            }
        }
        index.clear_uptodate();
        index.set_dirty(true);
    }
    // SAFETY: the reference `snap` took.
    unsafe { ix.decref() };
    new_map.dispose();
    new_entries.dispose();
    Ok(())
}

// ---------------------------------------------------------------------------
// Pathspec operations
// ---------------------------------------------------------------------------

const INDEX_ACTION_UPDATE: c_int = 1;
const INDEX_ACTION_REMOVE: c_int = 2;
const INDEX_ACTION_ADDALL: c_int = 3;

/// `git_repository__ensure_not_bare` of an optional repository (C's
/// `git_repository_is_bare(NULL)` fails its argument check and is taken as
/// bare).
fn ensure_not_bare_opt(repo: Option<Repo>, op: &str) -> Result<()> {
    match repo {
        Some(r) => foreign::ensure_not_bare(r, op),
        None => {
            let _ = errors::assert_arg_failed("repo");
            let _ = git_error_set!(
                ErrorClass::REPOSITORY,
                "cannot {op}. This operation is not allowed against bare repositories."
            );
            Err(ErrorCode::BARE_REPO)
        }
    }
}

/// The status of a callback-driven operation (non-zero stops).
fn code(r: Result<()>) -> c_int {
    match r {
        Ok(()) => 0,
        Err(e) => e.raw(),
    }
}

/// `git_index_add_all`.
pub(crate) fn add_all(ix: IndexPtr, paths: Paths<'_>, flags: c_uint, cb: Option<MatchedPathCb<'_>>) -> c_int {
    let repo = owner(ix);
    if let Err(e) = ensure_not_bare_opt(repo, "index add all") {
        return e.raw();
    }
    let no_fnmatch = flags & GIT_INDEX_ADD_DISABLE_PATHSPEC_MATCH != 0;
    let mut ps = match Pathspec::new(paths) {
        Ok(p) => p,
        Err(e) => return e.raw(),
    };
    // optionally check that pathspec doesn't mention any ignored files
    if flags & GIT_INDEX_ADD_CHECK_PATHSPEC != 0
        && flags & GIT_INDEX_ADD_FORCE == 0
        && let Some(repo) = repo
        && let Err(e) = ps.check_exact_ignores(repo, no_fnmatch)
    {
        return e.raw();
    }
    let error = apply_to_wd_diff(ix, INDEX_ACTION_ADDALL, paths, flags, cb);
    if error != 0 {
        foreign::set_after_callback(error, "git_index_add_all");
    }
    drop(ps);
    error
}

/// `index_apply_to_wd_diff`.
fn apply_to_wd_diff(ix: IndexPtr, action: c_int, paths: Paths<'_>, flags: c_uint, mut cb: Option<MatchedPathCb<'_>>) -> c_int {
    let Some(repo) = owner(ix) else {
        return index_error("cannot run update; the index is not backed up by a repository.").raw();
    };
    // We do the matching ourselves instead of passing the list to diff
    // because we want to tell the callback which one matched, which we do
    // not know if we ask diff to filter for us.
    let ps = match Pathspec::new(paths) {
        Ok(p) => p,
        Err(e) => return e.raw(),
    };
    let mut opts = d::GIT_DIFF_INCLUDE_TYPECHANGE;
    if action == INDEX_ACTION_ADDALL {
        opts |= d::GIT_DIFF_INCLUDE_UNTRACKED | d::GIT_DIFF_RECURSE_UNTRACKED_DIRS;
        if flags == GIT_INDEX_ADD_FORCE {
            opts |= d::GIT_DIFF_INCLUDE_IGNORED | d::GIT_DIFF_RECURSE_IGNORED_DIRS;
        }
    }
    // SAFETY: no reference into the index is live.
    let diff = match unsafe { Diff::index_to_workdir(repo, ix.as_ptr(), opts, None) } {
        Ok(d) => d,
        Err(e) => return e.raw(),
    };
    let mut each = |delta: &foreign::DeltaView<'_>| -> c_int {
        // `apply_each_file`
        let path = delta.old_path;
        // SAFETY: no foreign call.
        let casefold = unsafe { ix.get() }.ignore_case();
        // We only want those which match the pathspecs
        let Some(matched) = ps.matches(path, casefold) else {
            return 0;
        };
        let mut error = 0;
        if let Some(cb) = cb.as_mut() {
            error = cb(path.as_ptr(), matched);
        }
        if error > 0 {
            return 0;
        }
        if error < 0 {
            return error;
        }
        // If the workdir item does not exist, remove it from the index.
        if delta.new_flags & d::GIT_DIFF_FLAG_EXISTS == 0 {
            // SAFETY: no foreign call in `remove_bypath`.
            code(remove_bypath(unsafe { ix.get() }, path.to_bytes()))
        } else {
            code(add_bypath(ix, delta.new_path))
        }
    };
    let error = diff.foreach_file(&mut each);
    drop(diff);
    if error != 0 {
        foreign::set_after_callback(error, "index_apply_to_wd_diff");
    }
    drop(ps);
    error
}

/// `index_apply_to_all`.
fn apply_to_all(ix: IndexPtr, action: c_int, paths: Paths<'_>, mut cb: Option<MatchedPathCb<'_>>) -> c_int {
    let ps = match Pathspec::new(paths) {
        Ok(p) => p,
        Err(e) => return e.raw(),
    };
    // SAFETY: no foreign call.
    unsafe { ix.get() }.sort();
    let mut error = 0;
    let mut i = 0usize;
    loop {
        if error != 0 {
            break;
        }
        // SAFETY: no foreign call in this block.
        let (entry, casefold) = {
            let index = unsafe { ix.get() };
            match index.entry_at(i) {
                Some(e) => (e, index.ignore_case()),
                None => break,
            }
        };
        // SAFETY: a live entry of the index, its path a string.
        let path_c = unsafe { CStr::from_ptr((*entry.as_ptr()).entry.path) };
        // check if path actually matches
        let Some(matched) = ps.matches(path_c, casefold) else {
            i = i.wrapping_add(1);
            continue;
        };
        // index manipulation may alter entry, so don't depend on it (the
        // copy is made before the callback, which may remove the entry: C
        // copied it after, from freed memory then)
        let path = match c_copy(path_c.to_bytes()) {
            Ok(p) => p,
            Err(e) => {
                error = e.raw();
                break;
            }
        };
        // issue notification callback if requested
        if let Some(cb) = cb.as_mut() {
            error = cb(path.as_c_str().as_ptr(), matched);
            if error != 0 {
                if error > 0 {
                    error = 0;
                    i = i.wrapping_add(1);
                    continue;
                }
                break;
            }
        }
        match action {
            INDEX_ACTION_UPDATE => {
                let r = add_bypath(ix, path.as_c_str());
                error = code(r);
                if not_found(&r) {
                    errors::clear();
                    // SAFETY: no foreign call in `remove_bypath`.
                    let r = remove_bypath(unsafe { ix.get() }, path.as_bytes());
                    error = code(r);
                    if error == 0 {
                        // back up foreach if we removed this
                        i = i.wrapping_sub(1);
                    }
                }
            }
            INDEX_ACTION_REMOVE => {
                // SAFETY: no foreign call in `remove_bypath`.
                error = code(remove_bypath(unsafe { ix.get() }, path.as_bytes()));
                if error == 0 {
                    // back up foreach if we removed this
                    i = i.wrapping_sub(1);
                }
            }
            _ => {
                error = git_error_set!(ErrorClass::INVALID, "unknown index action {}", action).raw();
            }
        }
        i = i.wrapping_add(1);
    }
    drop(ps);
    error
}

/// `git_index_remove_all`.
pub(crate) fn remove_all(ix: IndexPtr, paths: Paths<'_>, cb: Option<MatchedPathCb<'_>>) -> c_int {
    let error = apply_to_all(ix, INDEX_ACTION_REMOVE, paths, cb);
    if error != 0 {
        foreign::set_after_callback(error, "git_index_remove_all");
    }
    error
}

/// `git_index_update_all`.
pub(crate) fn update_all(ix: IndexPtr, paths: Paths<'_>, cb: Option<MatchedPathCb<'_>>) -> c_int {
    let error = apply_to_wd_diff(ix, INDEX_ACTION_UPDATE, paths, 0, cb);
    if error != 0 {
        foreign::set_after_callback(error, "git_index_update_all");
    }
    error
}

// ---------------------------------------------------------------------------
// Snapshots
// ---------------------------------------------------------------------------

/// `git_index_snapshot_new`: a copy of the (sorted) entries vector, with a
/// reference and a reader on the index (removals are deferred meanwhile).
pub(crate) fn snapshot_new(ix: IndexPtr) -> Result<GitVector> {
    ix.incref();
    // SAFETY: no foreign call.
    let index = unsafe { ix.get() };
    index.readers.inc();
    index.sort();
    let cmp = index.entries.cmp_fn();
    match index.entries.dup(cmp) {
        Ok(v) => Ok(v),
        Err(e) => {
            snapshot_release(GitVector::new(), ix);
            Err(e)
        }
    }
}

/// `git_index_snapshot_release`.
pub(crate) fn snapshot_release(mut snap: GitVector, ix: IndexPtr) {
    snap.dispose();
    // the reader and reference the snapshot holds
    ix.readers().dec();
    // SAFETY: the snapshot's reference, given up.
    unsafe { ix.decref() };
}

