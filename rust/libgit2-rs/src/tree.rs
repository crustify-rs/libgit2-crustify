//! `src/libgit2/tree.c`: tree objects (zero-copy parse, lookups by name,
//! index, id and path, walks), tree entries, tree builders, writing a tree
//! from the index, and `git_tree_create_updated`.
//!
//! Types (PORTING.md §4.1):
//!
//! - [`GitTree`] is *mirrored*: `object.c` allocates it with
//!   `sizeof(git_tree)`, and `iterator.c` and `indexer.c` walk `entries`.
//! - [`GitTreeEntry`] is *mirrored*: the trees' `git_array` holds entries by
//!   value, and the C that stays (iterator, diff, checkout, merge, index)
//!   reads `attr`, `filename`, `filename_len` and `oid`.
//! - [`GitTreebuilder`] is *opaque*: only `tree.c` touched its fields, so
//!   its body left `tree.h`; it is a Rust struct (allocated with
//!   `git__allocator`, so that allocation failures report as the C's did).
//!
//! # Zero-copy entries
//!
//! A parsed tree's entries point into the bytes they were parsed from: each
//! `filename` is a pointer to the NUL-terminated name inside the object's
//! data. For `git_tree__parse` that data is the odb object's buffer, and the
//! tree keeps a reference to that object (`odb_obj`, released by
//! `git_tree__free`), so the names live exactly as long as the tree. For
//! `git_tree__parse_raw` (`git_object__from_raw`), the caller's buffer must
//! outlive the tree, as `object.h` documents. The safe API expresses this
//! with the lifetime of [`Tree`].
//!
//! Entries that a tree builder, `git_tree_entry_dup` or
//! `git_tree_entry_bypath` return are separate allocations
//! (`alloc_entry`): the entry, its name and room for an id, in one
//! `git__allocator` block, freed with `git_tree_entry_free`.

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr;

use crate::commit::{OBJ_ANY, OBJ_BLOB, OBJ_COMMIT, OBJ_INVALID, OBJ_TREE, code, object_id, repo_odb};
use crate::oid::{Oid, OidType, git_oid_t};
use crate::sys::pending::l_commit::{self as pend, GitObjectHead, git_repository};
use crate::util::alloc;
use crate::util::array::GitArray;
use crate::util::ctype_compat::isspace;
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::hashmap::{CStrKey, GitHashMap, HashMapIter};
use crate::util::str::GitStr;
use crate::{git_assert, git_assert_arg, git_error_set};

pub mod api;

/// `GIT_FILEMODE_TREE`.
pub const FILEMODE_TREE: u32 = 0o040000;
/// `GIT_FILEMODE_BLOB`.
pub const FILEMODE_BLOB: u32 = 0o100644;
/// `GIT_FILEMODE_BLOB_EXECUTABLE`.
pub const FILEMODE_BLOB_EXECUTABLE: u32 = 0o100755;
/// `GIT_FILEMODE_LINK`.
pub const FILEMODE_LINK: u32 = 0o120000;
/// `GIT_FILEMODE_COMMIT`.
pub const FILEMODE_COMMIT: u32 = 0o160000;
/// `GIT_FILEMODE_BLOB_GROUP_WRITABLE`: the obsolete mode kept for
/// compatibility.
pub const FILEMODE_BLOB_GROUP_WRITABLE: u32 = 0o100664;

const DEFAULT_TREE_SIZE: usize = 16;

/// `struct git_tree_entry` (`tree.h`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GitTreeEntry {
    pub attr: u16,
    pub filename_len: u16,
    pub oid: Oid,
    /// The NUL-terminated name, `filename_len` bytes (see the module docs
    /// for what it points into).
    pub filename: *const c_char,
}

impl Default for GitTreeEntry {
    fn default() -> Self {
        GitTreeEntry {
            attr: 0,
            filename_len: 0,
            oid: Oid::default(),
            filename: ptr::null(),
        }
    }
}

impl GitTreeEntry {
    /// The name's `filename_len` bytes.
    ///
    /// # Safety
    ///
    /// `filename` addresses `filename_len` readable bytes that outlive `'a`
    /// and that nothing writes meanwhile (an entry of a live tree or
    /// builder).
    #[inline]
    pub unsafe fn name<'a>(&self) -> &'a [u8] {
        if self.filename.is_null() {
            return &[];
        }
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(self.filename.cast(), usize::from(self.filename_len)) }
    }

    /// `git_tree_entry__is_tree`.
    #[inline]
    pub fn is_tree(&self) -> bool {
        is_tree_mode(self.attr)
    }
}

/// `struct git_tree` (`tree.h`).
#[repr(C)]
pub struct GitTree {
    pub object: GitObjectHead,
    pub odb_obj: *mut pend::git_odb_object,
    pub entries: GitArray<GitTreeEntry>,
}

/// `S_ISDIR(m) && !S_ISGITLINK(m)`.
#[inline]
pub const fn is_tree_mode(attr: u16) -> bool {
    (attr as u32 & 0o170000) == FILEMODE_TREE
}

/// `valid_filemode`.
pub const fn valid_filemode(filemode: u32) -> bool {
    matches!(
        filemode,
        FILEMODE_TREE | FILEMODE_BLOB | FILEMODE_BLOB_EXECUTABLE | FILEMODE_LINK | FILEMODE_COMMIT
    )
}

/// `normalize_filemode`: the canonical mode for a (possibly legacy) raw one.
pub const fn normalize_filemode(filemode: u32) -> u32 {
    let kind = filemode & !0o777;
    // Tree bits set, but it's not a commit
    if kind == FILEMODE_TREE {
        return FILEMODE_TREE;
    }
    // If any of the x bits are set
    if filemode & 0o100 != 0 {
        return FILEMODE_BLOB_EXECUTABLE;
    }
    // 16XXXX means commit
    if kind == FILEMODE_COMMIT {
        return FILEMODE_COMMIT;
    }
    // 12XXXX means symlink
    if kind == FILEMODE_LINK {
        return FILEMODE_LINK;
    }
    // Otherwise, return a blob
    FILEMODE_BLOB
}

/// `git_tree_entry_type` of a raw mode.
pub const fn entry_type(attr: u16) -> c_int {
    let m = attr as u32 & 0o170000;
    if m == FILEMODE_COMMIT {
        OBJ_COMMIT
    } else if m == FILEMODE_TREE {
        OBJ_TREE
    } else {
        OBJ_BLOB
    }
}

/// `git_object__type_from_filemode`.
pub const fn type_from_filemode(mode: u32) -> c_int {
    match mode {
        FILEMODE_TREE => OBJ_TREE,
        FILEMODE_COMMIT => OBJ_COMMIT,
        FILEMODE_BLOB | FILEMODE_BLOB_EXECUTABLE | FILEMODE_LINK => OBJ_BLOB,
        _ => OBJ_INVALID,
    }
}

/// `entry_sort_cmp`: git's tree order (a tree sorts as if its name ended
/// with `/`).
pub fn entry_cmp_parts(n1: &[u8], tree1: bool, n2: &[u8], tree2: bool) -> c_int {
    crate::util::fs_path::cmp(n1, n1.len(), tree1, n2, n2.len(), tree2, |a, b, n| {
        crate::util::util::strncmp(a, b, n)
    })
}

/// `entry_sort_cmp` of two live entries.
///
/// # Safety
///
/// As [`GitTreeEntry::name`] for both.
unsafe fn entry_sort_cmp(e1: &GitTreeEntry, e2: &GitTreeEntry) -> c_int {
    // SAFETY: the caller's contract.
    let (n1, n2) = unsafe { (e1.name(), e2.name()) };
    entry_cmp_parts(n1, e1.is_tree(), n2, e2.is_tree())
}

/// `TREE_ENTRY_CHECK_NAMELEN`: sets the error (and goes on, as the C's).
#[inline]
fn check_namelen(n: usize) {
    if n > usize::from(u16::MAX) {
        let _ = git_error_set!(ErrorClass::INVALID, "tree entry path too long");
    }
}

/// `alloc_entry`: a self-contained entry (the entry, then its name and a
/// NUL, then room for an id) from `git__allocator`; NULL on failure.
pub(crate) fn alloc_entry(filename: &[u8], id: &Oid) -> *mut GitTreeEntry {
    let oid_size = id.size();
    check_namelen(filename.len());
    let Some(len) = size_of::<GitTreeEntry>()
        .checked_add(filename.len())
        .and_then(|l| l.checked_add(1))
        .and_then(|l| l.checked_add(oid_size))
    else {
        return ptr::null_mut();
    };
    let entry = alloc::calloc(1, len).cast::<GitTreeEntry>();
    if entry.is_null() {
        return entry;
    }
    let name = entry.cast::<u8>().wrapping_add(size_of::<GitTreeEntry>());
    // SAFETY: `len` zeroed bytes: the entry, then room for the name and its
    // NUL (already 0).
    unsafe {
        ptr::copy_nonoverlapping(filename.as_ptr(), name, filename.len());
        entry.write(GitTreeEntry {
            attr: 0,
            filename_len: filename.len() as u16,
            oid: *id,
            filename: name.cast(),
        });
    }
    entry
}

