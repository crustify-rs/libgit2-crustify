//! The index file format (`parse_index`, `write_index` and their helpers in
//! `index.c`): a header (`DIRC`, version, entry count), the entries
//! (versions 2 and 3 padded to 8 bytes, version 4 with prefix-compressed
//! paths), the extensions (`TREE`, `REUC`, `NAME`, and unknown optional
//! ones, kept to be written back), and the checksum of what precedes it
//! (SHA-1 or SHA-256, by the index's id type; zero when `index.skipHash`
//! wrote it).
//!
//! Parsing works on a [`ParseState`] that the index lends out (its emptied
//! entries, map, pool and vectors), so that the path check of each entry,
//! which runs C, is made with no reference into the index; the caller
//! installs the state back, as far as it got, success or not (the C left
//! what it had read in the index).

use core::ffi::c_uint;
use core::ptr;
use std::borrow::Cow;

use super::map::EntryMap;
use super::{
    EntryPtr, EntryRef, Extension, GIT_INDEX_ENTRY_EXTENDED, GIT_INDEX_ENTRY_EXTENDED_FLAGS,
    GIT_INDEX_ENTRY_NAMEMASK, GitIndex, GitIndexEntry, GitIndexNameEntry,
    INDEX_VERSION_NUMBER_COMP, INDEX_VERSION_NUMBER_EXT, INDEX_VERSION_NUMBER_LB,
    INDEX_VERSION_NUMBER_UB, NewEntry, as_entry, entry_alloc, epath, name_free, reuc_alloc,
    reuc_free, sig_cmp, sort_entries,
};
use crate::oid::{Oid, OidType};
use crate::tree_cache::{self, GitTreeCache};
use crate::util::alloc;
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::filebuf::GitFilebuf;
use crate::util::hash::{self, HashAlgorithm};
use crate::util::pool::GitPool;
use crate::util::str::GitStr;
use crate::util::util::bsearch;
use crate::util::varint;
use crate::util::vector::GitVector;
use crate::{git_assert, git_error_set};

/// `INDEX_HEADER_SIZE`.
const INDEX_HEADER_SIZE: usize = 12;
/// `INDEX_HEADER_SIG` ("DIRC").
const INDEX_HEADER_SIG: u32 = 0x4449_5243;
/// The extension signatures the index understands.
const TREECACHE_SIG: &[u8; 4] = b"TREE";
const UNMERGED_SIG: &[u8; 4] = b"REUC";
const CONFLICT_NAME_SIG: &[u8; 4] = b"NAME";
/// `sizeof(struct entry_common)`.
const ENTRY_COMMON_SIZE: usize = 40;

/// `index_error_invalid`.
fn invalid(message: &str) -> ErrorCode {
    git_error_set!(ErrorClass::INDEX, "invalid data in index - {message}")
}

/// The hash algorithm of an id type.
fn algorithm(t: OidType) -> HashAlgorithm {
    match t {
        OidType::Sha1 => HashAlgorithm::Sha1,
        OidType::Sha256 => HashAlgorithm::Sha256,
    }
}

/// `index_entry_path_offset`: where an on-disk entry's path starts.
#[inline]
fn path_offset(t: OidType, flags: u16) -> usize {
    ENTRY_COMMON_SIZE
        + t.size()
        + if flags & GIT_INDEX_ENTRY_EXTENDED != 0 {
            4
        } else {
            2
        }
}

/// `index_entry_size`: the on-disk size of an entry (0 on overflow).
fn entry_size(path_len: usize, varint_len: usize, t: OidType, flags: u16) -> usize {
    let offset = path_offset(t, flags);
    if varint_len != 0 {
        offset
            .checked_add(path_len)
            .and_then(|s| s.checked_add(1))
            .and_then(|s| s.checked_add(varint_len))
            .unwrap_or(0)
    } else {
        offset
            .checked_add(path_len)
            .and_then(|s| s.checked_add(8))
            .map_or(0, |s| s & !7)
    }
}

#[inline(always)]
fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

#[inline(always)]
fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

/// `strlen` from `at`, the end of `b` standing for the NUL that ends the
/// file's buffer (C reads the `git_str`'s terminator).
#[inline(always)]
fn strlen_at(b: &[u8], at: usize) -> usize {
    match b.get(at..) {
        Some(rest) => memchr::memchr(0, rest).unwrap_or(rest.len()),
        None => 0,
    }
}