/// `tree_key_search`: the position of the entry named `name` in `entries`
/// (sorted in tree order), by a homing binary search on the common prefix
/// and a linear scan around it.
pub fn key_search(entries: &[GitTreeEntry], name: &[u8], entry_name: impl Fn(&GitTreeEntry) -> &[u8]) -> Option<usize> {
    check_namelen(name.len());
    let key_len = usize::from(name.len() as u16);
    let key = &name[..key_len];
    let homing_cmp = |e: &GitTreeEntry| -> c_int {
        let en = entry_name(e);
        let n = key_len.min(en.len());
        match key[..n].cmp(&en[..n]) {
            core::cmp::Ordering::Less => -1,
            core::cmp::Ordering::Equal => 0,
            core::cmp::Ordering::Greater => 1,
        }
    };

    // Initial homing search; find an entry on the tree with the same prefix
    // as the filename we're looking for
    let homing = crate::util::util::bsearch(entries, homing_cmp).ok()?;

    // We found a common prefix. Look forward as long as there are entries
    // that share the common prefix
    for (i, e) in entries.iter().enumerate().skip(homing) {
        if homing_cmp(e) < 0 {
            break;
        }
        if usize::from(e.filename_len) == name.len() && entry_name(e) == name {
            return Some(i);
        }
    }

    // If we haven't found our filename yet, look backwards too as long as
    // we have entries with the same prefix
    for i in (0..homing).rev() {
        let e = &entries[i];
        if homing_cmp(e) > 0 {
            break;
        }
        if usize::from(e.filename_len) == name.len() && entry_name(e) == name {
            return Some(i);
        }
    }

    // The filename doesn't exist at all
    None
}

/// The entries of a live tree.
///
/// # Safety
///
/// `tree` is a live tree (parsed, and not freed while the slice lives).
#[inline]
pub(crate) unsafe fn entries<'a>(tree: *const GitTree) -> &'a [GitTreeEntry] {
    // SAFETY: the parse wrote the array before the tree was published;
    // nothing writes it after, and it lives with the tree (contract).
    unsafe { (*ptr::addr_of!((*tree).entries)).as_slice() }
}

/// `entry_fromname`.
///
/// # Safety
///
/// As [`entries`].
pub(crate) unsafe fn entry_fromname<'a>(tree: *const GitTree, name: &[u8]) -> Option<&'a GitTreeEntry> {
    // SAFETY: the caller's contract.
    let es = unsafe { entries(tree) };
    // SAFETY: the entries of a live tree.
    let i = key_search(es, name, |e| unsafe { e.name() })?;
    Some(&es[i])
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

fn tree_error_code(msg: &str, path: Option<&[u8]>, code: ErrorCode) -> ErrorCode {
    let _ = match path {
        Some(p) => git_error_set!(ErrorClass::TREE, "{} - {}", msg, Bytes(p)),
        None => git_error_set!(ErrorClass::TREE, "{}", msg),
    };
    code
}

/// `tree_error`: the error set, -1.
fn tree_error(msg: &str, path: Option<&[u8]>) -> ErrorCode {
    tree_error_code(msg, path, ErrorCode::GENERIC)
}

/// `tree_parse_error`: the error set, `GIT_EINVALID`.
fn tree_parse_error(msg: &str) -> ErrorCode {
    tree_error_code(msg, None, ErrorCode::INVALID)
}

/// `parse_mode`: the octal mode at the start of `buf`, and its length.
#[inline]
fn parse_mode(buf: &[u8]) -> Option<(u16, usize)> {
    let first = *buf.first()?;
    if isspace(first) {
        return None;
    }
    // Fast path: up to 7 octal digits (git writes 5 or 6) followed by a
    // non-digit, which git__strntol32 reads the same way.
    let mut v: u32 = 0;
    let mut i = 0;
    while i < buf.len() && i < 8 {
        let c = buf[i];
        if !(b'0'..=b'7').contains(&c) {
            break;
        }
        v = v * 8 + u32::from(c - b'0');
        i += 1;
    }
    if i > 0 && i < 8 {
        return u16::try_from(v).ok().map(|m| (m, i));
    }
    // Anything else: the C's general conversion.
    let (mode, used) = crate::util::util::strntol32(buf, 8).ok()?;
    if mode < 0 {
        return None;
    }
    u16::try_from(mode).ok().map(|m| (m, used))
}

/// `git_tree__parse_raw`'s loop: the entries of the tree in `data`, their
/// names pointing into `data`.
pub fn parse_entries(out: &mut GitArray<GitTreeEntry>, data: &[u8], oid_type: git_oid_t) -> Result<()> {
    let kind = OidType::from_raw(oid_type);
    let oid_size = kind.map_or(0, OidType::size);
    let end = data.len();
    let mut pos = 0usize;
    while pos < end {
        let Some((attr, used)) = parse_mode(&data[pos..]) else {
            return Err(tree_parse_error("failed to parse tree: can't parse filemode"));
        };
        pos += used;
        if pos >= end || data[pos] != b' ' {
            return Err(tree_parse_error("failed to parse tree: missing space after filemode"));
        }
        pos += 1;
        let Some(nul) = memchr::memchr(0, &data[pos..]).map(|i| pos + i) else {
            return Err(tree_parse_error("failed to parse tree: object is corrupted"));
        };
        let filename_len = nul - pos;
        if filename_len == 0 || filename_len > usize::from(u16::MAX) {
            return Err(tree_parse_error("failed to parse tree: can't parse filename"));
        }
        if end - (nul + 1) < oid_size {
            return Err(tree_parse_error("failed to parse tree: can't parse OID"));
        }
        let raw = &data[nul + 1..nul + 1 + oid_size];
        let oid = match kind {
            Some(k) => Oid::from_bytes(raw, k).unwrap_or_default(),
            None => {
                let _ = git_error_set!(ErrorClass::INVALID, "unable to parse OID - unknown type");
                Oid::default()
            }
        };
        out.push(GitTreeEntry {
            attr,
            filename_len: filename_len as u16,
            oid,
            filename: data[pos..].as_ptr().cast(),
        })?;
        pos = nul + 1 + oid_size;
    }
    Ok(())
}

/// `git_tree__parse_raw` into `tree` (whose entries then point into `data`).
pub(crate) fn parse_into(tree: &mut GitTree, data: &[u8], oid_type: git_oid_t) -> Result<()> {
    tree.odb_obj = ptr::null_mut();
    tree.entries = GitArray::with_capacity(DEFAULT_TREE_SIZE)?;
    parse_entries(&mut tree.entries, data, oid_type)
}

// ---------------------------------------------------------------------------
// Tree builders
// ---------------------------------------------------------------------------

/// `git_treebuilder`: the entries by name. Each entry is an `alloc_entry`
/// block that the builder owns; its key is the entry's own name, so an
/// entry is freed only after its key has left the map.
pub struct GitTreebuilder {
    repo: *mut git_repository,
    map: GitHashMap<CStrKey<'static>, *mut GitTreeEntry>,
    write_cache: GitStr,
}

/// The key of an entry the builder owns.
///
/// # Safety
///
/// `e` is an `alloc_entry` block that lives while the key is in the map.
unsafe fn entry_key(e: *mut GitTreeEntry) -> CStrKey<'static> {
    // SAFETY: the name is NUL-terminated inside the block, which outlives
    // the key (contract: the builder removes the key before freeing).
    unsafe { CStrKey::from_ptr(ptr::NonNull::new_unchecked((*e).filename.cast_mut())) }
}

/// A key to look `name` up with. The map's key type claims `'static`; this
/// key is only compared against during the lookup and never stored, so it
/// need only outlive the call.
#[inline]
fn probe_key(name: &CStr) -> CStrKey<'static> {
    // SAFETY: the key does not outlive the map operation it is passed to
    // (`get` / `remove` only read it), during which `name` is borrowed.
    unsafe { CStrKey::from_ptr(ptr::NonNull::from(name).cast()) }
}

/// `valid_entry_name`.
///
/// # Safety
///
/// `repo` is NULL or live.
unsafe fn valid_entry_name(repo: *mut git_repository, filename: &CStr) -> bool {
    use crate::sys::path;
    !filename.is_empty()
        // SAFETY: the caller's contract; a NUL-terminated name.
        && unsafe {
            path::is_valid(
                repo,
                filename.as_ptr(),
                0,
                path::REJECT_TRAVERSAL | path::REJECT_DOT_GIT | path::REJECT_SLASH,
            )
        }
}

/// `otype_from_mode`.
fn otype_from_mode(filemode: u32) -> c_int {
    match filemode {
        FILEMODE_TREE => OBJ_TREE,
        FILEMODE_COMMIT => OBJ_COMMIT,
        _ => OBJ_BLOB,
    }
}

/// `check_entry`.
///
/// # Safety
///
/// `repo` is live.
unsafe fn check_entry(repo: *mut git_repository, filename: &CStr, id: &Oid, filemode: u32) -> Result<()> {
    let f = Some(filename.to_bytes());
    if !valid_filemode(filemode) {
        return Err(tree_error("failed to insert entry: invalid filemode for file", f));
    }
    // SAFETY: `repo` is live (contract).
    if !unsafe { valid_entry_name(repo, filename) } {
        return Err(tree_error("failed to insert entry: invalid name for a tree entry", f));
    }
    if id.is_zero() {
        return Err(tree_error("failed to insert entry: invalid null OID", f));
    }
    if filemode != FILEMODE_COMMIT
        // SAFETY: `repo` is live.
        && !unsafe { pend::git_object__is_valid(repo, id, otype_from_mode(filemode)) }
    {
        return Err(tree_error("failed to insert entry: invalid object specified", f));
    }
    Ok(())
}

impl GitTreebuilder {
    /// `git_treebuilder_new`, without the source: an empty builder.
    fn empty(repo: *mut git_repository) -> GitTreebuilder {
        GitTreebuilder {
            repo,
            map: GitHashMap::new(),
            write_cache: GitStr::new(),
        }
    }

    /// `git_treebuilder_new`: a builder allocated with `git__allocator`,
    /// holding the entries of `source` (unvalidated).
    ///
    /// # Safety
    ///
    /// `repo` is live; `source` is NULL or a live tree.
    pub(crate) unsafe fn new_raw(repo: *mut git_repository, source: *const GitTree) -> Result<*mut GitTreebuilder> {
        let p = alloc::malloc(size_of::<GitTreebuilder>()).cast::<GitTreebuilder>();
        if p.is_null() {
            return Err(ErrorCode::GENERIC);
        }
        // SAFETY: a fresh allocation, aligned (8) for the builder.
        unsafe { p.write(GitTreebuilder::empty(repo)) };
        if !source.is_null() {
            // SAFETY: a live tree (contract).
            for e in unsafe { entries(source) } {
                // SAFETY: the entry of a live tree: its name is
                // NUL-terminated after `filename_len` bytes.
                let r = unsafe { (*p).append_entry(e.filename, &e.oid, u32::from(e.attr), false) };
                if r.is_err() {
                    // SAFETY: the builder made above, ours alone.
                    unsafe { free_raw(p) };
                    return Err(ErrorCode::GENERIC);
                }
            }
        }
        Ok(p)
    }

    /// `append_entry`.
    ///
    /// # Safety
    ///
    /// `filename` is NUL-terminated; the builder's repository is live.
    unsafe fn append_entry(&mut self, filename: *const c_char, id: &Oid, filemode: u32, validate: bool) -> Result<()> {
        // SAFETY: NUL-terminated (contract).
        let name = unsafe { CStr::from_ptr(filename) };
        if validate {
            // SAFETY: the repository is live (contract).
            unsafe { check_entry(self.repo, name, id, filemode) }?;
        }
        let entry = alloc_entry(name.to_bytes(), id);
        if entry.is_null() {
            return Err(ErrorCode::GENERIC);
        }
        // SAFETY: the entry just made, ours.
        unsafe { (*entry).attr = filemode as u16 };
        // SAFETY: the entry lives while its key is in the map.
        match self.map.put(unsafe { entry_key(entry) }, entry) {
            Ok(old) => {
                if let Some((_, old)) = old {
                    // (The C leaks the entry a duplicate name replaces.)
                    // SAFETY: the replaced entry left the map with its key.
                    unsafe { alloc::free(old.cast()) };
                }
                Ok(())
            }
            Err(_) => {
                // SAFETY: never entered the map.
                unsafe { alloc::free(entry.cast()) };
                Err(git_error_set!(
                    ErrorClass::TREE,
                    "failed to append entry {} to the tree builder",
                    Bytes(name.to_bytes())
                ))
            }
        }
    }

    /// `git_treebuilder_entrycount`.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether it has no entries.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// `treebuilder_get`.
    pub(crate) fn get(&self, filename: &CStr) -> Option<*mut GitTreeEntry> {
        self.map.get(&probe_key(filename)).copied()
    }

    /// `git_treebuilder_insert`: the inserted (or updated) entry.
    ///
    /// # Safety
    ///
    /// The builder's repository is live.
    pub(crate) unsafe fn insert(&mut self, filename: &CStr, id: &Oid, filemode: u32) -> Result<*mut GitTreeEntry> {
        // SAFETY: the repository is live (contract).
        unsafe { check_entry(self.repo, filename, id, filemode) }?;
        let entry = match self.get(filename) {
            Some(e) => {
                // SAFETY: an entry the builder owns.
                unsafe { (*e).oid = *id };
                e
            }
            None => {
                let e = alloc_entry(filename.to_bytes(), id);
                if e.is_null() {
                    return Err(ErrorCode::GENERIC);
                }
                // SAFETY: the entry lives while its key is in the map.
                if self.map.put(unsafe { entry_key(e) }, e).is_err() {
                    // SAFETY: never entered the map.
                    unsafe { alloc::free(e.cast()) };
                    return Err(git_error_set!(ErrorClass::TREE, "failed to insert {}", Bytes(filename.to_bytes())));
                }
                e
            }
        };
        // SAFETY: an entry the builder owns.
        unsafe { (*entry).attr = filemode as u16 };
        Ok(entry)
    }

    /// `git_treebuilder_remove` (once found).
    fn remove_found(&mut self, filename: &CStr) {
        if let Some((_, e)) = self.map.remove(&probe_key(filename)) {
            // SAFETY: the entry left the map with its key; the builder owned
            // it.
            unsafe { alloc::free(e.cast()) };
        }
    }

    /// `git_treebuilder_clear`.
    pub(crate) fn clear(&mut self) {
        for &e in self.map.values() {
            // SAFETY: entries the builder owns; the map is cleared just
            // after, before any key is read again.
            unsafe { alloc::free(e.cast()) };
        }
        self.map.clear();
    }

    /// The entries sorted in tree order (`git_treebuilder__write_with_buffer`'s
    /// vector).
    fn sorted_entries(&self) -> Result<Vec<*mut GitTreeEntry>> {
        let mut v = Vec::new();
        v.try_reserve(self.map.len()).map_err(|_| errors::oom())?;
        let mut it: HashMapIter = 0;
        while let Some((_, &e)) = self.map.iterate(&mut it) {
            v.push(e);
        }
        // SAFETY: entries the builder owns, unchanged while sorting.
        v.sort_by(|&a, &b| crate::util::util::ordering(unsafe { entry_sort_cmp(&*a, &*b) }));
        Ok(v)
    }

    /// The tree's bytes, written to `buf` (`git_treebuilder__write_with_buffer`
    /// without the odb write).
    pub(crate) fn serialize(&self, buf: &mut GitStr, oid_size: usize) -> Result<()> {
        buf.clear();
        let entrycount = self.map.len();
        let entries = self.sorted_entries()?;
        if buf.capacity() == 0 {
            buf.grow(entrycount.saturating_mul(72))?;
        }
        for &e in &entries {
            // SAFETY: an entry the builder owns: `filename_len` bytes and a
            // NUL (alloc_entry).
            let (attr, name, id) = unsafe { ((*e).attr, core::slice::from_raw_parts((*e).filename.cast::<u8>(), usize::from((*e).filename_len) + 1), (*e).oid) };
            let _ = buf.printf(3, format_args!("{attr:o} "));
            let _ = buf.put(name);
            let _ = buf.put(&id.raw_id()[..oid_size.min(crate::oid::GIT_OID_MAX_SIZE)]);
            if buf.is_oom() {
                return Err(ErrorCode::GENERIC);
            }
        }
        Ok(())
    }

    /// `git_treebuilder__write_with_buffer`.
    ///
    /// # Safety
    ///
    /// The builder's repository is live.
    unsafe fn write_with_buffer(&self, buf: &mut GitStr) -> Result<Oid> {
        // SAFETY: a live repository (contract).
        let oid_type: git_oid_t = unsafe { pend::field(self.repo, pend::REPO_OID_TYPE) };
        self.serialize(buf, crate::oid::oid_size(oid_type))?;
        let odb = repo_odb(self.repo)?;
        let mut oid = Oid::default();
        let bytes = buf.as_bytes();
        // SAFETY: readable bytes; `oid` writable.
        ErrorCode::check(unsafe { pend::git_odb_write(&mut oid, odb, bytes.as_ptr().cast(), bytes.len(), OBJ_TREE) })?;
        Ok(oid)
    }