/// What the parser fills: the index's state, lent out (see the module
/// documentation).
pub struct ParseState {
    /// The index's id type.
    pub oid_type: OidType,
    /// The file's version, once its header is read.
    pub version: Option<c_uint>,
    /// `entries`.
    pub entries: GitVector,
    pub(crate) map: EntryMap,
    /// `tree`, in `tree_pool`.
    pub tree: *mut GitTreeCache,
    /// `tree_pool`.
    pub tree_pool: GitPool,
    /// `reuc`.
    pub reuc: GitVector,
    /// `names`.
    pub names: GitVector,
    /// `extensions`.
    pub extensions: Vec<Extension>,
    /// The checksum, once verified.
    pub checksum: Option<[u8; hash::MAX_SIZE]>,
}

impl GitIndex {
    /// Lend out the state the parser fills (the index keeps empty ones).
    pub(crate) fn take_parse_state(&mut self) -> ParseState {
        ParseState {
            oid_type: self.oid_kind(),
            version: None,
            entries: core::mem::take(&mut self.entries),
            map: core::mem::take(&mut self.entries_map),
            tree: core::mem::replace(&mut self.tree, ptr::null_mut()),
            tree_pool: core::mem::take(&mut self.tree_pool),
            reuc: core::mem::take(&mut self.reuc),
            names: core::mem::take(&mut self.names),
            extensions: core::mem::take(&mut self.extensions),
            checksum: None,
        }
    }

    /// Take the state back (after [`parse`]).
    pub(crate) fn put_parse_state(&mut self, st: ParseState) {
        if let Some(v) = st.version {
            self.version = v;
        }
        self.entries = st.entries;
        self.entries_map = st.map;
        self.tree = st.tree;
        self.tree_pool = st.tree_pool;
        self.reuc = st.reuc;
        self.names = st.names;
        self.extensions = st.extensions;
        if let Some(c) = st.checksum {
            let n = algorithm(self.oid_kind()).size();
            self.checksum[..n].copy_from_slice(&c[..n]);
            // Entries are stored case-sensitively on disk, so re-sort now
            // if in-memory index is supposed to be case-insensitive
            let icase = self.ignore_case();
            self.entries.set_sorted(!icase);
            // SAFETY: the index's entries, which the parse just made: live
            // (the invariant), and only the sort touches them.
            unsafe { sort_entries(&mut self.entries, icase) };
            self.set_dirty(false);
        }
    }
}

/// An entry as read from the file, before its path is checked.
struct RawEntry {
    entry: GitIndexEntry,
    size: usize,
}