    /// `git_treebuilder_write`.
    ///
    /// # Safety
    ///
    /// As [`write_with_buffer`](Self::write_with_buffer).
    pub(crate) unsafe fn write(&mut self) -> Result<Oid> {
        let mut cache = core::mem::take(&mut self.write_cache);
        // SAFETY: the caller's contract.
        let r = unsafe { self.write_with_buffer(&mut cache) };
        self.write_cache = cache;
        r
    }
}

impl Drop for GitTreebuilder {
    fn drop(&mut self) {
        self.clear();
    }
}

/// `git_treebuilder_free` of a builder from [`GitTreebuilder::new_raw`].
///
/// # Safety
///
/// `bld` is NULL or such a builder, not used after.
pub(crate) unsafe fn free_raw(bld: *mut GitTreebuilder) {
    if bld.is_null() {
        return;
    }
    // SAFETY: the caller hands the builder over.
    unsafe {
        ptr::drop_in_place(bld);
        alloc::free(bld.cast());
    }
}

// ---------------------------------------------------------------------------
// Writing the index
// ---------------------------------------------------------------------------

/// The bytes of a C string, `at(i)` reading 0 past the end.
#[inline]
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// Whether `path` is outside `dirname` (`write_tree`'s loop exit).
#[inline]
fn left_dir(path: &[u8], dirname: &[u8]) -> bool {
    path.len() < dirname.len() || &path[..dirname.len()] != dirname || (!dirname.is_empty() && at(path, dirname.len()) != b'/')
}

/// The path of index entry `i`.
///
/// # Safety
///
/// `index` is live and unchanged while the bytes are used.
unsafe fn index_entry<'a>(index: *mut pend::git_index, i: usize) -> (&'a [u8], Oid, u32) {
    // SAFETY: the caller's contract: a live entry, its path NUL-terminated.
    unsafe {
        let e = pend::git_index_get_byindex(index, i);
        (CStr::from_ptr((*e).path).to_bytes(), (*e).id, (*e).mode)
    }
}

/// `find_next_dir`.
///
/// # Safety
///
/// As [`index_entry`].
unsafe fn find_next_dir(dirname: &[u8], index: *mut pend::git_index, start: usize) -> usize {
    // SAFETY: a live index.
    let n = unsafe { pend::git_index_entrycount(index) };
    let mut i = start;
    while i < n {
        // SAFETY: `i < n`.
        let (path, _, _) = unsafe { index_entry(index, i) };
        if left_dir(path, dirname) {
            break;
        }
        i += 1;
    }
    i
}

/// `write_tree`: write the tree of `dirname` from the index entries from
/// `start`; the index of the first entry after it.
///
/// # Safety
///
/// `repo` and `index` are live, and the index unchanged during the call.
unsafe fn write_tree(
    oid: &mut Oid,
    repo: *mut git_repository,
    index: *mut pend::git_index,
    dirname: &CStr,
    start: usize,
    shared_buf: &mut GitStr,
) -> Result<usize> {
    let dir = dirname.to_bytes();
    // SAFETY: a live index: its tree cache is NULL or live.
    let cache = unsafe {
        let root: *const pend::git_tree_cache = pend::field(index, pend::INDEX_TREE);
        pend::git_tree_cache_get(root, dirname.as_ptr())
    };
    if !cache.is_null() {
        // SAFETY: a live tree cache entry.
        let (count, cid): (isize, Oid) =
            unsafe { (pend::field(cache, pend::TREE_CACHE_ENTRY_COUNT), pend::field(cache, pend::TREE_CACHE_OID)) };
        if count >= 0 {
            *oid = cid;
            // SAFETY: as above.
            return Ok(unsafe { find_next_dir(dir, index, start) });
        }
    }

    // SAFETY: `repo` live.
    let bld = unsafe { GitTreebuilder::new_raw(repo, ptr::null()) }?;
    let r = (|| -> Result<usize> {
        // SAFETY: a live index.
        let entries = unsafe { pend::git_index_entrycount(index) };
        let mut i = start;
        // This loop is unfortunate, but necessary. The index doesn't have
        // any directories, so we need to handle that manually, and we need
        // to keep track of the current position.
        while i < entries {
            // SAFETY: `i < entries`.
            let (path, id, mode) = unsafe { index_entry(index, i) };
            // If we've left our (sub)tree, exit the loop and return.
            if left_dir(path, dir) {
                break;
            }
            let mut fstart = dir.len();
            if at(path, fstart) == b'/' {
                fstart += 1;
            }
            let filename = &path[fstart..];
            if let Some(slash) = memchr::memchr(b'/', filename) {
                let subdir = &path[..fstart + slash];
                let subdir_c = cstring(subdir)?;
                let mut sub_oid = Oid::default();
                // Write out the subtree
                // SAFETY: the caller's contract.
                let written = unsafe { write_tree(&mut sub_oid, repo, index, &subdir_c, i, shared_buf) }?;
                i = written;
                // We need to figure out what we want to insert into this
                // tree. If we're traversing deps/zlib/, then we only want to
                // write 'zlib' into the tree.
                let last_comp = match memchr::memrchr(b'/', subdir) {
                    Some(p) => &subdir_c.to_bytes_with_nul()[p + 1..],
                    None => subdir_c.to_bytes_with_nul(),
                };
                // SAFETY: the builder is ours; `last_comp` NUL-terminated.
                unsafe { (*bld).append_entry(last_comp.as_ptr().cast(), &sub_oid, FILEMODE_TREE, true) }?;
            } else {
                // SAFETY: as above; the entry's name ends the path, which
                // is NUL-terminated.
                unsafe { (*bld).append_entry(filename.as_ptr().cast(), &id, mode, true) }?;
                i += 1;
            }
        }
        // SAFETY: the builder is ours; `repo` live.
        *oid = unsafe { (*bld).write_with_buffer(shared_buf) }.map_err(|_| ErrorCode::GENERIC)?;
        Ok(i)
    })();
    // SAFETY: the builder made above, freed once.
    unsafe { free_raw(bld) };
    r.map_err(|_| ErrorCode::GENERIC)
}

/// A NUL-terminated copy of `s` (which has no NUL: it is part of a path).
fn cstring(s: &[u8]) -> Result<std::ffi::CString> {
    let mut v = Vec::new();
    v.try_reserve(s.len() + 1).map_err(|_| errors::oom())?;
    v.extend_from_slice(s);
    std::ffi::CString::new(v).map_err(|_| errors::oom())
}

/// `git_tree__write_index`.
///
/// # Safety
///
/// `index` and `repo` are live, and nothing else uses the index meanwhile.
unsafe fn write_index(oid: &mut Oid, index: *mut pend::git_index, repo: *mut git_repository) -> Result<()> {
    // SAFETY: a live index.
    if unsafe { pend::git_index_has_conflicts(index) } != 0 {
        let _ = git_error_set!(ErrorClass::INDEX, "cannot create a tree from a not fully merged index.");
        return Err(ErrorCode::UNMERGED);
    }
    // SAFETY: a live index: its tree cache is NULL or live.
    let root: *const pend::git_tree_cache = unsafe { pend::field(index, pend::INDEX_TREE) };
    if !root.is_null() {
        // SAFETY: as above.
        let (count, cid): (isize, Oid) =
            unsafe { (pend::field(root, pend::TREE_CACHE_ENTRY_COUNT), pend::field(root, pend::TREE_CACHE_OID)) };
        if count >= 0 {
            *oid = cid;
            return Ok(());
        }
    }

    // The tree cache didn't help us; we'll have to write out a tree. If the
    // index is ignore_case, we must make it case-sensitive for the duration
    // of the tree-write operation.
    // SAFETY: a live index's bitfields.
    let flags: c_uint = unsafe { pend::field(index, pend::INDEX_FLAGS) };
    let old_ignore_case = flags & 2 != 0;
    if old_ignore_case {
        // SAFETY: a live index.
        unsafe { pend::git_index__set_ignore_case(index, false) };
    }
    let mut shared_buf = GitStr::new();
    // SAFETY: the caller's contract.
    let ret = unsafe { write_tree(oid, repo, index, c"", 0, &mut shared_buf) };
    drop(shared_buf);
    if old_ignore_case {
        // SAFETY: as above.
        unsafe { pend::git_index__set_ignore_case(index, true) };
    }
    // SAFETY: the index's `tree` field, not otherwise used meanwhile.
    unsafe { pend::field_ptr::<*mut pend::git_tree_cache, _>(index, pend::INDEX_TREE).write(ptr::null_mut()) };
    ret?;

    // SAFETY: the index's tree pool, a `git_pool` by value, not otherwise
    // used meanwhile (its cache, which pointed into it, was just dropped).
    unsafe { (*pend::field_ptr::<crate::util::pool::GitPool, _>(index, pend::INDEX_TREE_POOL)).clear() };

    let mut tree = ptr::null_mut();
    // SAFETY: `repo` live; `tree` writable.
    ErrorCode::check(unsafe { pend::git_tree_lookup(&mut tree, repo, oid) })?;
    // Read the tree cache into the index
    // SAFETY: the index's fields and the tree just looked up.
    let ret = unsafe {
        let oid_type: git_oid_t = pend::field(index, pend::INDEX_OID_TYPE);
        pend::git_tree_cache_read_tree(
            pend::field_ptr(index, pend::INDEX_TREE),
            tree,
            oid_type,
            pend::field_ptr(index, pend::INDEX_TREE_POOL),
        )
    };
    // SAFETY: freed once.
    unsafe { pend::git_tree_free(tree) };
    ErrorCode::check_zero(ret)
}

// ---------------------------------------------------------------------------
// Walks, paths, updates
// ---------------------------------------------------------------------------

/// What a walk's callback says about an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkResult {
    /// Go on.
    Continue,
    /// (Pre-order only) do not descend into this entry.
    Skip,
    /// Stop the walk, with this (negative) code.
    Stop(c_int),
}

/// `tree_walk`: `cb(root, entry)` for every entry under `tree`, pre- or
/// post-order; `lookup` loads a subtree (released by `free`).
///
/// # Safety
///
/// `tree` is a live tree; `lookup` returns live trees (released only by
/// `free`).
pub(crate) unsafe fn walk(
    tree: *const GitTree,
    path: &mut GitStr,
    preorder: bool,
    lookup: &mut dyn FnMut(&Oid) -> Result<*mut GitTree>,
    free: &mut dyn FnMut(*mut GitTree),
    cb: &mut dyn FnMut(&GitStr, &GitTreeEntry) -> WalkResult,
) -> c_int {
    // SAFETY: a live tree (contract).
    let n = unsafe { entries(tree) }.len();
    for i in 0..n {
        // Re-read each time: the callback runs between, and only reads the
        // tree (as the C's contract).
        // SAFETY: a live tree; `i < n`.
        let entry = unsafe { entries(tree) }[i];
        if preorder {
            match cb(path, &entry) {
                WalkResult::Stop(e) => {
                    // negative value stops iteration
                    return errors::set_after_callback(e, "git_tree_walk");
                }
                // positive value skips this entry
                WalkResult::Skip => continue,
                WalkResult::Continue => {}
            }
        }
        if entry.is_tree() {
            let path_len = path.len();
            let subtree = match lookup(&entry.oid) {
                Ok(t) => t,
                Err(e) => return e.raw(),
            };
            // append the next entry to the path
            // SAFETY: an entry of a live tree.
            let _ = path.puts(unsafe { CStr::from_ptr(entry.filename) }.to_bytes());
            let _ = path.putc(b'/');
            let error = if path.is_oom() {
                -1
            } else {
                // SAFETY: the subtree just looked up.
                unsafe { walk(subtree, path, preorder, lookup, free, cb) }
            };
            free(subtree);
            if error != 0 {
                return error;
            }
            path.truncate(path_len);
        }
        if !preorder {
            if let WalkResult::Stop(e) = cb(path, &entry) {
                return errors::set_after_callback(e, "git_tree_walk");
            }
        }
    }
    0
}

/// `subpath_len`.
fn subpath_len(path: &[u8]) -> usize {
    memchr::memchr(b'/', path).unwrap_or(path.len())
}

/// `git_tree_entry_dup` of an entry.
///
/// # Safety
///
/// As [`GitTreeEntry::name`].
pub(crate) unsafe fn entry_dup(source: &GitTreeEntry) -> Result<*mut GitTreeEntry> {
    // SAFETY: the caller's contract.
    let cpy = alloc_entry(unsafe { source.name() }, &source.oid);
    if cpy.is_null() {
        return Err(ErrorCode::GENERIC);
    }
    // SAFETY: the entry just made.
    unsafe { (*cpy).attr = source.attr };
    Ok(cpy)
}

/// `git_tree_entry_bypath`.
///
/// # Safety
///
/// `root` is a live tree; `lookup` returns live trees released by `free`.
pub(crate) unsafe fn entry_bypath(
    root: *const GitTree,
    path: &[u8],
    lookup: &mut dyn FnMut(&Oid) -> Result<*mut GitTree>,
    free: &mut dyn FnMut(*mut GitTree),
) -> Result<*mut GitTreeEntry> {
    // Find how long is the current path component (i.e. the filename
    // between two slashes)
    let filename_len = subpath_len(path);
    if filename_len == 0 {
        let _ = git_error_set!(ErrorClass::TREE, "invalid tree path given");
        return Err(ErrorCode::NOT_FOUND);
    }
    let comp = &path[..filename_len];
    // SAFETY: a live tree (contract).
    let Some(entry) = (unsafe { entry_fromname(root, comp) }) else {
        let _ = git_error_set!(
            ErrorClass::TREE,
            "the path '{}' does not exist in the given tree",
            Bytes(comp)
        );
        return Err(ErrorCode::NOT_FOUND);
    };
    let entry = *entry;
    match at(path, filename_len) {
        b'/' => {
            // If there are more components in the path... then this entry
            // *must* be a tree
            if !entry.is_tree() {
                let _ = git_error_set!(ErrorClass::TREE, "the path '{}' exists but is not a tree", Bytes(comp));
                return Err(ErrorCode::NOT_FOUND);
            }
            // If there's only a slash left in the path, we return the
            // current entry; otherwise, we keep walking down the path
            if at(path, filename_len + 1) == 0 {
                // SAFETY: an entry of a live tree.
                return unsafe { entry_dup(&entry) };
            }
        }
        // If there are no more components in the path, return this entry
        // SAFETY: as above.
        0 => return unsafe { entry_dup(&entry) },
        _ => {}
    }
    let subtree = lookup(&entry.oid).map_err(|_| ErrorCode::GENERIC)?;
    // SAFETY: the subtree just looked up.
    let r = unsafe { entry_bypath(subtree, &path[filename_len + 1..], lookup, free) };
    free(subtree);
    r
}

/// `git_tree_update` (public).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GitTreeUpdate {
    /// `git_tree_update_t`: `GIT_TREE_UPDATE_UPSERT` (0) or `_REMOVE` (1).
    pub action: c_int,
    pub id: Oid,
    /// `git_filemode_t`.
    pub filemode: c_uint,
    pub path: *const c_char,
}

/// One level of `git_tree_create_updated`'s stack.
struct StackEntry {
    bld: *mut GitTreebuilder,
    tree: *mut GitTree,
    name: Option<std::ffi::CString>,
}

impl StackEntry {
    /// Release what the level holds.
    fn release(&mut self) {
        // SAFETY: NULL or a builder this level made, freed once.
        unsafe { free_raw(core::mem::replace(&mut self.bld, ptr::null_mut())) };
        // SAFETY: NULL or a tree this level looked up, released once.
        unsafe { pend::git_tree_free(core::mem::replace(&mut self.tree, ptr::null_mut())) };
        self.name = None;
    }
}

/// `count_slashes`.
fn count_slashes(path: &[u8]) -> usize {
    memchr::memchr_iter(b'/', path).count()
}

/// `next_component`: the component before the next slash, if there is one.
fn next_component(input: &[u8]) -> Option<&[u8]> {
    memchr::memchr(b'/', input).map(|i| &input[..i])
}

/// The type name of an object type.
fn type2string(t: c_int) -> &'static [u8] {
    // SAFETY: git_object_type2string returns a static string.
    unsafe { CStr::from_ptr(pend::git_object_type2string(t)) }.to_bytes()
}

/// `create_popped_tree`: write the tree of `popped` (or remove it when
/// empty) into `current`.
///
/// # Safety
///
/// The builders' repository is live; the levels' trees and builders are
/// live.
unsafe fn create_popped_tree(current: &mut StackEntry, mut popped: StackEntry) -> Result<()> {
    // SAFETY: NULL or the level's tree, released once.
    unsafe { pend::git_tree_free(core::mem::replace(&mut popped.tree, ptr::null_mut())) };
    let name = popped.name.take().unwrap_or_default();
    // SAFETY: the level's builder.
    let pb = unsafe { &mut *popped.bld };
    // If the tree would be empty, remove it from the one higher up
    if pb.is_empty() {
        popped.release();
        // SAFETY: the parent level's builder.
        let cb = unsafe { &mut *current.bld };
        if cb.get(&name).is_none() {
            return Err(tree_error("failed to remove entry: file isn't in the tree", Some(name.to_bytes())));
        }
        cb.remove_found(&name);
        return Ok(());
    }
    // SAFETY: the repository is live (contract).
    let new_tree = unsafe { pb.write() };
    popped.release();
    let new_tree = new_tree?;

    // Error out if this would create a D/F conflict in this update
    if !current.tree.is_null() {
        // SAFETY: the parent level's live tree.
        if let Some(e) = unsafe { entry_fromname(current.tree, name.to_bytes()) } {
            if entry_type(e.attr) != OBJ_TREE {
                return Err(git_error_set!(ErrorClass::TREE, "D/F conflict when updating tree"));
            }
        }
    }
    // SAFETY: the parent level's builder; the repository is live.
    unsafe { (*current.bld).insert(&name, &new_tree, FILEMODE_TREE) }.map(|_| ())
}