/// `parse_index`: parse `buf` (the whole file) into `st`. `validate`
/// checks each path (`git_path_is_valid` with the index's defaults); it is
/// the only foreign code the parser runs.
pub fn parse(
    st: &mut ParseState,
    buf: &[u8],
    validate: &mut dyn FnMut(&[u8]) -> bool,
) -> Result<()> {
    let alg = algorithm(st.oid_type);
    let checksum_size = alg.size();
    if buf.len() < INDEX_HEADER_SIZE + checksum_size {
        return Err(invalid("insufficient buffer space"));
    }
    // Precalculate the hash of the files's contents -- we'll match
    // it to the provided checksum in the footer.
    let computed = hash::hash_buf(&buf[..buf.len() - checksum_size], alg).ok();

    // read_header
    if be32(buf, 0) != INDEX_HEADER_SIG {
        return Err(invalid("incorrect header signature"));
    }
    let version = be32(buf, 4);
    if !(INDEX_VERSION_NUMBER_LB..=INDEX_VERSION_NUMBER_UB).contains(&version) {
        return Err(invalid("incorrect header version"));
    }
    let entry_count = be32(buf, 8);
    st.version = Some(version);
    let compressed = version >= INDEX_VERSION_NUMBER_COMP;

    let mut pos = INDEX_HEADER_SIZE;
    git_assert!(st.entries.is_empty(), "!index->entries.length");
    st.map.resize(entry_count as usize)?;

    // the previous path (version 4), as an offset into the entries
    let mut last: Option<EntryPtr> = None;
    let mut i = 0u32;
    while i < entry_count && buf.len() - pos > checksum_size {
        let rest = &buf[pos..];
        let last_path: Option<&[u8]> = if compressed {
            Some(last.map_or(&b""[..], |e| {
                // SAFETY: `last` is an entry the state holds, not written
                // here.
                unsafe { epath(e) }
            }))
        } else {
            None
        };
        let (raw, path) = match read_entry(rest, st.oid_type, checksum_size, last_path) {
            Ok(r) => r,
            Err(_) => return Err(invalid("invalid entry")),
        };
        // `index_entry_dup`: the path check, then the copy
        if !validate(&path) {
            let _ = git_error_set!(ErrorClass::INDEX, "invalid path: '{}'", Bytes(&path));
            return Err(invalid("invalid entry"));
        }
        let mut entry = match entry_alloc(&path) {
            Ok(e) => e,
            Err(_) => return Err(invalid("invalid entry")),
        };
        entry.copy_from(&raw.entry);
        // not inserted: `entry` is freed (dropped)
        st.entries.insert(entry.as_raw().as_ptr().cast())?;
        // the vector holds it now
        let entry = entry.into_raw();
        // SAFETY: the entry stays live while the state (then the index)
        // holds it in the map: it is freed only after leaving the map.
        if let Err(e) = unsafe { st.map.put(entry) } {
            // C freed the entry the vector still held (see DEFECTS.md)
            st.entries.pop();
            // SAFETY: taken out of the vector again (the last element,
            // just pushed), in no map: ours alone.
            drop(unsafe { NewEntry::from_raw(entry) });
            return Err(e);
        }
        if compressed {
            last = Some(entry);
        }
        // seek_forward
        if raw.size >= buf.len() - pos {
            return Err(invalid("ran out of data while parsing"));
        }
        pos += raw.size;
        i += 1;
    }
    if i != entry_count {
        return Err(invalid("header entries changed while parsing"));
    }

    // There's still space for some extensions!
    while buf.len() - pos > checksum_size {
        let size = read_extension(st, &buf[pos..], checksum_size)?;
        if size >= buf.len() - pos {
            return Err(invalid("ran out of data while parsing"));
        }
        pos += size;
    }
    if buf.len() - pos != checksum_size {
        return Err(invalid("buffer size does not match index footer size"));
    }
    let footer = &buf[pos..];
    let computed = computed.ok_or(ErrorCode::GENERIC)?;
    if footer.iter().any(|&b| b != 0) && footer != computed.as_bytes() {
        return Err(invalid("calculated checksum does not match expected"));
    }
    let mut c = [0u8; hash::MAX_SIZE];
    c[..checksum_size].copy_from_slice(computed.as_bytes());
    st.checksum = Some(c);
    Ok(())
}

/// `read_entry` up to `index_entry_dup`: the entry's fields, its size on
/// disk, and its path. `rest` starts at the entry and runs to the end of
/// the file.
fn read_entry<'r>(
    rest: &'r [u8],
    t: OidType,
    checksum_size: usize,
    last: Option<&[u8]>,
) -> core::result::Result<(RawEntry, Cow<'r, [u8]>), ()> {
    let oid_size = t.size();
    let minimal = path_offset(t, 0);
    if checksum_size + minimal > rest.len() {
        return Err(());
    }
    let mut entry = GitIndexEntry::zeroed();
    entry.ctime.seconds = be32(rest, 0) as i32;
    entry.ctime.nanoseconds = be32(rest, 4);
    entry.mtime.seconds = be32(rest, 8) as i32;
    entry.mtime.nanoseconds = be32(rest, 12);
    entry.dev = be32(rest, 16);
    entry.ino = be32(rest, 20);
    entry.mode = be32(rest, 24);
    entry.uid = be32(rest, 28);
    entry.gid = be32(rest, 32);
    entry.file_size = be32(rest, 36);
    entry.id =
        Oid::from_bytes(&rest[ENTRY_COMMON_SIZE..ENTRY_COMMON_SIZE + oid_size], t).ok_or(())?;
    entry.flags = be16(rest, ENTRY_COMMON_SIZE + oid_size);
    let poff = path_offset(t, entry.flags);
    if entry.flags & GIT_INDEX_ENTRY_EXTENDED != 0 {
        entry.flags_extended = be16(rest, ENTRY_COMMON_SIZE + oid_size + 2);
    }

    let (path, size) = match last {
        None => {
            let mut path_length = usize::from(entry.flags & GIT_INDEX_ENTRY_NAMEMASK);
            // if this is a very long string, we must find its real length
            // without overflowing
            if path_length == 0xFFF {
                match memchr::memchr(0, &rest[poff..]) {
                    Some(n) => path_length = n,
                    None => {
                        let _ = invalid("invalid path name");
                        return Err(());
                    }
                }
            }
            let size = entry_size(path_length, 0, t, entry.flags);
            // `entry.path` is the bytes at `poff`, up to their NUL
            let n = strlen_at(rest, poff);
            (Cow::Borrowed(&rest[poff..poff + n]), size)
        }
        Some(last) => {
            let vbytes = &rest[poff..];
            // past the end, C reads the buffer's terminating NUL
            let (strip_len, varint_len) = varint::decode(vbytes).unwrap_or_else(|| {
                let mut v = vbytes.to_vec();
                v.push(0);
                varint::decode(&v).unwrap_or((0, 0))
            });
            if varint_len == 0 || (last.len() as u64) < strip_len {
                let _ = invalid("incorrect prefix length");
                return Err(());
            }
            let prefix_len = last.len() - strip_len as usize;
            let suffix_len = strlen_at(rest, poff + varint_len);
            let path_len = prefix_len
                .checked_add(suffix_len)
                .and_then(|n| n.checked_add(1))
                .ok_or(())?;
            if path_len > crate::util::posix::GIT_PATH_MAX {
                let _ = invalid("unreasonable path length");
                return Err(());
            }
            let mut p = Vec::new();
            p.try_reserve_exact(path_len).map_err(|_| ())?;
            p.extend_from_slice(&last[..prefix_len]);
            let s = poff + varint_len;
            if suffix_len > 0 {
                p.extend_from_slice(&rest[s..s + suffix_len]);
            }
            (
                Cow::Owned(p),
                entry_size(suffix_len, varint_len, t, entry.flags),
            )
        }
    };
    if size == 0 {
        return Err(());
    }
    if checksum_size + size > rest.len() {
        let _ = git_error_set!(ErrorClass::INTERNAL, "invalid index checksum");
        return Err(());
    }
    Ok((RawEntry { entry, size }, path))
}