/// `git_tree_create_updated`.
///
/// # Safety
///
/// `repo` live; `baseline` NULL or a live tree; `updates` holds `n` valid
/// updates whose paths are NUL-terminated.
unsafe fn create_updated(out: &mut Oid, repo: *mut git_repository, baseline: *mut GitTree, updates: &[GitTreeUpdate]) -> Result<()> {
    // Sort the entries for traversal
    let mut sorted: Vec<(&[u8], &GitTreeUpdate)> = Vec::new();
    sorted.try_reserve(updates.len()).map_err(|_| errors::oom())?;
    for u in updates {
        // SAFETY: NUL-terminated (contract).
        sorted.push((unsafe { CStr::from_ptr(u.path) }.to_bytes(), u));
    }
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    if sorted.windows(2).any(|w| w[0].0 == w[1].0) {
        return Err(git_error_set!(ErrorClass::TREE, "duplicate entries given for update"));
    }

    let mut stack: Vec<StackEntry> = Vec::new();
    let r = (|| -> Result<()> {
        stack.push(StackEntry { bld: ptr::null_mut(), tree: ptr::null_mut(), name: None });
        if !baseline.is_null() {
            let mut t = ptr::null_mut();
            // SAFETY: a live tree.
            ErrorCode::check(unsafe { pend::git_tree_dup(&mut t, baseline) })?;
            stack[0].tree = t;
        }
        // SAFETY: `repo` live; the tree NULL or live.
        stack[0].bld = unsafe { GitTreebuilder::new_raw(repo, stack[0].tree) }?;

        for i in 0..sorted.len() {
            let last_update = if i == 0 { None } else { Some(sorted[i - 1].0) };
            let (upath, update) = sorted[i];
            // Figure out how much we need to change from the previous tree
            let common_prefix = last_update.map_or(0, |l| crate::util::fs_path::common_dirlen(l, upath));
            // The entries are sorted, so when we find we're no longer in the
            // same directory, we need to abandon the old tree (steps up) and
            // dive down to the next one.
            let steps_up = last_update.map_or(0, |l| count_slashes(&l[common_prefix..]));
            for _ in 0..steps_up {
                let Some(popped) = stack.pop() else {
                    return Err(errors::assert_failed("popped"));
                };
                let Some(current) = stack.last_mut() else {
                    let mut popped = popped;
                    popped.release();
                    return Err(errors::assert_failed("current"));
                };
                // SAFETY: live levels; `repo` live.
                unsafe { create_popped_tree(current, popped) }?;
            }

            // Now that we've created the trees we popped from the stack,
            // let's go back down
            let mut path = &upath[common_prefix..];
            while let Some(component) = next_component(path) {
                let comp = cstring(component)?;
                let last = stack.last().expect("the root level is never popped here");
                let entry = if last.tree.is_null() {
                    None
                } else {
                    // SAFETY: the level's live tree.
                    unsafe { entry_fromname(last.tree, component) }.map(|e| (e.attr, e.oid))
                };
                // SAFETY: the level's builder; its entries live in it.
                let entry = entry.or_else(|| unsafe { last.bld.as_ref() }?.get(&comp).map(|e| unsafe { ((*e).attr, (*e).oid) }));
                if let Some((attr, _)) = entry {
                    if entry_type(attr) != OBJ_TREE {
                        return Err(git_error_set!(ErrorClass::TREE, "D/F conflict when updating tree"));
                    }
                }
                stack.push(StackEntry { bld: ptr::null_mut(), tree: ptr::null_mut(), name: None });
                let new_entry = stack.last_mut().expect("just pushed");
                if let Some((_, id)) = entry {
                    // SAFETY: `repo` live; `tree` writable.
                    ErrorCode::check(unsafe { pend::git_tree_lookup(&mut new_entry.tree, repo, &id) })?;
                }
                // SAFETY: `repo` live; the tree NULL or live.
                new_entry.bld = unsafe { GitTreebuilder::new_raw(repo, new_entry.tree) }?;
                new_entry.name = Some(comp);
                // Get to the start of the next component
                path = &path[component.len() + 1..];
            }

            // After all that, we're finally at the place where we want to
            // perform the update
            let last = stack.last_mut().expect("the root level");
            // SAFETY: the level's builder.
            let lb = unsafe { &mut *last.bld };
            match update.action {
                0 => {
                    // Make sure we're replacing something of the same type
                    let basename = basename_c(upath)?;
                    let new_type = type_from_filemode(update.filemode);
                    if let Some(e) = lb.get(&basename) {
                        // SAFETY: an entry of the builder.
                        let t = entry_type(unsafe { (*e).attr });
                        if t != new_type {
                            return Err(git_error_set!(
                                ErrorClass::TREE,
                                "cannot replace '{}' with '{}' at '{}'",
                                Bytes(type2string(t)),
                                Bytes(type2string(new_type)),
                                Bytes(upath)
                            ));
                        }
                    }
                    // SAFETY: `repo` live.
                    unsafe { lb.insert(&basename, &update.id, update.filemode) }?;
                }
                1 => {
                    let basename = basename_c(upath)?;
                    if lb.get(&basename).is_none() {
                        return Err(tree_error(
                            "failed to remove entry: file isn't in the tree",
                            Some(basename.to_bytes()),
                        ));
                    }
                    lb.remove_found(&basename);
                }
                _ => return Err(git_error_set!(ErrorClass::TREE, "unknown action for update")),
            }
        }

        // We're done, go up the stack again and write out the tree
        let mut root = loop {
            let popped = stack.pop().expect("the root level");
            match stack.last_mut() {
                // We've reached the top, current is the root tree
                None => break popped,
                // SAFETY: live levels; `repo` live.
                Some(current) => unsafe { create_popped_tree(current, popped) }?,
            }
        };
        // Write out the root tree
        // SAFETY: NULL or the root's tree, released once.
        unsafe { pend::git_tree_free(core::mem::replace(&mut root.tree, ptr::null_mut())) };
        // SAFETY: the root's builder; `repo` live.
        let r = unsafe { (*root.bld).write() };
        root.release();
        *out = r?;
        Ok(())
    })();
    for mut e in stack.drain(..).rev() {
        e.release();
    }
    r
}

/// `git_fs_path_basename` as a C string.
fn basename_c(path: &[u8]) -> Result<std::ffi::CString> {
    let b = crate::util::fs_path::basename(Some(path)).ok_or(ErrorCode::GENERIC)?;
    cstring(b.as_c_str().to_bytes())
}

pub mod ffi {
    //! The exports of `tree.c`.

    use super::*;
    use crate::util::export::{barrier, cstr_bytes};

    /// `git_tree_entry_cmp`.
    ///
    /// # Safety
    ///
    /// `e1` and `e2` are live entries.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_entry_cmp(e1: *const GitTreeEntry, e2: *const GitTreeEntry) -> c_int {
        // SAFETY: live entries (contract), only read.
        barrier(|| unsafe { entry_sort_cmp(&*e1, &*e2) })
    }

    /// `git_tree_entry_free`.
    ///
    /// # Safety
    ///
    /// `entry` is NULL or an entry from `git_tree_entry_dup` /
    /// `git_tree_entry_bypath`, not used after.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_entry_free(entry: *mut GitTreeEntry) {
        // SAFETY: the caller's contract: an `alloc_entry` block.
        barrier(|| unsafe { alloc::free(entry.cast()) })
    }

    /// `git_tree_entry_dup`.
    ///
    /// # Safety
    ///
    /// `dest` writable; `source` NULL or a live entry.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_entry_dup(dest: *mut *mut GitTreeEntry, source: *const GitTreeEntry) -> c_int {
        barrier(|| {
            git_assert_arg!(!source.is_null(), "source", -1);
            // SAFETY: a live entry (contract).
            match unsafe { entry_dup(&*source) } {
                Ok(p) => {
                    // SAFETY: writable (contract).
                    unsafe { dest.write(p) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_tree__free`.
    ///
    /// # Safety
    ///
    /// `tree` is a tree allocated with `git__allocator` (zeroed, then parsed
    /// or not), not used after.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree__free(tree: *mut c_void) {
        barrier(|| {
            let t = tree.cast::<GitTree>();
            if t.is_null() {
                return;
            }
            // SAFETY: the tree is handed over: ours alone.
            unsafe {
                if !(*t).odb_obj.is_null() {
                    pend::git_odb_object_free((*t).odb_obj);
                }
                (*t).entries.dispose();
                alloc::free(tree);
            }
        })
    }

    /// `git_tree_entry_filemode`.
    ///
    /// # Safety
    ///
    /// `entry` is a live entry.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_entry_filemode(entry: *const GitTreeEntry) -> c_uint {
        // SAFETY: a live entry (contract).
        barrier(|| normalize_filemode(u32::from(unsafe { (*entry).attr })))
    }

    /// `git_tree_entry_filemode_raw`.
    ///
    /// # Safety
    ///
    /// `entry` is a live entry.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_entry_filemode_raw(entry: *const GitTreeEntry) -> c_uint {
        // SAFETY: a live entry (contract).
        barrier(|| u32::from(unsafe { (*entry).attr }))
    }

    /// `git_tree_entry_name`.
    ///
    /// # Safety
    ///
    /// `entry` is NULL or a live entry.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_entry_name(entry: *const GitTreeEntry) -> *const c_char {
        barrier(|| {
            git_assert_arg!(!entry.is_null(), "entry", ptr::null());
            // SAFETY: a live entry (contract).
            unsafe { (*entry).filename }
        })
    }

    /// `git_tree_entry_id`.
    ///
    /// # Safety
    ///
    /// `entry` is NULL or a live entry.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_entry_id(entry: *const GitTreeEntry) -> *const Oid {
        barrier(|| {
            git_assert_arg!(!entry.is_null(), "entry", ptr::null());
            // SAFETY: a live entry (contract).
            unsafe { ptr::addr_of!((*entry).oid) }
        })
    }

    /// `git_tree_entry_type`.
    ///
    /// # Safety
    ///
    /// `entry` is NULL or a live entry.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_entry_type(entry: *const GitTreeEntry) -> c_int {
        barrier(|| {
            git_assert_arg!(!entry.is_null(), "entry", OBJ_INVALID);
            // SAFETY: a live entry (contract).
            entry_type(unsafe { (*entry).attr })
        })
    }

    /// `git_tree_entry_to_object`.
    ///
    /// # Safety
    ///
    /// `object_out` writable; `repo` live; `entry` a live entry.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_entry_to_object(
        object_out: *mut *mut pend::git_object,
        repo: *mut git_repository,
        entry: *const GitTreeEntry,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!entry.is_null(), "entry", -1);
            git_assert_arg!(!object_out.is_null(), "object_out", -1);
            // SAFETY: the caller's contract.
            unsafe { pend::git_object_lookup(object_out, repo, ptr::addr_of!((*entry).oid), OBJ_ANY) }
        })
    }

    /// `git_tree_entry_byname`.
    ///
    /// # Safety
    ///
    /// `tree` NULL or a live tree; `filename` NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_entry_byname(tree: *const GitTree, filename: *const c_char) -> *const GitTreeEntry {
        barrier(|| {
            git_assert_arg!(!tree.is_null(), "tree", ptr::null());
            git_assert_arg!(!filename.is_null(), "filename", ptr::null());
            // SAFETY: the caller's contract.
            unsafe { entry_fromname(tree, cstr_bytes(filename)) }.map_or(ptr::null(), ptr::from_ref)
        })
    }

    /// `git_tree_entry_byindex`.
    ///
    /// # Safety
    ///
    /// `tree` NULL or a live tree.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_entry_byindex(tree: *const GitTree, idx: usize) -> *const GitTreeEntry {
        barrier(|| {
            git_assert_arg!(!tree.is_null(), "tree", ptr::null());
            // SAFETY: a live tree (contract).
            unsafe { entries(tree) }.get(idx).map_or(ptr::null(), ptr::from_ref)
        })
    }

    /// `git_tree_entry_byid`.
    ///
    /// # Safety
    ///
    /// `tree` NULL or a live tree; `id` readable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_entry_byid(tree: *const GitTree, id: *const Oid) -> *const GitTreeEntry {
        barrier(|| {
            git_assert_arg!(!tree.is_null(), "tree", ptr::null());
            // SAFETY: readable (contract).
            let id = unsafe { id.read() };
            // SAFETY: a live tree.
            unsafe { entries(tree) }.iter().find(|e| e.oid == id).map_or(ptr::null(), ptr::from_ref)
        })
    }

    /// `git_tree_entrycount`.
    ///
    /// # Safety
    ///
    /// `tree` NULL or a live tree.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_entrycount(tree: *const GitTree) -> usize {
        barrier(|| {
            git_assert_arg!(!tree.is_null(), "tree", 0);
            // SAFETY: a live tree (contract).
            unsafe { entries(tree) }.len()
        })
    }

    /// `git_treebuilder_entrycount`.
    ///
    /// # Safety
    ///
    /// `bld` NULL or a live builder.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_treebuilder_entrycount(bld: *mut GitTreebuilder) -> usize {
        barrier(|| {
            git_assert_arg!(!bld.is_null(), "bld", 0);
            // SAFETY: a live builder, not used elsewhere meanwhile.
            unsafe { (*bld).len() }
        })
    }

    /// `git_tree__parse_raw`.
    ///
    /// # Safety
    ///
    /// `tree` is a fresh (zeroed) tree that only the caller holds; `data` is
    /// readable for `size` bytes and outlives the tree.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree__parse_raw(tree: *mut c_void, data: *const c_char, size: usize, oid_type: git_oid_t) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract.
            let (t, d) = unsafe { (&mut *tree.cast::<GitTree>(), crate::util::export::bytes(data, size)) };
            code(parse_into(t, d, oid_type))
        })
    }

    /// `git_tree__parse`: parse the odb object, whose buffer the entries
    /// point into, and keep it.
    ///
    /// # Safety
    ///
    /// `tree` is a fresh (zeroed) tree that only the caller holds; `odb_obj`
    /// a live odb object.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree__parse(tree: *mut c_void, odb_obj: *mut pend::git_odb_object, oid_type: git_oid_t) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract: an exclusive tree; the object's
            // data is readable for its size and lives as long as the object,
            // which the tree keeps (dup) from here on.
            unsafe {
                let t = &mut *tree.cast::<GitTree>();
                let p = pend::git_odb_object_data(odb_obj).cast::<c_char>();
                let n = pend::git_odb_object_size(odb_obj);
                let d = if p.is_null() { &[][..] } else { crate::util::export::bytes(p, n) };
                if let Err(e) = parse_into(t, d, oid_type) {
                    return e.raw();
                }
                pend::git_odb_object_dup(&mut t.odb_obj, odb_obj)
            }
        })
    }

    /// `git_tree__write_index`.
    ///
    /// # Safety
    ///
    /// `oid` writable; `index` and `repo` NULL or live, the index used by
    /// nothing else meanwhile.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree__write_index(oid: *mut Oid, index: *mut pend::git_index, repo: *mut git_repository) -> c_int {
        barrier(|| {
            git_assert_arg!(!oid.is_null(), "oid", -1);
            git_assert_arg!(!index.is_null(), "index", -1);
            git_assert_arg!(!repo.is_null(), "repo", -1);
            let mut out = Oid::default();
            // SAFETY: the caller's contract.
            let r = unsafe { write_index(&mut out, index, repo) };
            if r.is_ok() || out != Oid::default() {
                // SAFETY: writable (contract).
                unsafe { oid.write(out) };
            }
            code(r)
        })
    }

    /// `git_treebuilder_new`.
    ///
    /// # Safety
    ///
    /// `builder_p` writable; `repo` live; `source` NULL or a live tree.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_treebuilder_new(
        builder_p: *mut *mut GitTreebuilder,
        repo: *mut git_repository,
        source: *const GitTree,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!builder_p.is_null(), "builder_p", -1);
            git_assert_arg!(!repo.is_null(), "repo", -1);
            // SAFETY: the caller's contract.
            match unsafe { GitTreebuilder::new_raw(repo, source) } {
                Ok(b) => {
                    // SAFETY: writable (contract).
                    unsafe { builder_p.write(b) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_treebuilder_insert`.
    ///
    /// # Safety
    ///
    /// `entry_out` NULL or writable; `bld` NULL or a live builder; `filename`
    /// NULL or NUL-terminated; `id` NULL or readable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_treebuilder_insert(
        entry_out: *mut *const GitTreeEntry,
        bld: *mut GitTreebuilder,
        filename: *const c_char,
        id: *const Oid,
        filemode: c_uint,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!bld.is_null(), "bld", -1);
            git_assert_arg!(!id.is_null(), "id", -1);
            git_assert_arg!(!filename.is_null(), "filename", -1);
            // SAFETY: the caller's contract; the builder is used by nothing
            // else during the call (no C is called back with it).
            let r = unsafe { (*bld).insert(CStr::from_ptr(filename), &id.read(), filemode) };
            match r {
                Ok(e) => {
                    if !entry_out.is_null() {
                        // SAFETY: writable (contract).
                        unsafe { entry_out.write(e) };
                    }
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_treebuilder_get`.
    ///
    /// # Safety
    ///
    /// `bld` NULL or a live builder; `filename` NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_treebuilder_get(bld: *mut GitTreebuilder, filename: *const c_char) -> *const GitTreeEntry {
        barrier(|| {
            git_assert_arg!(!bld.is_null(), "bld", ptr::null());
            git_assert_arg!(!filename.is_null(), "filename", ptr::null());
            // SAFETY: the caller's contract.
            unsafe { (*bld).get(CStr::from_ptr(filename)) }.map_or(ptr::null(), |e| e.cast_const())
        })
    }

    /// `git_treebuilder_remove`.
    ///
    /// # Safety
    ///
    /// As [`git_treebuilder_get`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_treebuilder_remove(bld: *mut GitTreebuilder, filename: *const c_char) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract.
            let found = unsafe { git_treebuilder_get(bld, filename) };
            // SAFETY: NULL or NUL-terminated.
            let name = unsafe { crate::commit::opt_cbytes(filename) };
            if found.is_null() {
                return tree_error("failed to remove entry: file isn't in the tree", name).raw();
            }
            // SAFETY: a live builder; NUL-terminated name.
            unsafe { (*bld).remove_found(CStr::from_ptr(filename)) };
            0
        })
    }

    /// `git_treebuilder_write`.
    ///
    /// # Safety
    ///
    /// `oid` NULL or writable; `bld` NULL or a live builder.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_treebuilder_write(oid: *mut Oid, bld: *mut GitTreebuilder) -> c_int {
        barrier(|| {
            git_assert_arg!(!oid.is_null(), "oid", -1);
            git_assert_arg!(!bld.is_null(), "bld", -1);
            // SAFETY: a live builder whose repository is live.
            match unsafe { (*bld).write() } {
                Ok(id) => {
                    // SAFETY: writable (contract).
                    unsafe { oid.write(id) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_treebuilder_filter`: remove the entries for which `filter`
    /// returns non-zero.
    ///
    /// # Safety
    ///
    /// `bld` NULL or a live builder; `filter` callable with `payload`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_treebuilder_filter(
        bld: *mut GitTreebuilder,
        filter: Option<unsafe extern "C" fn(entry: *const GitTreeEntry, payload: *mut c_void) -> c_int>,
        payload: *mut c_void,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!bld.is_null(), "bld", -1);
            let Some(filter) = filter else {
                return errors::assert_arg_failed("filter").raw();
            };
            let mut it: HashMapIter = 0;
            loop {
                // No reference to the builder is held across the callback.
                // SAFETY: a live builder.
                let Some((key, entry)) = (unsafe { (*bld).map.iterate(&mut it) }).map(|(k, v)| (*k, *v)) else {
                    break;
                };
                // SAFETY: the callback's contract (the caller's).
                if unsafe { filter(entry, payload) } != 0 {
                    // SAFETY: a live builder; the key is the entry's name,
                    // still in the map.
                    unsafe {
                        let b = &mut *bld;
                        if let Some((_, e)) = b.map.remove(&key) {
                            alloc::free(e.cast());
                        }
                    }
                }
            }
            0
        })
    }

    /// `git_treebuilder_clear`.
    ///
    /// # Safety
    ///
    /// `bld` NULL or a live builder.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_treebuilder_clear(bld: *mut GitTreebuilder) -> c_int {
        barrier(|| {
            git_assert_arg!(!bld.is_null(), "bld", -1);
            // SAFETY: a live builder.
            unsafe { (*bld).clear() };
            0
        })
    }

    /// `git_treebuilder_free`.
    ///
    /// # Safety
    ///
    /// `bld` NULL or a live builder, not used after.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_treebuilder_free(bld: *mut GitTreebuilder) {
        // SAFETY: the caller's contract.
        barrier(|| unsafe { free_raw(bld) })
    }

    /// `git_tree_entry_bypath`.
    ///
    /// # Safety
    ///
    /// `entry_out` writable; `root` a live tree; `path` NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_entry_bypath(
        entry_out: *mut *mut GitTreeEntry,
        root: *const GitTree,
        path: *const c_char,
    ) -> c_int {
        barrier(|| {
            if root.is_null() || path.is_null() || entry_out.is_null() {
                return errors::assert_arg_failed("root").raw();
            }
            // SAFETY: a live tree: its repository is live.
            let repo = unsafe { (*root).object.repo };
            let mut lookup = |id: &Oid| -> Result<*mut GitTree> {
                let mut t = ptr::null_mut();
                // SAFETY: `repo` live; `t` writable.
                ErrorCode::check(unsafe { pend::git_tree_lookup(&mut t, repo, id) })?;
                Ok(t)
            };
            // SAFETY: trees from `lookup`, released once.
            let mut free = |t: *mut GitTree| unsafe { pend::git_tree_free(t) };
            // SAFETY: the caller's contract.
            match unsafe { entry_bypath(root, cstr_bytes(path), &mut lookup, &mut free) } {
                Ok(e) => {
                    // SAFETY: writable (contract).
                    unsafe { entry_out.write(e) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_tree_walk`.
    ///
    /// # Safety
    ///
    /// `tree` a live tree; `callback` callable with `payload`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_walk(
        tree: *const GitTree,
        mode: c_int,
        callback: Option<unsafe extern "C" fn(root: *const c_char, entry: *const GitTreeEntry, payload: *mut c_void) -> c_int>,
        payload: *mut c_void,
    ) -> c_int {
        barrier(|| {
            if mode != 0 && mode != 1 {
                let _ = git_error_set!(ErrorClass::INVALID, "invalid walking mode for tree walk");
                return -1;
            }
            let Some(callback) = callback else {
                return errors::assert_arg_failed("callback").raw();
            };
            git_assert!(!tree.is_null(), "tree", -1);
            // SAFETY: a live tree: its repository is live.
            let repo = unsafe { (*tree).object.repo };
            let mut lookup = |id: &Oid| -> Result<*mut GitTree> {
                let mut t = ptr::null_mut();
                // SAFETY: `repo` live; `t` writable.
                ErrorCode::check(unsafe { pend::git_tree_lookup(&mut t, repo, id) })?;
                Ok(t)
            };
            // SAFETY: trees from `lookup`, released once.
            let mut free = |t: *mut GitTree| unsafe { pend::git_tree_free(t) };
            let mut cb = |root: &GitStr, entry: &GitTreeEntry| {
                // SAFETY: the callback's contract (the caller's); `root` is
                // NUL-terminated (a `git_str`'s buffer), `entry` a copy of a
                // live entry, valid for the call.
                match unsafe { callback(root.as_ptr(), entry, payload) } {
                    e if e < 0 => WalkResult::Stop(e),
                    e if e > 0 => WalkResult::Skip,
                    _ => WalkResult::Continue,
                }
            };
            let mut root_path = GitStr::new();
            // SAFETY: the caller's contract.
            unsafe { walk(tree, &mut root_path, mode == 0, &mut lookup, &mut free, &mut cb) }
        })
    }

    /// `git_tree_create_updated`.
    ///
    /// # Safety
    ///
    /// `out` writable; `repo` live; `baseline` NULL or a live tree;
    /// `updates` holds `nupdates` valid updates.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_tree_create_updated(
        out: *mut Oid,
        repo: *mut git_repository,
        baseline: *mut GitTree,
        nupdates: usize,
        updates: *const GitTreeUpdate,
    ) -> c_int {
        barrier(|| {
            let ups = if nupdates == 0 {
                &[][..]
            } else {
                // SAFETY: `nupdates` readable updates (contract).
                unsafe { core::slice::from_raw_parts(updates, nupdates) }
            };
            let mut id = Oid::default();
            // SAFETY: the caller's contract.
            match unsafe { create_updated(&mut id, repo, baseline, ups) } {
                Ok(()) => {
                    // SAFETY: writable (contract).
                    unsafe { out.write(id) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// The object id of a tree, for the safe API.
    ///
    /// # Safety
    ///
    /// `tree` is a live tree.
    pub(crate) unsafe fn tree_id(tree: *const GitTree) -> Oid {
        // SAFETY: the caller's contract.
        unsafe { object_id(tree) }.unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;

crate::c_layout_test! {
    headers: ["tree.h"],
    types: [
        GitTreeEntry => "git_tree_entry" { attr, filename_len, oid, filename },
        GitTree => "git_tree" { object, odb_obj, entries },
        GitTreeUpdate => "git_tree_update" { action, id, filemode, path },
    ],
    asserts: [
        "GIT_FILEMODE_BLOB_GROUP_WRITABLE == 0100664",
        "GIT_TREE_UPDATE_UPSERT == 0", "GIT_TREE_UPDATE_REMOVE == 1",
        "GIT_TREEWALK_PRE == 0", "GIT_TREEWALK_POST == 1",
    ],
}