/// `read_extension`: the extension at the start of `rest`; its size.
fn read_extension(st: &mut ParseState, rest: &[u8], checksum_size: usize) -> Result<usize> {
    let sig: [u8; 4] = [rest[0], rest[1], rest[2], rest[3]];
    let ext_size = be32(rest, 4) as usize;
    let total_size = ext_size + 8;
    if rest.len() < total_size || rest.len() - total_size < checksum_size {
        return Err(invalid("extension is truncated"));
    }
    let data = &rest[8..total_size];
    // `%.4s`
    let sig_str = &sig[..memchr::memchr(0, &sig).unwrap_or(4)];
    if sig[0].is_ascii_uppercase() {
        if &sig == TREECACHE_SIG {
            tree_cache::read_into(&mut st.tree, data, st.oid_type, &mut st.tree_pool)?;
        } else if &sig == UNMERGED_SIG {
            read_reuc(st, data)?;
        } else if &sig == CONFLICT_NAME_SIG {
            read_conflict_names(st, data)?;
        } else {
            // Unknown extension; keep a copy of this.
            let mut v = Vec::new();
            v.try_reserve_exact(data.len()).map_err(|_| errors::oom())?;
            v.extend_from_slice(data);
            match bsearch(&st.extensions, |x| sig_cmp(&sig, &x.signature)) {
                Ok(_) => {
                    // `extension_ondup`: the duplicate is dropped (C leaked
                    // it), with the error set, and the read goes on.
                    let _ = git_error_set!(
                        ErrorClass::INDEX,
                        "found duplicate extension '{}'",
                        Bytes(sig_str)
                    );
                }
                Err(p) => {
                    if st.extensions.try_reserve(1).is_ok() {
                        st.extensions.insert(
                            p,
                            Extension {
                                signature: sig,
                                data: v,
                            },
                        );
                    }
                }
            }
        }
    } else {
        // we cannot handle non-ignorable extensions
        return Err(git_error_set!(
            ErrorClass::INDEX,
            "unsupported mandatory extension: '{}'",
            Bytes(sig_str)
        ));
    }
    Ok(total_size)
}

/// `read_reuc`.
fn read_reuc(st: &mut ParseState, mut buf: &[u8]) -> Result<()> {
    let oid_size = st.oid_type.size();
    while !buf.is_empty() {
        let len = memchr::memchr(0, buf).unwrap_or(buf.len()) + 1;
        if buf.len() <= len {
            return Err(invalid("reading reuc entries"));
        }
        let Some(lost) = reuc_alloc(&buf[..len - 1]) else {
            return Err(errors::oom());
        };
        let fail = |msg: &str| {
            // SAFETY: ours, not inserted.
            unsafe { reuc_free(lost.as_ptr().cast()) };
            Err(invalid(msg))
        };
        buf = &buf[len..];
        // read 3 ASCII octal numbers for stage entries
        for i in 0..3 {
            let parsed = crate::util::util::strntol64(buf, 8);
            let (tmp, used) = match parsed {
                // (at the end of the data, C read the next byte of the file
                // for `*endptr`; whatever it is, a check fails with the
                // same message)
                Ok((tmp, used))
                    if used > 0
                        && buf.get(used) == Some(&0)
                        && (0..=i64::from(u32::MAX)).contains(&tmp) =>
                {
                    (tmp, used)
                }
                _ => return fail("reading reuc entry stage"),
            };
            // SAFETY: ours.
            unsafe { (*lost.as_ptr()).mode[i] = tmp as u32 };
            let len = used + 1;
            if buf.len() <= len {
                return fail("reading reuc entry stage");
            }
            buf = &buf[len..];
        }
        // read up to 3 OIDs for stage entries
        for i in 0..3 {
            // SAFETY: ours.
            if unsafe { (*lost.as_ptr()).mode[i] } == 0 {
                continue;
            }
            if buf.len() < oid_size {
                return fail("reading reuc entry oid");
            }
            let oid = Oid::from_bytes(&buf[..oid_size], st.oid_type).unwrap_or_default();
            // SAFETY: ours.
            unsafe { (*lost.as_ptr()).oid[i] = oid };
            buf = &buf[oid_size..];
        }
        // entry was read successfully - insert into reuc vector
        if let Err(e) = st.reuc.insert(lost.as_ptr().cast()) {
            // SAFETY: not inserted (C leaked it).
            unsafe { reuc_free(lost.as_ptr().cast()) };
            return Err(e);
        }
    }
    // entries are guaranteed to be sorted on-disk
    st.reuc.set_sorted(true);
    Ok(())
}

/// `read_conflict_names`.
fn read_conflict_names(st: &mut ParseState, mut buf: &[u8]) -> Result<()> {
    while !buf.is_empty() {
        let ne = alloc::calloc(1, size_of::<GitIndexNameEntry>()).cast::<GitIndexNameEntry>();
        if ne.is_null() {
            return Err(ErrorCode::GENERIC);
        }
        let mut fields: [*mut core::ffi::c_char; 3] = [ptr::null_mut(); 3];
        let mut failure = None;
        for f in &mut fields {
            let len = memchr::memchr(0, buf).unwrap_or(buf.len()) + 1;
            if buf.len() < len {
                failure = Some(invalid("reading conflict name entries"));
                break;
            }
            if len > 1 {
                let p = alloc::malloc(len).cast::<u8>();
                if p.is_null() {
                    failure = Some(ErrorCode::GENERIC);
                    break;
                }
                // SAFETY: `p` has `len` bytes; `buf` has them too.
                unsafe { ptr::copy_nonoverlapping(buf.as_ptr(), p, len) };
                *f = p.cast();
            }
            buf = &buf[len..];
        }
        // SAFETY: `ne` is fresh, zeroed and ours.
        let n = unsafe { &mut *ne };
        n.ancestor = fields[0];
        n.ours = fields[1];
        n.theirs = fields[2];
        if failure.is_none()
            && let Err(e) = st.names.insert(ne.cast())
        {
            failure = Some(e);
        }
        if let Some(e) = failure {
            // SAFETY: ours, not inserted.
            unsafe { name_free(ne) };
            return Err(e);
        }
    }
    // entries are guaranteed to be sorted on-disk
    st.names.set_sorted(true);
    Ok(())
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// `is_index_extended`: set each entry's `EXTENDED` flag from its extended
/// flags; whether any has them.
fn is_index_extended(ix: &mut GitIndex) -> bool {
    let mut extended = 0usize;
    for e in ix.entries.iter() {
        // SAFETY: a live entry of the index, not otherwise borrowed.
        let ent = unsafe { &mut (*as_entry(e).as_ptr()).entry };
        ent.flags &= !GIT_INDEX_ENTRY_EXTENDED;
        if ent.flags_extended & GIT_INDEX_ENTRY_EXTENDED_FLAGS != 0 {
            extended += 1;
            ent.flags |= GIT_INDEX_ENTRY_EXTENDED;
        }
    }
    extended > 0
}

/// `write_disk_entry`.
fn write_disk_entry(
    file: &mut GitFilebuf,
    t: OidType,
    entry: EntryRef<'_>,
    last: Option<&[u8]>,
) -> Result<()> {
    let (e, path) = (entry.entry(), entry.path());
    let mut path_len = path.len();
    let mut same_len = 0;
    let mut varint_len = 0usize;
    if let Some(last) = last {
        same_len = path.iter().zip(last).take_while(|(a, b)| a == b).count();
        path_len -= same_len;
        varint_len = varint::encode(None, (last.len() - same_len) as u64) as usize;
    }
    let mut disk_size = entry_size(path_len, varint_len, t, e.flags);
    if disk_size == 0 {
        return Err(ErrorCode::GENERIC);
    }
    let mem = file.reserve(disk_size)?;
    if mem.len() < disk_size {
        // a filebuf without a buffer (C would have written through NULL)
        return Err(ErrorCode::GENERIC);
    }
    mem.fill(0);
    // Yes, we have to truncate (the on-disk times and size are 32 bits).
    let put32 = |m: &mut [u8], at: usize, v: u32| m[at..at + 4].copy_from_slice(&v.to_be_bytes());
    put32(mem, 0, e.ctime.seconds as u32);
    put32(mem, 4, e.ctime.nanoseconds);
    put32(mem, 8, e.mtime.seconds as u32);
    put32(mem, 12, e.mtime.nanoseconds);
    put32(mem, 16, e.dev);
    put32(mem, 20, e.ino);
    put32(mem, 24, e.mode);
    put32(mem, 28, e.uid);
    put32(mem, 32, e.gid);
    put32(mem, 36, e.file_size);
    let oid_size = t.size();
    mem[ENTRY_COMMON_SIZE..ENTRY_COMMON_SIZE + oid_size]
        .copy_from_slice(&e.id.raw_id()[..oid_size]);
    let foff = ENTRY_COMMON_SIZE + oid_size;
    mem[foff..foff + 2].copy_from_slice(&e.flags.to_be_bytes());
    let poff = path_offset(t, e.flags);
    if e.flags & GIT_INDEX_ENTRY_EXTENDED != 0 {
        let fx = e.flags_extended & GIT_INDEX_ENTRY_EXTENDED_FLAGS;
        mem[foff + 2..foff + 4].copy_from_slice(&fx.to_be_bytes());
    }
    let mut at = poff;
    disk_size -= poff;
    if let Some(last) = last {
        let n = varint::encode(Some(&mut mem[at..]), (last.len() - same_len) as u64);
        git_assert!(n > 0, "varint_len > 0");
        at += n as usize;
        disk_size -= n as usize;
        // If using path compression, we are not allowed to have additional
        // trailing NULs.
        git_assert!(disk_size == path_len + 1, "disk_size == path_len + 1");
    } else {
        // If no path compression is used, we do have NULs as padding.
        git_assert!(disk_size > path_len, "disk_size > path_len");
    }
    mem[at..at + path_len].copy_from_slice(&path[same_len..]);
    mem[at + path_len] = 0;
    Ok(())
}

/// `write_entries`.
fn write_entries(ix: &mut GitIndex, file: &mut GitFilebuf) -> Result<()> {
    let t = ix.oid_kind();
    // If index->entries is sorted case-insensitively, then we need to
    // re-sort it case-sensitively before writing
    let case_sorted;
    let entries: &GitVector = if ix.ignore_case() {
        let mut v = ix.entries.dup(Some(super::ffi::git_index_entry_cmp))?;
        // SAFETY: a copy of the index's vector: its live entries (the
        // invariant), unwritten while `ix` is borrowed.
        unsafe { sort_entries(&mut v, false) };
        case_sorted = v;
        &case_sorted
    } else {
        &ix.entries
    };
    let compressed = ix.version >= INDEX_VERSION_NUMBER_COMP;
    let mut last: Option<EntryRef<'_>> = None;
    for e in entries.iter() {
        // SAFETY: the index's entries (or a copy of its vector): live (the
        // invariant), and unwritten while `ix` is borrowed.
        let e = unsafe { EntryRef::new(as_entry(e)) };
        let last_path = compressed.then(|| last.map_or(&b""[..], EntryRef::path));
        write_disk_entry(file, t, e, last_path)?;
        if compressed {
            last = Some(e);
        }
    }
    Ok(())
}

/// `write_extension`.
fn write_extension(file: &mut GitFilebuf, signature: &[u8; 4], data: &[u8]) -> Result<()> {
    let mut hdr = [0u8; 8];
    hdr[..4].copy_from_slice(signature);
    hdr[4..].copy_from_slice(&(data.len() as u32).to_be_bytes());
    let _ = file.write(&hdr);
    file.write(data)
}

/// `create_name_extension_data` of every NAME entry.
fn name_extension_data(ix: &GitIndex, out: &mut GitStr) -> Result<()> {
    for ne in ix.names.iter() {
        let ne = ne.cast::<GitIndexNameEntry>();
        // SAFETY: the index's name entries.
        let n = unsafe { &*ne };
        let fields = [n.ancestor, n.ours, n.theirs];
        for f in fields {
            // SAFETY: NULL or a string the entry owns.
            match unsafe { crate::util::export::cstr(f) } {
                None => out.put(b"\0")?,
                Some(s) => out.put(s.to_bytes_with_nul())?,
            }
        }
    }
    Ok(())
}

/// `create_reuc_extension_data` of every REUC entry.
fn reuc_extension_data(ix: &GitIndex, out: &mut GitStr) -> Result<()> {
    let oid_size = ix.oid_kind().size();
    for r in ix.reuc.iter() {
        let r = r.cast::<super::GitIndexReucEntry>();
        // SAFETY: the index's REUC entries.
        let path = unsafe { super::reuc_path(r) };
        // SAFETY: as above.
        let rr = unsafe { &*r };
        let (modes, oids) = (rr.mode, rr.oid);
        out.put(path)?;
        out.put(b"\0")?;
        for m in modes {
            out.printf(2, format_args!("{m:o}"))?;
            out.put(b"\0")?;
        }
        for i in 0..3 {
            if modes[i] != 0 {
                out.put(&oids[i].raw_id()[..oid_size])?;
            }
        }
    }
    Ok(())
}

/// `write_index`: write the whole file to `file` (still to commit), and
/// the checksum (its length is the id type's hash size).
pub(crate) fn write_index(
    ix: &mut GitIndex,
    file: &mut GitFilebuf,
) -> Result<[u8; hash::MAX_SIZE]> {
    git_assert!(ix.oid_type != 0, "index->oid_type");
    let checksum_size = algorithm(ix.oid_kind()).size();
    let version = if ix.version <= INDEX_VERSION_NUMBER_EXT {
        if is_index_extended(ix) {
            INDEX_VERSION_NUMBER_EXT
        } else {
            INDEX_VERSION_NUMBER_LB
        }
    } else {
        ix.version
    };
    let mut hdr = [0u8; INDEX_HEADER_SIZE];
    hdr[..4].copy_from_slice(&INDEX_HEADER_SIG.to_be_bytes());
    hdr[4..8].copy_from_slice(&version.to_be_bytes());
    hdr[8..].copy_from_slice(&(ix.entries.len() as u32).to_be_bytes());
    file.write(&hdr)?;

    write_entries(ix, file)?;

    // write the tree cache extension
    if !ix.tree.is_null() {
        let mut buf = GitStr::new();
        // SAFETY: the index's invariant: a tree in its pool.
        unsafe { tree_cache::write(&mut buf, ix.tree)? };
        write_extension(file, TREECACHE_SIG, buf.as_bytes())?;
    }
    // write the rename conflict extension
    if !ix.names.is_empty() {
        let mut buf = GitStr::new();
        name_extension_data(ix, &mut buf)?;
        write_extension(file, CONFLICT_NAME_SIG, buf.as_bytes())?;
    }
    // write the reuc extension
    if !ix.reuc.is_empty() {
        let mut buf = GitStr::new();
        reuc_extension_data(ix, &mut buf)?;
        write_extension(file, UNMERGED_SIG, buf.as_bytes())?;
    }
    // write any unknown extension
    for ext in &ix.extensions {
        write_extension(file, &ext.signature, &ext.data)?;
    }
    // get out the hash for all the contents we've appended to the file
    let mut checksum = [0u8; hash::MAX_SIZE];
    let _ = file.hash(&mut checksum);
    // write it at the end of the file
    file.write(&checksum[..checksum_size])?;
    // file entries are no longer up to date
    ix.clear_uptodate();
    Ok(checksum)
}
