//! Port of `src/libgit2/pack.c`: pack files and their indexes.
//!
//! A pack (`.pack`) holds zlib-deflated objects, some stored as deltas
//! against others; its index (`.idx`, version 1 or 2) maps object ids to
//! offsets in the pack. This module reads both: it finds objects in the
//! index (binary search over the fanout table, 31-bit and 64-bit
//! offsets), resolves delta chains (with a per-pack cache of delta bases),
//! inflates objects whole or as a stream, and lists the objects.
//!
//! # Types
//!
//! | C | Rust | Kind (PORTING §4.1) |
//! |---|---|---|
//! | `struct git_pack_file` | [`GitPackFile`] | shared prefix: the indexer reads and writes `mwf`, `has_cache`, `idx_cache`; midx/commit-graph read `pack_name`; a test reads `refcount`. The rest (index, delta base cache, locks) is Rust's, after the prefix |
//! | `struct git_pack_entry` | [`GitPackEntry`] | mirrored: the indexer allocates them |
//! | `git_pack_oidmap` | [`PackOidMap`] | mirrored: embedded in the pack, used by the indexer through the exported functions |
//! | `git_packfile_stream` | [`GitPackfileStream`] | mirrored: the indexer embeds one |
//! | `git_pack_cache`, `git_pack_cache_entry`, `pack_chain_elem` | [`PackCache`], `CacheEntry`, `ChainElem` | Rust-only (their C definitions are left in `pack.h` only for `USE_RUST=OFF`) |
//!
//! # Threads
//!
//! A pack is shared by every repository of the process that uses it (the
//! pack cache of `mwindow.rs`), and read from several threads at once. Its
//! index is opened once (under the pack's lock) and immutable after; its
//! file is opened under the pack's and the file's locks, which are taken in
//! that order; its delta base cache has its own lock. Reads of the mapped
//! index and pack go through borrowed slices, valid as long as the pack (for
//! the index) or the window cursor (for the pack) lives: see `mwindow.rs`
//! for what mapped memory assumes of other processes.
//!
//! The indexer's pack is the exception: the indexer (C) makes it with
//! [`packfile_alloc`], fills `idx_cache` and sets `has_cache`, and uses it
//! from its own thread only. The reads of `idx_cache` rely on that.

use core::cell::UnsafeCell;
use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::collections::HashMap;
use std::ffi::CString;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use crate::hashmap_oid::OidPtr;
use crate::mwindow::{self, Cursor, GitMwindow, GitMwindowFile};
use crate::oid::{self, GIT_OID_MAX_SIZE, Oid, OidType, git_oid_t};
use crate::util::alloc;
use crate::util::errors::{self, Bytes, ErrorClass, ErrorCode, Result};
use crate::util::futils;
use crate::util::hashmap::{CStrKey, GitHashMap, HashKey};
use crate::util::integer::alloc_add;
use crate::util::posix::{self, GitMap};
use crate::util::str::GitStr;
use crate::util::thread::{Atomic32, GitMutexGuard};
use crate::util::zstream::{GitZstream, ZstreamKind, ZstreamPtr};
use crate::{git_assert, git_assert_arg, git_error_set};

/// `GIT_OBJECT_INVALID`.
pub const GIT_OBJECT_INVALID: c_int = -1;
pub use crate::odb::GIT_OBJECT_COMMIT;
pub use crate::odb::GIT_OBJECT_TREE;
/// `GIT_OBJECT_BLOB`.
pub const GIT_OBJECT_BLOB: c_int = 3;
/// `GIT_OBJECT_TAG`.
pub const GIT_OBJECT_TAG: c_int = 4;
/// `GIT_PACKFILE_OFS_DELTA`.
pub const GIT_PACKFILE_OFS_DELTA: c_int = 6;
/// `GIT_PACKFILE_REF_DELTA`.
pub const GIT_PACKFILE_REF_DELTA: c_int = 7;

/// `PACK_SIGNATURE` ("PACK").
pub const PACK_SIGNATURE: u32 = 0x5041_434b;
/// `PACK_VERSION`.
pub const PACK_VERSION: u32 = 2;
/// `PACK_IDX_SIGNATURE` ("\377tOc").
pub const PACK_IDX_SIGNATURE: u32 = 0xff74_4f63;

/// `GIT_PACK_CACHE_MEMORY_LIMIT`.
pub const GIT_PACK_CACHE_MEMORY_LIMIT: usize = 16 * 1024 * 1024;
/// `GIT_PACK_CACHE_SIZE_LIMIT`: bases larger than this are not cached.
pub const GIT_PACK_CACHE_SIZE_LIMIT: usize = 1024 * 1024;

/// `SMALL_STACK_SIZE`: delta chains up to this long need no allocation.
const SMALL_STACK_SIZE: usize = 64;

/// `git_disable_pack_keep_file_checks` (`GIT_OPT_DISABLE_PACK_KEEP_FILE_CHECKS`),
/// a C `bool` that `settings.c` and a test write with plain stores.
#[unsafe(no_mangle)]
pub static git_disable_pack_keep_file_checks: AtomicBool = AtomicBool::new(false);

#[inline]
fn is_delta(t: c_int) -> bool {
    t == GIT_PACKFILE_OFS_DELTA || t == GIT_PACKFILE_REF_DELTA
}

#[inline]
fn be32(b: &[u8], at: usize) -> u32 {
    let w: [u8; 4] = b[at..at + 4].try_into().expect("4 bytes");
    u32::from_be_bytes(w)
}

/// `packfile_error`: `invalid pack file - <message>`, class `GIT_ERROR_ODB`.
#[cold]
fn packfile_error(message: &str) -> ErrorCode {
    git_error_set!(ErrorClass::ODB, "invalid pack file - {}", message)
}

/// `git_oid_raw_ncmp`: whether the first `len` hex digits of `a` and `b`
/// differ (1) or not (0). Reads `len.div_ceil(2)` bytes, as C does.
#[inline]
fn raw_ncmp(a: &[u8], b: &[u8], len: usize) -> bool {
    let len = len.min(oid::GIT_OID_MAX_HEXSIZE);
    let full = len / 2;
    if a[..full] != b[..full] {
        return true;
    }
    len & 1 == 1 && (a[full] ^ b[full]) & 0xf0 != 0
}

// ---------------------------------------------------------------------------
// Owned object data
// ---------------------------------------------------------------------------

/// Object data in `git__allocator`'s memory, as C gets it from the pack
/// reader: `len` bytes and a NUL, freed with `git__free` (on drop, or by C
/// after [`into_raw`](Self::into_raw)).
pub struct GitBytes {
    ptr: NonNull<u8>,
    len: usize,
}

// SAFETY: field by field: `ptr` is a `git__allocator` block this value
// owns alone (`into_raw` gives it up by value), which `git__free` may free
// on any thread; `len` is plain data. `&GitBytes` only reads the block
// (`as_bytes`), so shared use from several threads is reads only.
unsafe impl Send for GitBytes {}
// SAFETY: as for `Send` just above: `&self` methods only read.
unsafe impl Sync for GitBytes {}

impl GitBytes {
    /// A copy of `data`, NUL-terminated.
    pub fn copy_from(data: &[u8]) -> Result<GitBytes> {
        let n = alloc_add(data.len(), 1)?;
        let p = alloc::malloc(n).cast::<u8>();
        let ptr = NonNull::new(p).ok_or(ErrorCode::GENERIC)?;
        // SAFETY: `p` has `n = data.len() + 1` writable bytes, distinct
        // from `data`.
        unsafe { ptr::copy_nonoverlapping(data.as_ptr(), p, data.len()) };
        // SAFETY: as above: the last of them.
        unsafe { p.wrapping_add(data.len()).write(0) };
        Ok(GitBytes {
            ptr,
            len: data.len(),
        })
    }

    /// Adopt a buffer.
    ///
    /// # Safety
    ///
    /// `ptr` is a `git__allocator` allocation of at least `len + 1`
    /// initialized bytes, which the caller gives up.
    pub unsafe fn from_raw(ptr: NonNull<u8>, len: usize) -> GitBytes {
        GitBytes { ptr, len }
    }

    /// The bytes (without the NUL).
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `len` initialized bytes, owned (the type's invariant).
        unsafe { core::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }

    /// The length.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether it is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Give the buffer up: C frees it with `git__free`.
    pub fn into_raw(self) -> (*mut u8, usize) {
        let me = core::mem::ManuallyDrop::new(self);
        (me.ptr.as_ptr(), me.len)
    }
}

impl Drop for GitBytes {
    fn drop(&mut self) {
        // SAFETY: owned, from `git__allocator`.
        unsafe { alloc::free(self.ptr.as_ptr().cast()) }
    }
}

impl core::fmt::Debug for GitBytes {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GitBytes").field("len", &self.len).finish()
    }
}

/// `git_rawobj` (`odb.h`), mirrored: the out-parameter of
/// [`ffi::git_packfile_unpack`] (`odb.rs` keeps its raw objects in Rust
/// types and does not define it).
#[repr(C)]
pub struct GitRawobj {
    /// `data`.
    pub data: *mut c_void,
    /// `len`.
    pub len: usize,
    /// `type`.
    pub type_: c_int,
}

// ---------------------------------------------------------------------------
// The index
// ---------------------------------------------------------------------------

/// A private, read-only mapping, unmapped on drop.
struct Mapping(GitMap);

// SAFETY: the one field is a private read-only mapping this value owns,
// unmapped only by `Drop` (`&mut self`); a mapping is not tied to a thread,
// and through `&Mapping` it is only read (`bytes`).
unsafe impl Send for Mapping {}
// SAFETY: as for `Send` just above.
unsafe impl Sync for Mapping {}

impl Mapping {
    fn bytes(&self) -> &[u8] {
        // SAFETY: a live private read-only mapping of `len` bytes, which
        // nothing in the process writes (and which, by the assumption every
        // mmap reader makes, no other process rewrites: see `mwindow.rs`).
        unsafe { core::slice::from_raw_parts(self.0.data.cast::<u8>(), self.0.len) }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: the mapping `mmap_ro` made, unmapped once; references into
        // it borrow `self`, so none outlives this.
        unsafe { posix::munmap(&mut self.0) }
    }
}

/// The result of looking an id (prefix) up in an index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup {
    /// No object matches.
    NotFound,
    /// More than one object matches the prefix.
    Ambiguous,
    /// The object at this position of the index matches (uniquely).
    Found(u32),
}

/// A validated pack index (`.idx`, version 1 or 2) over its bytes: the
/// layout logic, apart from where the bytes live (a mapping, or a test's
/// vector).
#[derive(Clone, Copy)]
pub struct IndexView<'a> {
    data: &'a [u8],
    version: u32,
    num_objects: u32,
    oid_size: usize,
}

impl<'a> IndexView<'a> {
    /// `pack_index_check_locked`'s checks of the contents: the version and
    /// the size implied by the fanout table. The error is the message of
    /// C's `packfile_error`. (C checks first that there are at least `4 *
    /// 256 + 2 * oid_size` bytes, with another message: `PackIndex::open`
    /// does; here a shorter index is a wrong size.)
    pub fn parse(
        data: &'a [u8],
        oid_size: usize,
    ) -> core::result::Result<IndexView<'a>, &'static str> {
        if data.len() < 4 * 256 + 2 * oid_size {
            return Err("wrong index size");
        }
        let version = if be32(data, 0) == PACK_IDX_SIGNATURE {
            let v = be32(data, 4);
            if v != 2 {
                return Err("unsupported index version");
            }
            v
        } else {
            1
        };
        let fanout = if version > 1 { 8 } else { 0 };
        // Version 2's fanout (8 + 1024 bytes) fits: the minimum size is
        // 1024 + 2 * 20.
        if data.len() < fanout + 4 * 256 {
            return Err("wrong index size");
        }
        let mut nr = 0u32;
        for i in 0..256 {
            let n = be32(data, fanout + 4 * i);
            if n < nr {
                return Err("index is non-monotonic");
            }
            nr = n;
        }
        let idx_size = data.len() as u64;
        let (nr64, oid) = (u64::from(nr), oid_size as u64);
        if version == 1 {
            if idx_size != 4 * 256 + nr64 * (oid + 4) + oid * 2 {
                return Err("index is corrupted");
            }
        } else {
            let min_size = 8 + 4 * 256 + nr64 * (oid + 4 + 4) + oid * 2;
            let max_size = min_size + if nr != 0 { (nr64 - 1) * 8 } else { 0 };
            if idx_size < min_size || idx_size > max_size {
                return Err("wrong index size");
            }
        }
        Ok(IndexView {
            data,
            version,
            num_objects: nr,
            oid_size,
        })
    }

    /// The index version (1 or 2).
    pub fn version(&self) -> u32 {
        self.version
    }

    /// The number of objects.
    pub fn num_objects(&self) -> u32 {
        self.num_objects
    }

    /// The bytes.
    pub fn bytes(&self) -> &'a [u8] {
        self.data
    }

    /// The fanout table's start.
    #[inline]
    fn fanout_base(&self) -> usize {
        if self.version > 1 { 8 } else { 0 }
    }

    /// Where the id table starts (C's `index` in `pack_entry_find_offset`,
    /// after its adjustments), and the stride between ids.
    #[inline]
    fn ids(&self) -> (usize, usize) {
        let base = self.fanout_base() + 4 * 256;
        if self.version > 1 {
            (base, self.oid_size)
        } else {
            (base + 4, self.oid_size + 4)
        }
    }

    /// The raw id of the `n`th object.
    #[inline]
    pub fn oid_at(&self, n: u32) -> &'a [u8] {
        let (base, stride) = self.ids();
        let at = base + n as usize * stride;
        &self.data[at..at + self.oid_size]
    }

    /// `nth_packed_object_offset_locked`: the offset of the `n`th object,
    /// or -1 for a large offset outside the table.
    pub fn nth_offset(&self, n: u32) -> i64 {
        let d = self.data;
        let mut index = 4 * 256;
        if self.version == 1 {
            return i64::from(be32(d, index + (self.oid_size + 4) * n as usize));
        }
        let num = self.num_objects as usize;
        index += 8 + num * (self.oid_size + 4);
        let off32 = be32(d, index + 4 * n as usize);
        if off32 & 0x8000_0000 == 0 {
            return i64::from(off32);
        }
        // `index` and the length are well below `usize::MAX`: no overflow.
        // C computes `(off32 & 0x7fffffff) * 8` in `uint32_t`, which wraps.
        let large = index + num * 4 + (off32 & 0x7fff_ffff).wrapping_mul(8) as usize;
        // C: `index >= end - 8` (a large offset may not end within 8 bytes
        // of the end, which is checksum anyway).
        if large >= d.len() - 8 {
            return -1;
        }
        ((u64::from(be32(d, large)) << 32) | u64::from(be32(d, large + 4))) as i64
    }

    /// `pack_entry_find_offset`'s search: the object whose id starts with
    /// the first `len` hex digits of `short_id` (whose other digits are
    /// zero), unique unless `len` is the full hex size.
    pub fn lookup(&self, short_id: &[u8; GIT_OID_MAX_SIZE], len: usize) -> Lookup {
        // C compares a longer prefix with the bytes after the id (the next
        // entry's): here it is the full id (DIVERGENCES.md).
        let len = len.min(self.oid_size * 2);
        let fan = self.fanout_base();
        let id0 = short_id[0] as usize;
        let hi = be32(self.data, fan + 4 * id0);
        let lo = if id0 == 0 {
            0
        } else {
            be32(self.data, fan + 4 * (id0 - 1))
        };
        let (base, stride) = self.ids();
        let table = &self.data[base..];

        let pos = lookup_id(
            table,
            stride,
            lo,
            hi,
            &short_id[..self.oid_size],
            self.oid_size,
        );
        let num = i64::from(self.num_objects);
        let (mut found, pos) = if pos >= 0 {
            (1, i64::from(pos))
        } else {
            let pos = -1 - i64::from(pos);
            let matches = pos < num && !raw_ncmp(short_id, self.oid_at(pos as u32), len);
            (i32::from(matches), pos)
        };
        let hexsize = self.oid_size * 2;
        if found != 0
            && len != hexsize
            && pos + 1 < num
            && !raw_ncmp(short_id, self.oid_at(pos as u32 + 1), len)
        {
            found = 2;
        }
        match found {
            0 => Lookup::NotFound,
            1 => Lookup::Found(pos as u32),
            _ => Lookup::Ambiguous,
        }
    }

    /// The positions of the objects sorted by their 32-bit offset entry, as
    /// `git_pack_foreach_entry` lists them (a stable sort, as
    /// `git_vector_sort`'s).
    pub fn positions_by_offset(&self) -> Vec<u32> {
        let num = self.num_objects as usize;
        let mut pos: Vec<u32> = (0..self.num_objects).collect();
        let d = self.data;
        if self.version > 1 {
            let off = 8 + 4 * 256 + (self.oid_size + 4) * num;
            pos.sort_by_key(|&i| be32(d, off + 4 * i as usize));
        } else {
            let base = 4 * 256;
            let stride = self.oid_size + 4;
            pos.sort_by_key(|&i| be32(d, base + stride * i as usize));
        }
        pos
    }

    /// `git_pack_foreach_entry_offset`'s walk: each object's id and offset,
    /// in index order; `Err` is C's `invalid large offset` (a large offset
    /// outside the table), or what `f` returns.
    pub fn for_each_offset<E>(
        &self,
        mut f: impl FnMut(&'a [u8], i64) -> core::result::Result<(), E>,
        invalid_large_offset: impl Fn() -> E,
    ) -> core::result::Result<(), E> {
        let d = self.data;
        let num = self.num_objects as usize;
        let oid = self.oid_size;
        let index = self.fanout_base() + 4 * 256;
        if oid == 0 {
            return Ok(());
        }
        if self.version > 1 {
            let offsets = index + (oid + 4) * num;
            let large_offsets = index + (oid + 8) * num;
            let large_offsets_end = d.len() - oid;
            let ids = d[index..index + oid * num].chunks_exact(oid);
            let offs = d[offsets..offsets + 4 * num].as_chunks::<4>().0;
            for (raw, o) in ids.zip(offs) {
                let mut current = i64::from(u32::from_be_bytes(*o));
                if current & 0x8000_0000 != 0 {
                    let p = large_offsets + (current & 0x7fff_ffff) as usize * 8;
                    if p >= large_offsets_end {
                        return Err(invalid_large_offset());
                    }
                    current = ((u64::from(be32(d, p)) << 32) | u64::from(be32(d, p + 4))) as i64;
                }
                f(raw, current)?;
            }
        } else {
            for e in d[index..index + (oid + 4) * num].chunks_exact(oid + 4) {
                let (o, raw) = e.split_at(4);
                f(
                    raw,
                    i64::from(u32::from_be_bytes(o.try_into().expect("4 bytes"))),
                )?;
            }
        }
        Ok(())
    }
}

/// `git_pack__lookup_id`: binary search of `prefix` among the ids at
/// `table[i * stride..][..oid_size]` for `lo <= i < hi` (which `table`
/// holds); the position of an equal id, or `-(insertion point) - 1`.
pub fn lookup_id(
    table: &[u8],
    stride: usize,
    mut lo: u32,
    mut hi: u32,
    prefix: &[u8],
    oid_size: usize,
) -> i32 {
    let prefix = &prefix[..oid_size];
    while lo < hi {
        let mi = ((u64::from(lo) + u64::from(hi)) / 2) as u32;
        let at = mi as usize * stride;
        let cmp = table[at..at + oid_size].cmp(prefix);
        match cmp {
            core::cmp::Ordering::Equal => return mi as i32,
            core::cmp::Ordering::Greater => hi = mi,
            core::cmp::Ordering::Less => lo = mi + 1,
        }
    }
    (lo as i32).wrapping_neg().wrapping_sub(1)
}

/// A pack's index, open: the mapping and its validated layout.
pub struct PackIndex {
    map: Mapping,
    version: u32,
    num_objects: u32,
    oid_size: usize,
}

impl PackIndex {
    /// `pack_index_check_locked`: open, map and check the index at `path`.
    fn open(path: &CStr, oid_size: usize) -> Result<PackIndex> {
        let fd = futils::open_ro(path)?;
        let st = match posix::fstat(fd) {
            Ok(st) => st,
            Err(_) => {
                // SAFETY: the index descriptor this function opened, closed once.
                let _ = unsafe { posix::close(fd) };
                return Err(git_error_set!(
                    ErrorClass::OS,
                    "unable to stat pack index '{}'",
                    Bytes(path.to_bytes())
                ));
            }
        };
        if !posix::s_isreg(st.st_mode)
            || st.st_size < 0
            || (st.st_size as u64) < (4 * 256 + oid_size * 2) as u64
        {
            // SAFETY: the index descriptor this function opened, closed once.
            let _ = unsafe { posix::close(fd) };
            return Err(git_error_set!(
                ErrorClass::ODB,
                "invalid pack index '{}'",
                Bytes(path.to_bytes())
            ));
        }
        let r = futils::mmap_ro(fd, 0, st.st_size as usize);
        // SAFETY: the index descriptor this function opened, closed once.
        let _ = unsafe { posix::close(fd) };
        let map = Mapping(r?);
        let (version, num_objects) = match IndexView::parse(map.bytes(), oid_size) {
            Ok(v) => (v.version, v.num_objects),
            Err(msg) => {
                drop(map);
                return Err(packfile_error(msg));
            }
        };
        Ok(PackIndex {
            map,
            version,
            num_objects,
            oid_size,
        })
    }

    /// The index's layout over its bytes.
    #[inline]
    pub fn view(&self) -> IndexView<'_> {
        IndexView {
            data: self.map.bytes(),
            version: self.version,
            num_objects: self.num_objects,
            oid_size: self.oid_size,
        }
    }
}

// ---------------------------------------------------------------------------
// The delta base cache
// ---------------------------------------------------------------------------

/// A cached delta base: the object, its type.
struct CacheEntry {
    data: GitBytes,
    kind: c_int,
}

/// `off64_hash`.
#[derive(Default)]
struct OffHasher(u64);

impl Hasher for OffHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, _: &[u8]) {
        unreachable!("only offsets are hashed")
    }
    fn write_i64(&mut self, k: i64) {
        self.0 = u64::from(k.hash_code());
    }
}

struct CacheInner {
    memory_used: usize,
    memory_limit: usize,
    entries: HashMap<i64, Arc<CacheEntry>, BuildHasherDefault<OffHasher>>,
}

/// `git_pack_cache`: the delta bases of one pack, by offset, up to
/// [`GIT_PACK_CACHE_MEMORY_LIMIT`] bytes. An entry is in use while
/// someone other than the cache holds its `Arc` (C's `refcount`).
pub struct PackCache(Mutex<CacheInner>);

impl PackCache {
    fn new() -> PackCache {
        PackCache(Mutex::new(CacheInner {
            memory_used: 0,
            memory_limit: GIT_PACK_CACHE_MEMORY_LIMIT,
            entries: HashMap::default(),
        }))
    }

    fn lock(&self) -> MutexGuard<'_, CacheInner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `cache_get`.
    fn get(&self, offset: i64) -> Option<Arc<CacheEntry>> {
        self.lock().entries.get(&offset).cloned()
    }

    /// `cache_add`: cache `base` (of type `kind`) at `offset`, unless it is
    /// too large or already there; it is given back then.
    fn add(
        &self,
        base: GitBytes,
        kind: c_int,
        offset: i64,
    ) -> core::result::Result<Arc<CacheEntry>, GitBytes> {
        if base.len() > GIT_PACK_CACHE_SIZE_LIMIT {
            return Err(base);
        }
        let mut c = self.lock();
        if c.entries.contains_key(&offset) {
            return Err(base);
        }
        while c.memory_used + base.len() > c.memory_limit {
            // `free_lowest_entry`: drop every entry not in use. C loops
            // forever when all are in use: see DEFECTS.md.
            let before = c.entries.len();
            let mut freed = 0;
            c.entries.retain(|_, e| {
                if Arc::strong_count(e) == 1 {
                    freed += e.data.len();
                    false
                } else {
                    true
                }
            });
            c.memory_used -= freed;
            if c.entries.len() == before {
                break;
            }
        }
        let len = base.len();
        let e = Arc::new(CacheEntry { data: base, kind });
        c.entries.insert(offset, e.clone());
        c.memory_used += len;
        Ok(e)
    }
}

// ---------------------------------------------------------------------------
// The pack file
// ---------------------------------------------------------------------------

/// `git_pack_oidmap`: id to entry, filled by the indexer (C), which owns
/// the entries.
pub type PackOidMap = GitHashMap<OidPtr, *mut GitPackEntry>;

crate::git_hashmap_exports! {
    map: PackOidMap, key: *const Oid, val: *mut GitPackEntry,
    size: git_pack_oidmap_size, contains: git_pack_oidmap_contains,
    remove: git_pack_oidmap_remove, clear: git_pack_oidmap_clear,
    dispose: git_pack_oidmap_dispose, get: git_pack_oidmap_get,
    put: git_pack_oidmap_put, iterate: git_pack_oidmap_iterate,
    foreach: git_pack_oidmap_foreach,
}

/// `struct git_pack_entry`, mirrored.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GitPackEntry {
    /// `offset`: where the object starts in the pack.
    pub offset: i64,
    /// `id`.
    pub id: Oid,
    /// `p`: the pack.
    pub p: *mut GitPackFile,
}

impl Default for GitPackEntry {
    fn default() -> Self {
        GitPackEntry {
            offset: 0,
            id: Oid::NONE,
            p: ptr::null_mut(),
        }
    }
}

/// `struct git_pack_file`: the prefix C sees (`pack.h` with `GIT_RUST`),
/// then the Rust's own state. Always on the heap ([`packfile_alloc`]),
/// never moved: its `mwf` is registered by address.
#[repr(C)]
pub struct GitPackFile {
    /// `mwf`: the pack's mapped windows and descriptor.
    pub mwf: GitMwindowFile,
    /// `refcount`: the references from [`mwindow::get_pack`].
    pub refcount: Atomic32,
    /// `has_cache`: set by the indexer (C) on its own pack.
    has_cache: AtomicU32,
    /// `idx_cache`: the indexer's (C's) map of the objects it has seen.
    idx_cache: UnsafeCell<PackOidMap>,
    /// `pack_name`: points into `name`.
    pack_name: *const c_char,

    // Rust's own state.
    name: CString,
    oid_type: OidType,
    pack_keep: bool,
    mtime: i64,
    last_freshen: AtomicI64,
    /// C's `p->lock`: serializes opening the index and the file, and
    /// building `ids`.
    lock: Mutex<()>,
    index: OnceLock<PackIndex>,
    /// The index positions in pack order (`git_pack_foreach_entry`'s
    /// `p->ids`).
    ids: OnceLock<Box<[u32]>>,
    bases: PackCache,
}

// SAFETY: field by field, for shared use from several threads:
// - `mwf`: its `lock` is a mutex, `fd` and `size` are atomics, and
//   `windows` (an atomic pointer) and the windows it lists are reached
//   only under `git_mwindow__mutex` (`mwindow::Ctl`);
// - `refcount`, `has_cache`, `last_freshen`: atomics;
// - `idx_cache`: an `UnsafeCell` written only by the indexer (C) on its own
//   pack, which no other thread uses (module docs), and read only there
//   (`idx_cache_offset`, behind `has_cache`);
// - `pack_name`: points into `name`; `name`, `oid_type`, `pack_keep`,
//   `mtime`: written only by `packfile_alloc`, before the pack is shared;
// - `lock` (`Mutex`), `index` and `ids` (`OnceLock`s of `Send + Sync`
//   data), `bases` (`PackCache`, its own lock): `Sync`.
unsafe impl Sync for GitPackFile {}
// SAFETY: nothing in a pack is tied to a thread: the fields above own heap
// data, mappings and a descriptor, which any thread may free, unmap or
// close (`packfile_free`).
unsafe impl Send for GitPackFile {}

/// The C prefix of [`GitPackFile`], for the layout test.
#[repr(C)]
struct GitPackFilePrefix {
    mwf: GitMwindowFile,
    refcount: Atomic32,
    has_cache: AtomicU32,
    idx_cache: PackOidMap,
    pack_name: *const c_char,
}

const _: () = {
    use core::mem::offset_of;
    assert!(offset_of!(GitPackFile, mwf) == offset_of!(GitPackFilePrefix, mwf));
    assert!(offset_of!(GitPackFile, refcount) == offset_of!(GitPackFilePrefix, refcount));
    assert!(offset_of!(GitPackFile, has_cache) == offset_of!(GitPackFilePrefix, has_cache));
    assert!(offset_of!(GitPackFile, idx_cache) == offset_of!(GitPackFilePrefix, idx_cache));
    assert!(offset_of!(GitPackFile, pack_name) == offset_of!(GitPackFilePrefix, pack_name));
};

/// The result of reading an object's header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Header {
    used: usize,
    size: usize,
    kind: c_int,
}

/// `packfile_unpack_header1`: decode an object header (type and inflated
/// size, in a little-endian base-128 varint) at the start of `buf`.
/// `Err(BUFS)` with C's "buffer too small", or `Err(GENERIC)` for a size
/// wider than 64 bits (C's "packfile corrupted").
fn unpack_header1(buf: &[u8]) -> core::result::Result<Header, ErrorCode> {
    let Some(&first) = buf.first() else {
        let _ = git_error_set!(ErrorClass::ODB, "buffer too small");
        return Err(ErrorCode::BUFS);
    };
    let mut c = u64::from(first);
    let kind = ((c >> 4) & 7) as c_int;
    let mut size = c & 15;
    let mut shift = 4u32;
    let mut used = 1;
    while c & 0x80 != 0 {
        if buf.len() <= used {
            let _ = git_error_set!(ErrorClass::ODB, "buffer too small");
            return Err(ErrorCode::BUFS);
        }
        if u64::BITS <= shift {
            let _ = git_error_set!(ErrorClass::ODB, "packfile corrupted");
            return Err(ErrorCode::GENERIC);
        }
        c = u64::from(buf[used]);
        used += 1;
        size = size.wrapping_add((c & 0x7f) << shift);
        shift += 7;
    }
    Ok(Header {
        used,
        size: size as usize,
        kind,
    })
}

/// `git_packfile__object_header`: encode the header of an object of type
/// `kind` and `size` bytes into `hdr`; the bytes written (at most 10).
pub fn object_header(hdr: &mut [u8; 10], mut size: usize, kind: c_int) -> Result<usize> {
    git_assert_arg!(
        (GIT_OBJECT_COMMIT..=GIT_PACKFILE_REF_DELTA).contains(&kind),
        "type >= GIT_OBJECT_COMMIT && type <= GIT_PACKFILE_REF_DELTA"
    );
    let mut c = ((kind << 4) as u8) | (size & 15) as u8;
    size >>= 4;
    let mut n = 0;
    while size != 0 {
        hdr[n] = c | 0x80;
        n += 1;
        c = (size & 0x7f) as u8;
        size >>= 7;
    }
    hdr[n] = c;
    Ok(n + 1)
}

/// `get_delta_base`'s decoding of an `OFS_DELTA`'s base distance at the
/// start of `buf`: `(distance, bytes used)`, `Err(BUFS)` if `buf` ends
/// within it, or C's "overflow" message.
fn decode_ofs(buf: &[u8]) -> core::result::Result<(usize, usize), ErrorCode> {
    let Some(&first) = buf.first() else {
        return Err(ErrorCode::BUFS);
    };
    let mut c = first;
    let mut used = 1;
    let mut ubo = (c & 127) as usize;
    while c & 128 != 0 {
        if buf.len() <= used {
            return Err(ErrorCode::BUFS);
        }
        ubo = ubo.wrapping_add(1);
        // C: `!ubo || MSB(ubo, 7)`.
        if ubo == 0 || (ubo >> (usize::BITS - 7)) != 0 {
            return Err(packfile_error("overflow"));
        }
        c = buf[used];
        used += 1;
        ubo = (ubo << 7) + (c & 127) as usize;
    }
    Ok((ubo, used))
}

/// `git_delta_apply` into `git__allocator`'s memory (the result is the
/// caller's, which may hand it to C).
fn delta_apply(base: &[u8], delta: &[u8]) -> Result<GitBytes> {
    let max = crate::sys::indexer::max_object_size();
    let (res_sz, pos) = crate::delta::apply_header(base.len(), delta, max)?;
    let alloc_sz = alloc_add(res_sz, 1)?;
    if alloc_sz > isize::MAX as usize {
        return Err(errors::oom());
    }
    let p = NonNull::new(alloc::malloc(alloc_sz).cast::<u8>()).ok_or(ErrorCode::GENERIC)?;
    // SAFETY: a fresh allocation of `alloc_sz <= isize::MAX` bytes that
    // nothing else references; `MaybeUninit` needs no initialization.
    let buf = unsafe {
        core::slice::from_raw_parts_mut(p.as_ptr().cast::<core::mem::MaybeUninit<u8>>(), alloc_sz)
    };
    let (body, nul) = buf.split_at_mut(res_sz);
    if let Err(e) = crate::delta::apply_body(base, delta, pos, body) {
        // SAFETY: ours, not handed out; `buf` is not used again.
        unsafe { alloc::free(p.as_ptr().cast()) };
        return Err(e);
    }
    nul[0].write(0);
    // SAFETY: `apply_body` wrote the `res_sz` bytes, and the NUL follows.
    Ok(unsafe { GitBytes::from_raw(p, res_sz) })
}

/// A delta chain element (`struct pack_chain_elem`).
#[derive(Clone, Copy, Default)]
struct ChainElem {
    base_key: i64,
    offset: i64,
    size: usize,
    kind: c_int,
}

/// The delta chain: [`SMALL_STACK_SIZE`] elements inline, then the heap
/// (C's `small_stack` and `git_dependency_chain`).
struct Chain {
    small: [ChainElem; SMALL_STACK_SIZE],
    heap: Vec<ChainElem>,
    len: usize,
}

impl Chain {
    fn new() -> Chain {
        Chain {
            small: [ChainElem::default(); SMALL_STACK_SIZE],
            heap: Vec::new(),
            len: 0,
        }
    }

    fn push(&mut self, e: ChainElem) -> Result<&mut ChainElem> {
        if self.len < SMALL_STACK_SIZE {
            self.small[self.len] = e;
            self.len += 1;
            return Ok(&mut self.small[self.len - 1]);
        }
        if self.len == SMALL_STACK_SIZE {
            self.heap
                .try_reserve(2 * SMALL_STACK_SIZE)
                .map_err(|_| errors::oom())?;
            self.heap.extend_from_slice(&self.small);
        }
        self.heap.try_reserve(1).map_err(|_| errors::oom())?;
        self.heap.push(e);
        self.len += 1;
        Ok(self.heap.last_mut().expect("just pushed"))
    }

    fn get(&self, i: usize) -> ChainElem {
        if self.len <= SMALL_STACK_SIZE {
            self.small[i]
        } else {
            self.heap[i]
        }
    }
}

/// Brent's cycle detection over a chain of offsets: a valid pack's delta
/// chains never revisit an object, and C follows a cyclic one forever (see
/// DEFECTS.md).
struct CycleCheck {
    mark: i64,
    power: u64,
    steps: u64,
}

impl CycleCheck {
    fn new(start: i64) -> CycleCheck {
        CycleCheck {
            mark: start,
            power: 1,
            steps: 0,
        }
    }

    /// Step to `next`: `Err` (C has no message: the Rust's) if the chain
    /// came back to an offset it visited.
    fn step(&mut self, next: i64) -> Result<()> {
        if next == self.mark {
            return Err(packfile_error("delta chain loops"));
        }
        self.steps += 1;
        if self.steps == self.power {
            self.mark = next;
            self.power *= 2;
            self.steps = 0;
        }
        Ok(())
    }
}

/// An initialized inflate stream on the heap, freed on drop.
///
/// zlib-rs's `inflateInit` and `inflateEnd` cost more than inflating a
/// small object (about 0.5 µs more than zlib's); so the stream an object is
/// inflated with is kept per thread ([`with_inflater`]) and reset for the
/// next one (`inflateReset`), where C makes and frees one per object.
struct Inflater(NonNull<GitZstream>);

impl Inflater {
    fn new() -> core::result::Result<Inflater, ()> {
        let me = Inflater(NonNull::from(Box::leak(Box::new(GitZstream::new()))));
        me.z().init(ZstreamKind::Inflate as c_int).map_err(|_| ())?;
        Ok(me)
    }

    fn z(&self) -> ZstreamPtr<'_> {
        // SAFETY: the box this owns, reached only through this pointer (one
        // at a time: `with_inflater` lends it to one user), on this thread,
        // and never moved.
        unsafe { ZstreamPtr::new(self.0.as_ptr()) }
    }
}

impl Drop for Inflater {
    fn drop(&mut self) {
        self.z().free();
        // SAFETY: the box `new` leaked, freed once; zlib's state is freed.
        drop(unsafe { Box::from_raw(self.0.as_ptr()) });
    }
}

std::thread_local! {
    static INFLATER: core::cell::Cell<Option<Inflater>> = const { core::cell::Cell::new(None) };
}

/// Run `f` with a reset inflate stream: the thread's, or a new one (at the
/// first use, re-entrantly, or while the thread's locals are destroyed).
/// `init_error` is the message for a stream that cannot be made.
fn with_inflater<R>(init_error: &str, f: impl FnOnce(ZstreamPtr<'_>) -> Result<R>) -> Result<R> {
    let cached = INFLATER.try_with(core::cell::Cell::take).ok().flatten();
    let inflater = match cached {
        Some(i) => {
            i.z().reset();
            i
        }
        None => Inflater::new().map_err(|()| git_error_set!(ErrorClass::ZLIB, "{}", init_error))?,
    };
    let r = f(inflater.z());
    // Keep it for the next object (dropping it if the thread is exiting).
    let _ = INFLATER.try_with(|c| c.set(Some(inflater)));
    r
}

impl GitPackFile {
    /// The pack's file name.
    pub fn name(&self) -> &CStr {
        &self.name
    }

    /// The pack's modification time (seconds) when it was opened.
    pub fn mtime(&self) -> i64 {
        self.mtime
    }

    /// Whether a `.keep` file was beside the index.
    pub fn pack_keep(&self) -> bool {
        self.pack_keep
    }

    /// The object id type.
    pub fn oid_type(&self) -> OidType {
        self.oid_type
    }

    #[inline]
    fn oid_size(&self) -> usize {
        self.oid_type.size()
    }

    #[inline]
    fn oid_hexsize(&self) -> usize {
        self.oid_type.hexsize()
    }

    /// When `freshen` last touched the pack.
    pub fn last_freshen(&self) -> i64 {
        self.last_freshen.load(Ordering::Relaxed)
    }

    /// Record a `freshen`.
    pub fn set_last_freshen(&self, t: i64) {
        self.last_freshen.store(t, Ordering::Relaxed);
    }

    /// The key of the pack in the process's pack cache.
    pub(crate) fn pack_name_key(&self) -> CStrKey<'static> {
        // SAFETY: `pack_name` points into `name`, NUL-terminated and
        // immutable for the pack's life; the key is used for lookups and
        // kept in the pack cache only while the pack lives (it leaves the
        // cache before it is freed).
        unsafe { CStrKey::from_ptr(NonNull::from(self.name.as_c_str()).cast()) }
    }

    /// `None` unless the indexer built the pack's cache (`has_cache`);
    /// then the offset of its entry for `id`, if it has one.
    fn idx_cache_offset(&self, id: &Oid) -> Option<Option<i64>> {
        if self.has_cache.load(Ordering::Relaxed) == 0 {
            return None;
        }
        // SAFETY: `has_cache` (checked) is set only on the indexer's own
        // pack, which only the indexer's thread uses, and which this thread
        // is reading on its behalf: nothing writes the map during this
        // borrow, which ends with this fn.
        let map = unsafe { &*self.idx_cache.get() };
        // SAFETY: `OidPtr::new` needs the id to outlive the key: `id` is
        // borrowed for this fn, and the lookup does not keep the key.
        let key = unsafe { OidPtr::new(NonNull::from(id)) };
        let Some(&entry) = map.get(&key) else {
            return Some(None);
        };
        // SAFETY: the map's values are the indexer's `git_pack_entry`s,
        // which it frees only after removing them from the map (or with
        // it): live and aligned, and written by no one during this read.
        Some(Some(unsafe { (*entry).offset }))
    }

    fn plock(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The index, opened if need be (`pack_index_open_locked`, with the
    /// lock taken only to open it).
    pub(crate) fn index(&self) -> Result<&PackIndex> {
        if let Some(i) = self.index.get() {
            return Ok(i);
        }
        let g = self.plock();
        self.index_locked(&g)
    }

    /// `pack_index_open_locked`, the pack's lock held.
    fn index_locked(&self, _locked: &MutexGuard<'_, ()>) -> Result<&PackIndex> {
        if let Some(i) = self.index.get() {
            return Ok(i);
        }
        let name = self.name.to_bytes();
        git_assert!(name.len() > ".pack".len(), "name_len > strlen(\".pack\")");
        let mut idx_name = GitStr::with_capacity(name.len())?;
        idx_name.put(&name[..name.len() - ".pack".len()])?;
        idx_name.puts(b".idx")?;
        let idx_path = idx_name.as_c_str().ok_or(ErrorCode::GENERIC)?;
        let index = PackIndex::open(idx_path, self.oid_size())?;
        // The lock is held: no other thread sets it.
        let _ = self.index.set(index);
        Ok(self.index.get().expect("just set"))
    }

    /// `packfile_open_locked`: open the index and the pack file, check the
    /// pack against the index, and register the file. The pack's and the
    /// file's locks are held.
    fn open_locked(&self, plock: &MutexGuard<'_, ()>, _flock: &GitMutexGuard<'_>) -> Result<()> {
        let index = match self.index_locked(plock) {
            Ok(i) => i,
            Err(_) => {
                return Err(crate::odb::error_notfound(
                    b"failed to open packfile",
                    None,
                    0,
                ));
            }
        };
        if self.mwf.fd() >= 0 {
            return Ok(());
        }

        let ok = (|| -> Option<()> {
            let fd = match futils::open_ro(&self.name) {
                Ok(fd) => fd,
                Err(e) => {
                    self.mwf.set_fd(e.raw());
                    return None;
                }
            };
            self.mwf.set_fd(fd);
            let Ok(st) = posix::fstat(fd) else {
                let _ = git_error_set!(ErrorClass::OS, "could not stat packfile");
                return None;
            };
            // If the pack was made before its file had its size, take it.
            if self.mwf.size() == 0 {
                if !posix::s_isreg(st.st_mode) {
                    return None;
                }
                self.mwf.set_size(st.st_size);
            } else if self.mwf.size() != st.st_size {
                return None;
            }

            // C reads into an uninitialized header and checks only for a
            // failed read: a short file compares garbage (DEFECTS.md).
            let mut hdr = [0u8; 12];
            posix::read(fd, &mut hdr).ok()?;
            if be32(&hdr, 0) != PACK_SIGNATURE || be32(&hdr, 4) != PACK_VERSION {
                return None;
            }
            let view = index.view();
            if view.num_objects() != be32(&hdr, 8) {
                return None;
            }
            let oid_size = self.oid_size();
            let mut checksum = [0u8; GIT_OID_MAX_SIZE];
            // SAFETY: `checksum` has `oid_size` writable bytes.
            let r = unsafe {
                libc::pread(
                    fd,
                    checksum.as_mut_ptr().cast(),
                    oid_size,
                    self.mwf.size() - oid_size as i64,
                )
            };
            if r < 0 {
                return None;
            }
            let idx = view.bytes();
            let idx_checksum = &idx[idx.len() - 2 * oid_size..idx.len() - oid_size];
            if checksum[..oid_size] != *idx_checksum {
                return None;
            }
            // SAFETY: the pack is on the heap and never moves; it
            // deregisters its file before it is freed (`packfile_free`).
            unsafe { mwindow::file_register(&self.mwf) }.ok()?;
            Some(())
        })();

        if ok.is_some() {
            return Ok(());
        }
        let _ = git_error_set!(
            ErrorClass::OS,
            "invalid packfile '{}'",
            Bytes(self.name.to_bytes())
        );
        let fd = self.mwf.fd();
        if fd >= 0 {
            // SAFETY: the pack's descriptor, which the pack owns, closed once
            // (and set to -1 below), with its locks held.
            let _ = unsafe { posix::close(fd) };
        }
        self.mwf.set_fd(-1);
        Err(ErrorCode::GENERIC)
    }

    /// Make sure the pack file is open: C takes both locks and opens it if
    /// `fd` is -1. Here the file's lock alone is taken when it is open, and
    /// both, in C's order, only to open it. The file's lock is returned
    /// held, as callers map windows under it.
    fn ensure_open(&self) -> Result<GitMutexGuard<'_>> {
        let flock = self.mwf.lock.lock();
        if self.mwf.fd() != -1 {
            return Ok(flock);
        }
        drop(flock);
        let plock = self.plock();
        let flock = self.mwf.lock.lock();
        if self.mwf.fd() == -1 {
            self.open_locked(&plock, &flock)?;
        }
        drop(plock);
        Ok(flock)
    }

    /// `pack_window_open`: the bytes of the pack from `offset` to the end
    /// of a window holding at least the hash size after it. `None`, with no
    /// error of its own, for an offset in the trailing checksum or negative
    /// (and with the error set if the file cannot be opened).
    fn window_open<'c>(&self, cursor: &'c mut Cursor, offset: i64) -> Option<&'c [u8]> {
        let flock = self.ensure_open().ok()?;
        // An offset into the trailing checksum, or negative (wrapped), is
        // refused.
        if offset > self.mwf.size() - self.oid_size() as i64 || offset < 0 {
            return None;
        }
        let r = cursor.open(&self.mwf, offset, self.oid_size());
        drop(flock);
        r
    }

    /// `git_packfile_unpack_header`: the inflated size and type of the
    /// object at `*curpos`, which moves past the header. `Err(BUFS)` if the
    /// window cannot be opened or ends within the header.
    pub(crate) fn unpack_header(
        &self,
        cursor: &mut Cursor,
        curpos: &mut i64,
    ) -> Result<(usize, c_int)> {
        let flock = self.ensure_open()?;
        let r = match cursor.open(&self.mwf, *curpos, self.oid_size()) {
            None => {
                drop(flock);
                return Err(ErrorCode::BUFS);
            }
            Some(base) => {
                drop(flock);
                unpack_header1(base)
            }
        };
        cursor.close();
        match r {
            Ok(h) => {
                *curpos += h.used as i64;
                Ok((h.size, h.kind))
            }
            Err(ErrorCode::BUFS) => Err(ErrorCode::BUFS),
            Err(_) => Err(packfile_error("header length is zero")),
        }
    }

    /// `get_delta_base`: the offset of the base of the delta of type `kind`
    /// whose header starts at `delta_obj_offset` and whose data (the base
    /// reference first) starts at `*curpos`, which moves past the
    /// reference. `Err(BUFS)` if the window ends within the reference;
    /// `Err(PASSTHROUGH)`, with no message, for a `REF_DELTA` the indexer
    /// has not seen the base of yet. The cursor is left open.
    pub(crate) fn delta_base(
        &self,
        cursor: &mut Cursor,
        curpos: &mut i64,
        kind: c_int,
        delta_obj_offset: i64,
    ) -> Result<i64> {
        let oid_size = self.oid_size();
        let Some(base_info) = self.window_open(cursor, *curpos) else {
            // Assumption: the only reason this would fail is because the
            // file is too small.
            return Err(ErrorCode::BUFS);
        };
        let base_offset = if kind == GIT_PACKFILE_OFS_DELTA {
            let (ubo, used) = decode_ofs(base_info)?;
            if ubo == 0 || delta_obj_offset as usize <= ubo {
                return Err(packfile_error("out of bounds"));
            }
            *curpos += used as i64;
            delta_obj_offset - ubo as i64
        } else if kind == GIT_PACKFILE_REF_DELTA {
            // The window holds the hash size from `*curpos` unless it was
            // made smaller (a window size below the hash size): C reads past
            // it then; this stops (DIVERGENCES.md).
            let Some(raw) = base_info.get(..oid_size) else {
                return Err(ErrorCode::BUFS);
            };
            let base_oid = Oid::from_bytes(raw, self.oid_type).expect("oid_size bytes");
            if let Some(found) = self.idx_cache_offset(&base_oid) {
                // The indexer's pack: look in its cache, and if the base is
                // not there yet, let it come back later.
                return match found {
                    Some(0) => Err(packfile_error("delta offset is zero")),
                    Some(off) => {
                        *curpos += oid_size as i64;
                        Ok(off)
                    }
                    None => Err(ErrorCode::PASSTHROUGH),
                };
            }
            // The base entry must be in the same pack.
            let off = match self.find_offset(&base_oid, self.oid_hexsize()) {
                Ok((off, _)) => off,
                Err(_) => return Err(packfile_error("base entry delta is not in the same pack")),
            };
            *curpos += oid_size as i64;
            off
        } else {
            return Err(packfile_error("unknown object type"));
        };
        if base_offset == 0 {
            return Err(packfile_error("delta offset is zero"));
        }
        Ok(base_offset)
    }

    /// `git_packfile_resolve_header`: the size and type of the object at
    /// `offset`, following deltas to their base for the type.
    pub(crate) fn resolve_header(&self, offset: i64) -> Result<(usize, c_int)> {
        drop(self.ensure_open()?);
        let mut cursor = Cursor::new();
        let mut curpos = offset;
        let (size, mut kind) = self.unpack_header(&mut cursor, &mut curpos)?;

        let mut size_out = size;
        let mut base_offset;
        if is_delta(kind) {
            let r = self.delta_base(&mut cursor, &mut curpos, kind, offset);
            cursor.close();
            base_offset = r?;
            size_out = self.delta_header(curpos)?.1;
        } else {
            base_offset = 0;
        }

        let mut cycle = CycleCheck::new(offset);
        while is_delta(kind) {
            cycle.step(base_offset)?;
            curpos = base_offset;
            let (_, t) = self.unpack_header(&mut cursor, &mut curpos)?;
            kind = t;
            if !is_delta(kind) {
                break;
            }
            let r = self.delta_base(&mut cursor, &mut curpos, kind, base_offset);
            cursor.close();
            base_offset = r?;
        }
        Ok((size_out, kind))
    }

    /// `pack_dependency_chain`: the chain of deltas from `obj_offset` to
    /// its base (or to a base in the cache, returned with its offset).
    fn dependency_chain(
        &self,
        chain: &mut Chain,
        mut obj_offset: i64,
    ) -> Result<Option<(Arc<CacheEntry>, i64)>> {
        let mut cursor = Cursor::new();
        let mut cycle = CycleCheck::new(obj_offset);
        loop {
            // If a base is cached, we can stop here.
            if let Some(cached) = self.bases.get(obj_offset) {
                return Ok(Some((cached, obj_offset)));
            }
            let mut curpos = obj_offset;
            let elem = chain.push(ChainElem {
                base_key: obj_offset,
                ..ChainElem::default()
            })?;
            let (size, kind) = self.unpack_header(&mut cursor, &mut curpos)?;
            elem.offset = curpos;
            elem.size = size;
            elem.kind = kind;
            if !is_delta(kind) {
                return Ok(None);
            }
            let r = self.delta_base(&mut cursor, &mut curpos, kind, obj_offset);
            cursor.close();
            let base_offset = r?;
            // The position after the delta's base reference.
            chain_last(chain).offset = curpos;
            cycle.step(base_offset)?;
            obj_offset = base_offset;
        }
    }

    /// `packfile_unpack_compressed`: inflate `size` bytes from `*position`
    /// (which moves past the deflated data).
    fn unpack_compressed(
        &self,
        cursor: &mut Cursor,
        position: &mut i64,
        size: usize,
    ) -> Result<GitBytes> {
        let buffer_len = alloc_add(size, 1)?;
        let data =
            NonNull::new(alloc::malloc(buffer_len).cast::<u8>()).ok_or(ErrorCode::GENERIC)?;
        // Frees `data` unless it is handed over.
        struct Buf(NonNull<u8>);
        impl Drop for Buf {
            fn drop(&mut self) {
                // SAFETY: an allocation of `git__allocator` we own.
                unsafe { alloc::free(self.0.as_ptr().cast()) }
            }
        }
        let buf = Buf(data);

        with_inflater("failed to init zlib stream on unpack", |z| {
            // SAFETY: `data` is our allocation of `buffer_len = size + 1`
            // writable bytes, which nothing else reaches during the call.
            unsafe { self.inflate_into(z, cursor, position, data, size) }
        })?;
        core::mem::forget(buf);
        // SAFETY: `data` is ours, `size + 1` initialized bytes
        // (`inflate_into`).
        Ok(unsafe { GitBytes::from_raw(data, size) })
    }

    /// The loop of `packfile_unpack_compressed`: inflate exactly `size`
    /// bytes from `*position` into `data`, then the NUL.
    ///
    /// # Safety
    ///
    /// `data` is valid for writes of `size + 1` bytes (so `size + 1` does
    /// not overflow), which nothing else reads or writes during the call.
    unsafe fn inflate_into(
        &self,
        z: ZstreamPtr<'_>,
        cursor: &mut Cursor,
        position: &mut i64,
        data: NonNull<u8>,
        size: usize,
    ) -> Result<()> {
        let buffer_len = size + 1;
        let mut total = 0usize;
        loop {
            let mut bytes = buffer_len - total;
            let Some(input) = self.window_open(cursor, *position) else {
                return Err(ErrorCode::GENERIC);
            };
            let window_len = input.len();
            z.set_input(input.as_ptr().cast(), window_len);
            // SAFETY: the input is the window the cursor holds, mapped and
            // unwritten for the call; `data + total` has `bytes` writable
            // bytes (`total + bytes == buffer_len`, `data`'s length by this
            // fn's contract), distinct from the input and the stream.
            let r = unsafe { z.get_output_chunk(data.as_ptr().wrapping_add(total), &mut bytes) };
            cursor.close();
            r?;
            let consumed = window_len - z.in_len();
            if bytes == 0 && consumed == 0 {
                return Err(git_error_set!(
                    ErrorClass::ZLIB,
                    "error inflating zlib stream"
                ));
            }
            *position += consumed as i64;
            total += bytes;
            if z.eos() {
                break;
            }
        }

        if total != size || !z.eos() {
            return Err(git_error_set!(
                ErrorClass::ZLIB,
                "error inflating zlib stream"
            ));
        }
        // SAFETY: byte `size` is within `data`'s `size + 1` writable bytes
        // (this fn's contract); zlib wrote `total == size` bytes, so the NUL
        // makes all of them initialized.
        unsafe { data.as_ptr().wrapping_add(size).write(0) };
        Ok(())
    }

    /// `git_delta_read_header_fromstream` over a stream from `curpos`: the
    /// base and result sizes of the delta whose data starts there. Its
    /// first 16 inflated bytes (or all, if fewer) are parsed by
    /// `git_delta_read_header`.
    ///
    /// C's loop retries forever when the stream makes no progress (the
    /// window cannot be opened, or zlib needs input it does not get), and
    /// counts a failed read's -1 into its length, parsing uninitialized
    /// memory: both fail here (DEFECTS.md).
    fn delta_header(&self, mut curpos: i64) -> Result<(usize, usize)> {
        const LEN: usize = 16;
        let mut buf = [0u8; LEN];
        let mut len = 0;
        with_inflater("failed to init packfile stream", |z| {
            let mut cursor = Cursor::new();
            let mut done = false;
            let stuck = || git_error_set!(ErrorClass::ZLIB, "error reading from the zlib stream");
            while len < LEN && !done {
                // `git_packfile_stream_read`.
                let Some(input) = self.window_open(&mut cursor, curpos) else {
                    return Err(stuck());
                };
                let window_len = input.len();
                z.set_input(input.as_ptr().cast(), window_len);
                let mut n = LEN - len;
                // SAFETY: the input is the window the cursor holds, for the
                // call; `buf[len..]` has `n` writable bytes.
                let r = unsafe { z.get_output_chunk(buf.as_mut_ptr().wrapping_add(len), &mut n) };
                cursor.close();
                if r.is_err() {
                    return Err(stuck());
                }
                let consumed = window_len - z.in_len();
                curpos += consumed as i64;
                done = z.eos();
                if n == 0 {
                    // The end, or no output yet (`GIT_EBUFS`): C tries
                    // again, which only helps if input was consumed.
                    if done {
                        break;
                    }
                    if consumed == 0 {
                        return Err(stuck());
                    }
                    continue;
                }
                len += n;
            }
            Ok(())
        })?;
        crate::delta::read_header(&buf[..len])
    }

    /// `git_packfile_unpack`: the object at `*obj_offset`, its deltas
    /// applied. `*obj_offset` becomes what C leaves there: the position
    /// after the last deflated data read (or the offset of the cached base
    /// when the object itself was cached).
    pub(crate) fn unpack(&self, obj_offset: &mut i64) -> Result<(GitBytes, c_int)> {
        drop(self.ensure_open()?);

        let mut curpos = *obj_offset;
        let mut chain = Chain::new();
        let cached = self.dependency_chain(&mut chain, *obj_offset)?;
        let stack_size = chain.len + usize::from(cached.is_some());
        let mut elem_set = false;
        let mut cursor = Cursor::new();

        enum Obj {
            Cached(Arc<CacheEntry>),
            Owned(GitBytes),
        }
        impl Obj {
            fn bytes(&self) -> &[u8] {
                match self {
                    Obj::Cached(e) => e.data.as_bytes(),
                    Obj::Owned(b) => b.as_bytes(),
                }
            }
        }

        let result = (|| -> Result<(GitBytes, c_int)> {
            let mut elem_pos = stack_size - 1;
            // The base: from the cache, or the last element of the chain.
            let (base_type, mut last_elem) = match &cached {
                Some((entry, _)) => (entry.kind, None),
                None => {
                    elem_set = true;
                    let e = chain.get(elem_pos);
                    (e.kind, Some(e))
                }
            };
            match base_type {
                GIT_OBJECT_COMMIT | GIT_OBJECT_TREE | GIT_OBJECT_BLOB | GIT_OBJECT_TAG => {}
                GIT_PACKFILE_OFS_DELTA | GIT_PACKFILE_REF_DELTA => {
                    return Err(packfile_error("dependency chain ends in a delta"));
                }
                _ => return Err(packfile_error("invalid packfile type in header")),
            }
            let mut current = match cached {
                Some((entry, cached_off)) => {
                    // C's `pack_dependency_chain` stores the cached base's
                    // offset through the caller's `obj_offset`.
                    *obj_offset = cached_off;
                    // The object we want is a cached base: copy it, as the
                    // caller frees what it gets.
                    if stack_size == 1 {
                        return Ok((GitBytes::copy_from(entry.data.as_bytes())?, base_type));
                    }
                    Obj::Cached(entry)
                }
                None => {
                    let e = last_elem.expect("set above");
                    curpos = e.offset;
                    let r = self.unpack_compressed(&mut cursor, &mut curpos, e.size);
                    cursor.close();
                    Obj::Owned(r?)
                }
            };

            // Apply each delta in turn.
            while elem_pos > 0 {
                // Try to cache the base, unless it came from the cache, under
                // the offset of the object it is.
                let base = match current {
                    Obj::Cached(e) => Obj::Cached(e),
                    Obj::Owned(b) => {
                        let key = last_elem.map_or(0, |e| e.base_key);
                        match self.bases.add(b, base_type, key) {
                            Ok(e) => Obj::Cached(e),
                            Err(b) => Obj::Owned(b),
                        }
                    }
                };

                let e = chain.get(elem_pos - 1);
                elem_set = true;
                last_elem = Some(e);
                curpos = e.offset;
                let delta = self.unpack_compressed(&mut cursor, &mut curpos, e.size);
                cursor.close();
                // C leaks an uncached base when this fails (DEFECTS.md).
                let delta = delta?;

                current = Obj::Owned(delta_apply(base.bytes(), delta.as_bytes())?);
                elem_pos -= 1;
            }

            match current {
                Obj::Owned(b) => Ok((b, base_type)),
                // A chain that starts at a cached base has a delta (the
                // cached object alone returned above), so it ends owned.
                Obj::Cached(e) => Ok((GitBytes::copy_from(e.data.as_bytes())?, base_type)),
            }
        })();

        if elem_set {
            *obj_offset = curpos;
        }
        result
    }

    /// `pack_entry_find_offset`: the offset and full id of the object whose
    /// id starts with the first `len` hex digits of `short_oid`.
    pub(crate) fn find_offset(&self, short_oid: &Oid, len: usize) -> Result<(i64, Oid)> {
        let index = self.index()?;
        let view = index.view();
        match view.lookup(short_oid.raw_id(), len) {
            Lookup::NotFound => Err(crate::odb::error_notfound(
                b"failed to find offset for pack entry",
                Some(short_oid),
                len,
            )),
            Lookup::Ambiguous => Err(crate::odb::error_ambiguous(
                b"found multiple offsets for pack entry",
            )),
            Lookup::Found(pos) => {
                let offset = view.nth_offset(pos);
                if offset < 0 {
                    return Err(git_error_set!(ErrorClass::ODB, "packfile index is corrupt"));
                }
                let id = Oid::from_bytes(view.oid_at(pos), self.oid_type).expect("oid_size bytes");
                Ok((offset, id))
            }
        }
    }

    /// `git_pack_entry_find`: the entry of the object whose id starts with
    /// the first `len` hex digits of `short_oid`, once the pack file is
    /// known to be open.
    pub(crate) fn entry_find(&self, short_oid: &Oid, len: usize) -> Result<(i64, Oid)> {
        // C checks `bad_object_ids`, which nothing ever fills.
        let r = self.find_offset(short_oid, len)?;
        // Make sure the pack file backing the index still exists on disk
        // (C takes both locks to look at `fd`; an open file needs neither).
        if self.mwf.fd() == -1 {
            drop(self.ensure_open()?);
        }
        Ok(r)
    }

    /// `git_pack_foreach_entry`'s list: the ids, in pack order.
    pub(crate) fn ids(&self) -> Result<Vec<Oid>> {
        let index = self.index()?;
        let view = index.view();
        let ids = match self.ids.get() {
            Some(ids) => ids,
            None => {
                let _g = self.plock();
                if self.ids.get().is_none() {
                    let _ = self.ids.set(view.positions_by_offset().into_boxed_slice());
                }
                self.ids.get().expect("just set")
            }
        };
        // C copies the ids before calling back, without the lock.
        let mut out = Vec::new();
        out.try_reserve_exact(ids.len())
            .map_err(|_| errors::oom())?;
        out.extend(
            ids.iter()
                .map(|&i| Oid::from_bytes(view.oid_at(i), self.oid_type).expect("oid_size bytes")),
        );
        Ok(out)
    }

    /// `git_pack_foreach_entry`: call `f` with each id, in pack order, until
    /// it returns non-zero; that code (with the error set as after a
    /// callback), or the error of listing the ids.
    pub(crate) fn foreach_entry(&self, mut f: impl FnMut(&Oid) -> c_int) -> c_int {
        let ids = match self.ids() {
            Ok(ids) => ids,
            Err(e) => return e.raw(),
        };
        for id in &ids {
            let error = f(id);
            if error != 0 {
                return errors::set_after_callback(error, "git_pack_foreach_entry");
            }
        }
        0
    }

    /// `git_pack_foreach_entry_offset`: each object's id and offset, in
    /// index order, until `f` returns non-zero (which is returned, with the
    /// error set as after a callback). C holds the pack's lock meanwhile;
    /// the index is immutable once open, so this does not.
    pub(crate) fn foreach_entry_offset(
        &self,
        mut f: impl FnMut(&Oid, i64) -> c_int,
    ) -> core::result::Result<(), c_int> {
        let index = self.index().map_err(ErrorCode::raw)?;
        // One id, of which each entry overwrites the hash bytes (the rest
        // stay zero), as C's `current_oid`.
        let mut id = Oid::zero(self.oid_type);
        let n = self.oid_size();
        index.view().for_each_offset(
            |raw, off| {
                id.raw_id_mut()[..n].copy_from_slice(raw);
                match f(&id, off) {
                    0 => Ok(()),
                    e => Err(errors::set_after_callback(
                        e,
                        "git_pack_foreach_entry_offset",
                    )),
                }
            },
            || packfile_error("invalid large offset").raw(),
        )
    }
}

/// The last element of a chain (the one just pushed).
fn chain_last(chain: &mut Chain) -> &mut ChainElem {
    let i = chain.len - 1;
    if chain.len <= SMALL_STACK_SIZE {
        &mut chain.small[i]
    } else {
        &mut chain.heap[i]
    }
}

/// `git_packfile__name`: the `.pack` name of the index `path`.
pub fn packfile_name(path: &[u8]) -> Result<crate::util::alloc::GitCStr> {
    if path.len() < ".idx".len() {
        return Err(crate::odb::error_notfound(
            b"invalid packfile path",
            None,
            0,
        ));
    }
    let mut buf = GitStr::new();
    buf.put(&path[..path.len() - ".idx".len()])?;
    buf.puts(b".pack")?;
    buf.detach().ok_or_else(errors::oom)
}

/// `git_packfile_alloc`: a pack for the index (or pack) `path`, whose
/// pack file must exist; nothing is opened yet.
pub fn packfile_alloc(path: &[u8], oid_type: git_oid_t) -> Result<*mut GitPackFile> {
    if path.len() < ".idx".len() {
        return Err(crate::odb::error_notfound(
            b"invalid packfile path",
            None,
            0,
        ));
    }
    let oid_type = if oid_type == 0 {
        OidType::DEFAULT.raw()
    } else {
        oid_type
    };
    let Some(oid_type) = OidType::from_raw(oid_type) else {
        // C goes on with a hash size of 0 (DIVERGENCES.md).
        return Err(git_error_set!(ErrorClass::INVALID, "invalid oid type"));
    };
    let path = match memchr::memchr(0, path) {
        Some(n) => &path[..n],
        None => path,
    };

    // Make sure a corresponding .pack file exists and that the index looks
    // sane.
    let mut name = path.to_vec();
    let mut pack_keep = false;
    if path.ends_with(b".idx") {
        let root = path.len() - ".idx".len();
        if !git_disable_pack_keep_file_checks.load(Ordering::Relaxed) {
            name.truncate(root);
            name.extend_from_slice(b".keep");
            let keep = CString::new(name.clone()).expect("no NUL");
            if crate::util::fs_path::exists(&keep) {
                pack_keep = true;
            }
        }
        name.truncate(root);
        name.extend_from_slice(b".pack");
    }
    let name = CString::new(name).expect("no NUL");

    let st = match posix::stat(&name) {
        Ok(st) if posix::s_isreg(st.st_mode) => st,
        _ => return Err(crate::odb::error_notfound(b"packfile not found", None, 0)),
    };

    let pack_name = name.as_ptr();
    let p = Box::new(GitPackFile {
        mwf: GitMwindowFile::new(),
        refcount: Atomic32::new(0),
        has_cache: AtomicU32::new(0),
        idx_cache: UnsafeCell::new(GitHashMap::new()),
        pack_name,
        name,
        oid_type,
        pack_keep,
        mtime: st.st_mtime,
        last_freshen: AtomicI64::new(0),
        lock: Mutex::new(()),
        index: OnceLock::new(),
        ids: OnceLock::new(),
        bases: PackCache::new(),
    });
    p.mwf.set_size(st.st_size);
    Ok(Box::into_raw(p))
}

/// `git_packfile_free`: close and free a pack, and unlink its file if
/// asked.
///
/// # Safety
///
/// `p` is NULL or a pack from [`packfile_alloc`] that nothing else uses
/// any more (in particular, not in the pack cache), freed once.
pub unsafe fn packfile_free(p: *mut GitPackFile, unlink_packfile: bool) {
    if p.is_null() {
        return;
    }
    // SAFETY: a non-NULL `p` is the `Box::into_raw` of `packfile_alloc`,
    // which nothing else uses any more and is freed once (this fn's
    // contract).
    let pack = unsafe { Box::from_raw(p) };
    {
        let _g = pack.plock();
        // C frees the windows only when the descriptor is open; a file
        // registered without one is deregistered too, so that no dangling
        // pointer stays in the global list.
        let _ = mwindow::free_all(&pack.mwf);
        let fd = pack.mwf.fd();
        if fd >= 0 {
            // SAFETY: the pack's own descriptor, closed once (and set to -1),
            // with the pack's lock held and the pack ours alone.
            let _ = unsafe { posix::close(fd) };
            pack.mwf.set_fd(-1);
        }
    }
    if unlink_packfile {
        let _ = posix::unlink(&pack.name);
    }
    // The index is unmapped and the cache freed with the box. Windows still
    // in use (which `free_all` refused) stay mapped, as in C.
    drop(pack);
}

// ---------------------------------------------------------------------------
// Streams
// ---------------------------------------------------------------------------

/// `git_packfile_stream`, mirrored: an object's deflated data, inflated as
/// it is read.
#[repr(C)]
pub struct GitPackfileStream {
    /// `curpos`: where the next input starts.
    pub curpos: i64,
    /// `done`.
    pub done: c_int,
    /// `zstream`.
    pub zstream: GitZstream,
    /// `p`.
    pub p: *mut GitPackFile,
    /// `mw`: the cursor.
    pub mw: *mut GitMwindow,
}

impl GitPackfileStream {
    /// `git_packfile_stream_open`: initialize the stream at `obj` in place,
    /// to inflate from `curpos` of `p`.
    ///
    /// # Safety
    ///
    /// `obj` is writable for a stream; until [`dispose`](Self::dispose) it
    /// stays at its address and is used only through these functions (and
    /// C's reads of `curpos` and `done`), by one thread at a time; `p`
    /// outlives it.
    pub unsafe fn open_in(obj: *mut GitPackfileStream, p: &GitPackFile, curpos: i64) -> Result<()> {
        // SAFETY: `obj` is valid for writes of a stream and aligned (this
        // fn's contract); `write` does not read or drop what was there
        // (C's `memset` then the fields).
        unsafe {
            obj.write(GitPackfileStream {
                curpos,
                done: 0,
                zstream: GitZstream::new(),
                p: ptr::from_ref(p).cast_mut(),
                mw: ptr::null_mut(),
            })
        };
        // SAFETY: the stream's zlib stream, reached only through stream
        // functions (the caller's contract).
        let z = unsafe { Self::z(obj) };
        if z.init(ZstreamKind::Inflate as c_int).is_err() {
            return Err(git_error_set!(
                ErrorClass::ZLIB,
                "failed to init packfile stream"
            ));
        }
        Ok(())
    }

    /// The zlib stream of a stream.
    ///
    /// # Safety
    ///
    /// `obj` is a stream as [`open_in`](Self::open_in) states.
    unsafe fn z<'a>(obj: *mut GitPackfileStream) -> ZstreamPtr<'a> {
        // SAFETY: a field's place, no reference (`obj` is valid).
        let zs = unsafe { &raw mut (*obj).zstream };
        // SAFETY: the zlib stream is reached only through such pointers, by
        // one thread at a time, and stays put (the caller's contract).
        unsafe { ZstreamPtr::new(zs) }
    }

    /// `git_packfile_stream_read`: inflate into `buffer` (`len` writable
    /// bytes); the bytes written, 0 at the end, `GIT_EBUFS` if more input is
    /// needed (or the window cannot be opened), -1 on a zlib error.
    ///
    /// # Safety
    ///
    /// `obj` is an open stream (as [`open_in`](Self::open_in) states);
    /// `buffer` is writable for `len` bytes and overlaps neither the stream
    /// nor the pack.
    pub unsafe fn read(obj: *mut GitPackfileStream, buffer: *mut u8, len: usize) -> isize {
        // SAFETY: `obj` is an open stream, valid and aligned, that only
        // this call uses (this fn's contract); its fields are read and
        // written as places, with no reference to the whole stream, so the
        // `&mut` to `mw` and the zlib stream's pointer below overlap none
        // of these accesses.
        if unsafe { (*obj).done } != 0 {
            return 0;
        }
        // SAFETY: a field read in place, as for `done` just above.
        let p = unsafe { (*obj).p };
        // SAFETY: `p` is the live pack `open_in` stored, which outlives the
        // stream (`open_in`'s contract); `GitPackFile` is only read through
        // `&` (interior mutability where it changes).
        let p = unsafe { &*p };
        // SAFETY: a field's place, as for `done` above; no reference made.
        let mw = unsafe { &raw mut (*obj).mw };
        // SAFETY: `mw` is valid, aligned and NULL or a window this stream's
        // cursor holds (only `window_open` and `close` below store into
        // it); nothing else reaches it while `cursor` lives, which ends
        // with this call.
        let cursor = unsafe { Cursor::from_raw_mut(mw) };
        // SAFETY: a field read in place, as for `done` above.
        let curpos = unsafe { (*obj).curpos };
        let Some(input) = p.window_open(cursor, curpos) else {
            return ErrorCode::BUFS.raw() as isize;
        };
        // SAFETY: `obj` is an open stream used only by this call (this
        // fn's contract), as `z` requires.
        let z = unsafe { Self::z(obj) };
        let window_len = input.len();
        z.set_input(input.as_ptr().cast(), window_len);
        let mut len = len;
        // SAFETY: the input is the window the cursor holds, mapped for the
        // call; `buffer` is writable for `len` bytes and distinct from both.
        let r = unsafe { z.get_output_chunk(buffer, &mut len) };
        cursor.close();
        if r.is_err() {
            let _ = git_error_set!(ErrorClass::ZLIB, "error reading from the zlib stream");
            return -1;
        }
        let eos = z.eos();
        // SAFETY: a field written in place, as `done` is read at the top;
        // `cursor` (the `&mut` to `mw`) is not used again.
        unsafe {
            (*obj).curpos = curpos + (window_len - z.in_len()) as i64;
        }
        if eos {
            // SAFETY: as the write just above, for `done`.
            unsafe { (*obj).done = 1 };
        }
        // If we wrote nothing but are not done, we need more data.
        if len == 0 && !eos {
            return ErrorCode::BUFS.raw() as isize;
        }
        len as isize
    }

    /// `git_packfile_stream_dispose`.
    ///
    /// # Safety
    ///
    /// `obj` is an open stream, disposed once.
    pub unsafe fn dispose(obj: *mut GitPackfileStream) {
        // SAFETY: an open stream.
        unsafe { Self::z(obj) }.free();
    }
}

// ---------------------------------------------------------------------------
// The safe API
// ---------------------------------------------------------------------------

/// The type of an object in a pack (`git_object_t`, with the pack's two
/// delta types).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PackObjectType {
    /// A commit.
    Commit,
    /// A tree.
    Tree,
    /// A blob.
    Blob,
    /// An annotated tag.
    Tag,
    /// A delta against an object at an earlier offset.
    OfsDelta,
    /// A delta against an object named by its id.
    RefDelta,
}

impl PackObjectType {
    /// The `git_object_t` value.
    pub fn raw(self) -> c_int {
        match self {
            PackObjectType::Commit => GIT_OBJECT_COMMIT,
            PackObjectType::Tree => GIT_OBJECT_TREE,
            PackObjectType::Blob => GIT_OBJECT_BLOB,
            PackObjectType::Tag => GIT_OBJECT_TAG,
            PackObjectType::OfsDelta => GIT_PACKFILE_OFS_DELTA,
            PackObjectType::RefDelta => GIT_PACKFILE_REF_DELTA,
        }
    }

    /// The type of a `git_object_t` value.
    pub fn from_raw(t: c_int) -> Option<PackObjectType> {
        Some(match t {
            GIT_OBJECT_COMMIT => PackObjectType::Commit,
            GIT_OBJECT_TREE => PackObjectType::Tree,
            GIT_OBJECT_BLOB => PackObjectType::Blob,
            GIT_OBJECT_TAG => PackObjectType::Tag,
            GIT_PACKFILE_OFS_DELTA => PackObjectType::OfsDelta,
            GIT_PACKFILE_REF_DELTA => PackObjectType::RefDelta,
            _ => return None,
        })
    }
}

/// An object read from a pack: its type and its data.
#[derive(Debug)]
pub struct PackObject {
    kind: PackObjectType,
    data: GitBytes,
}

impl PackObject {
    pub(crate) fn new(kind: PackObjectType, data: GitBytes) -> PackObject {
        PackObject { kind, data }
    }

    /// The type (never a delta type).
    pub fn kind(&self) -> PackObjectType {
        self.kind
    }

    /// The data.
    pub fn data(&self) -> &[u8] {
        self.data.as_bytes()
    }

    /// The data, owned.
    pub fn into_data(self) -> GitBytes {
        self.data
    }
}

/// A pack, shared through the process's pack cache (`git_mwindow_get_pack`),
/// released on drop (`git_mwindow_put_pack`).
pub struct Pack(NonNull<GitPackFile>);

// SAFETY: the one field is a counted reference to a heap `GitPackFile`,
// which is `Send + Sync`; the count is atomic and the reference is
// released by `put_pack`, under the global mutex, from any thread.
unsafe impl Send for Pack {}
// SAFETY: `&Pack` reaches only `&GitPackFile` (`file`), which is `Sync`,
// and `Clone` takes the global mutex.
unsafe impl Sync for Pack {}

impl Pack {
    /// Open the pack of the index `idx_path` (a `.idx` file; its `.pack`
    /// must exist), or share it if this process has it open.
    ///
    /// The pack and its index are read through memory maps, borrowed as
    /// byte slices: this relies on git's rule that pack and index files are
    /// never modified in place (they are written under a new name and
    /// renamed). A file truncated under the map makes the process fault
    /// (`SIGBUS`), as with the C.
    pub fn open(idx_path: &[u8], oid_type: OidType) -> core::result::Result<Pack, errors::Error> {
        let path = CString::new(idx_path).map_err(|_| {
            errors::Error::from(git_error_set!(ErrorClass::INVALID, "path contains a NUL"))
        })?;
        let p = mwindow::get_pack(&path, oid_type.raw())?;
        Ok(Pack(NonNull::new(p).expect("get_pack gives a pack")))
    }

    /// Adopt a reference from `git_mwindow_get_pack`.
    ///
    /// # Safety
    ///
    /// `p` is a pack from [`mwindow::get_pack`] whose reference the caller
    /// gives up.
    pub(crate) unsafe fn from_raw(p: NonNull<GitPackFile>) -> Pack {
        Pack(p)
    }

    /// The pack.
    pub(crate) fn file(&self) -> &GitPackFile {
        // SAFETY: the reference keeps it alive; it is `Sync`.
        unsafe { self.0.as_ref() }
    }

    /// The pointer C knows the pack by.
    #[allow(dead_code)] // for l-odb's `Odb`, at integration
    pub(crate) fn as_ptr(&self) -> *mut GitPackFile {
        self.0.as_ptr()
    }

    /// The `.pack` file name.
    pub fn name(&self) -> &[u8] {
        self.file().name().to_bytes()
    }

    /// The number of objects.
    pub fn num_objects(&self) -> core::result::Result<u32, errors::Error> {
        Ok(self.file().index()?.view().num_objects())
    }

    /// The offset and full id of the object whose id starts with the first
    /// `len` hex digits of `id` (all of them: `id.hexsize()`).
    pub fn find(&self, id: &Oid, len: usize) -> core::result::Result<(u64, Oid), errors::Error> {
        let len = len.min(self.file().oid_hexsize());
        let (off, id) = self.file().entry_find(id, len)?;
        Ok((off as u64, id))
    }

    /// The object at `offset`, its deltas applied.
    pub fn read(&self, offset: u64) -> core::result::Result<PackObject, errors::Error> {
        let mut off = offset as i64;
        let (data, kind) = self.file().unpack(&mut off)?;
        let kind = PackObjectType::from_raw(kind).ok_or_else(|| {
            errors::Error::from(packfile_error("invalid packfile type in header"))
        })?;
        Ok(PackObject { kind, data })
    }

    /// The object with id `id`.
    pub fn read_id(&self, id: &Oid) -> core::result::Result<PackObject, errors::Error> {
        let (off, _) = self.find(id, id.hexsize())?;
        self.read(off)
    }

    /// The size and type of the object at `offset`.
    pub fn read_header(
        &self,
        offset: u64,
    ) -> core::result::Result<(usize, PackObjectType), errors::Error> {
        let (size, kind) = self.file().resolve_header(offset as i64)?;
        let kind = PackObjectType::from_raw(kind).ok_or_else(|| {
            errors::Error::from(packfile_error("invalid packfile type in header"))
        })?;
        Ok((size, kind))
    }

    /// The ids of the objects, in pack order.
    pub fn ids(&self) -> core::result::Result<Vec<Oid>, errors::Error> {
        Ok(self.file().ids()?)
    }

    /// Each object's id and offset, in index order.
    pub fn entries(&self) -> core::result::Result<Vec<(Oid, u64)>, errors::Error> {
        let mut out = Vec::new();
        self.file()
            .foreach_entry_offset(|id, off| {
                out.push((*id, off as u64));
                0
            })
            .map_err(|e| {
                errors::Error::from(ErrorCode::from_raw(e).unwrap_or(ErrorCode::GENERIC))
            })?;
        Ok(out)
    }
}

impl Clone for Pack {
    fn clone(&self) -> Pack {
        // Another reference: the cache's count. Taken under the global
        // mutex, as `get_pack` takes it.
        let _ctl = mwindow::Ctl::lock();
        self.file().refcount.inc();
        Pack(self.0)
    }
}

impl Drop for Pack {
    fn drop(&mut self) {
        // SAFETY: our reference, given up once.
        let _ = unsafe { mwindow::put_pack(self.0.as_ptr()) };
    }
}

impl core::fmt::Debug for Pack {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Pack").field(&self.file().name()).finish()
    }
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------

/// The exports of `pack.c`.
pub mod ffi {
    use super::*;
    use crate::git_arg;
    use crate::util::cabi::{CStrArg, Out};
    use crate::util::export::{barrier, status};

    /// `git_pack__lookup_id`.
    ///
    /// # Safety
    ///
    /// `id_lookup_table` holds readable ids at `i * stride` for every
    /// `0 <= i < hi` (the slice made covers them all; midx.c and
    /// commit_graph.c pass the whole table);
    /// `id_prefix` is readable for the hash size of `oid_type`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pack__lookup_id(
        id_lookup_table: *const c_void,
        stride: usize,
        lo: c_uint,
        hi: c_uint,
        id_prefix: *const u8,
        oid_type: git_oid_t,
    ) -> c_int {
        barrier(|| {
            let oid_size = oid::oid_size(oid_type);
            let len = if hi == 0 || lo >= hi {
                0
            } else {
                (hi as usize - 1) * stride + oid_size
            };
            let table = if len == 0 {
                &[][..]
            } else {
                // SAFETY: the entries below `hi` are readable (this export's
                // contract), the last ending at `len`, which is therefore
                // within one allocation (`<= isize::MAX`); `u8` alignment 1.
                unsafe { core::slice::from_raw_parts(id_lookup_table.cast::<u8>(), len) }
            };
            let prefix = if oid_size == 0 {
                &[][..]
            } else {
                // SAFETY: `id_prefix` is readable for `oid_size` bytes (this
                // export's contract).
                unsafe { core::slice::from_raw_parts(id_prefix, oid_size) }
            };
            lookup_id(table, stride, lo, hi, prefix, oid_size)
        })
    }

    /// `git_packfile__object_header`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `hdr` has room for the header (10 bytes for any
    /// size).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_packfile__object_header(
        mut out: Out<'_, usize>,
        hdr: *mut u8,
        size: usize,
        type_: c_int,
    ) -> c_int {
        barrier(|| {
            let mut buf = [0u8; 10];
            match object_header(&mut buf, size, type_) {
                Ok(n) => {
                    // SAFETY: `hdr` has room for the header, `n <= 10` bytes
                    // (this export's contract), and is not `buf`, a local.
                    unsafe { ptr::copy_nonoverlapping(buf.as_ptr(), hdr, n) };
                    out.write(n);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_packfile__name`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `path` is NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_packfile__name(
        mut out: Out<'_, *mut c_char>,
        path: CStrArg<'_>,
    ) -> c_int {
        barrier(|| {
            // (C's `strlen` dereferences a NULL `path`.)
            let path = git_arg!(path.get(), "path", -1);
            match packfile_name(path.to_bytes()) {
                Ok(s) => {
                    out.write(s.into_raw());
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_packfile_alloc`.
    ///
    /// # Safety
    ///
    /// `pack_out` is writable; `path` is NULL or NUL-terminated.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_packfile_alloc(
        mut pack_out: Out<'_, *mut GitPackFile>,
        path: CStrArg<'_>,
        oid_type: git_oid_t,
    ) -> c_int {
        barrier(|| {
            pack_out.write(ptr::null_mut());
            let path = path.to_bytes();
            match packfile_alloc(path, oid_type) {
                Ok(p) => {
                    pack_out.write(p);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_packfile_free`.
    ///
    /// # Safety
    ///
    /// As [`packfile_free`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_packfile_free(p: *mut GitPackFile, unlink_packfile: bool) {
        // SAFETY: `p` is NULL or a pack from `packfile_alloc` that nothing
        // uses any more (not in the pack cache), freed once: this export's
        // contract, which is `packfile_free`'s.
        barrier(|| unsafe { packfile_free(p, unlink_packfile) })
    }

    /// `git_packfile_unpack_header`.
    ///
    /// # Safety
    ///
    /// `size_p` and `type_p` are NULL or writable; `p` is NULL or a live
    /// pack; `w_curs` NULL or a valid cursor (as
    /// [`Cursor::from_raw_mut`] states, for the call); `curpos` NULL or
    /// valid, used by nothing else during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_packfile_unpack_header(
        mut size_p: Out<'_, usize>,
        mut type_p: Out<'_, c_int>,
        p: Option<&GitPackFile>,
        w_curs: Option<&mut Cursor>,
        curpos: Option<&mut i64>,
    ) -> c_int {
        barrier(|| {
            let p = git_arg!(p, "p", -1);
            // (C dereferences a NULL cursor or position.)
            let cursor = git_arg!(w_curs, "w_curs", -1);
            let curpos = git_arg!(curpos, "curpos", -1);
            match p.unpack_header(cursor, curpos) {
                Ok((size, kind)) => {
                    size_p.write(size);
                    type_p.write(kind);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_packfile_resolve_header`.
    ///
    /// # Safety
    ///
    /// `size_p` and `type_p` are writable; `p` is a live pack.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_packfile_resolve_header(
        mut size_p: Out<'_, usize>,
        mut type_p: Out<'_, c_int>,
        p: Option<&GitPackFile>,
        offset: i64,
    ) -> c_int {
        barrier(|| {
            let p = git_arg!(p, "p", -1);
            match p.resolve_header(offset) {
                Ok((size, kind)) => {
                    size_p.write(size);
                    type_p.write(kind);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `get_delta_base`.
    ///
    /// # Safety
    ///
    /// `delta_base_out` is NULL or writable; `p` is NULL or a live pack;
    /// `w_curs` and `curpos` as in [`git_packfile_unpack_header`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn get_delta_base(
        mut delta_base_out: Out<'_, i64>,
        p: Option<&GitPackFile>,
        w_curs: Option<&mut Cursor>,
        curpos: Option<&mut i64>,
        type_: c_int,
        delta_obj_offset: i64,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!delta_base_out.is_null(), "delta_base_out", -1);
            let p = git_arg!(p, "p", -1);
            // (C dereferences a NULL cursor or position.)
            let cursor = git_arg!(w_curs, "w_curs", -1);
            let curpos = git_arg!(curpos, "curpos", -1);
            match p.delta_base(cursor, curpos, type_, delta_obj_offset) {
                Ok(off) => {
                    delta_base_out.write(off);
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_packfile_unpack`.
    ///
    /// # Safety
    ///
    /// `obj` is NULL or writable; `p` is NULL or a live pack; `obj_offset`
    /// NULL or valid, used by nothing else during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_packfile_unpack(
        mut obj: Out<'_, GitRawobj>,
        p: Option<&GitPackFile>,
        obj_offset: Option<&mut i64>,
    ) -> c_int {
        barrier(|| {
            let p = git_arg!(p, "p", -1);
            // (C dereferences a NULL offset.)
            let off = git_arg!(obj_offset, "obj_offset", -1);
            match p.unpack(off) {
                Ok((d, kind)) => {
                    let (data, len) = d.into_raw();
                    // C frees `data`.
                    obj.write(GitRawobj {
                        data: data.cast(),
                        len,
                        type_: kind,
                    });
                    0
                }
                Err(e) => {
                    // C leaves `obj` empty (or, if the chain could not be
                    // built, untouched); callers read it only on success.
                    obj.write(GitRawobj {
                        data: ptr::null_mut(),
                        len: 0,
                        type_: GIT_OBJECT_INVALID,
                    });
                    e.raw()
                }
            }
        })
    }

    /// `git_packfile_stream_open`.
    ///
    /// # Safety
    ///
    /// `obj` is writable and stays at its address until
    /// `git_packfile_stream_dispose`; `p` is a live pack that outlives it.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_packfile_stream_open(
        obj: *mut GitPackfileStream,
        p: Option<&GitPackFile>,
        curpos: i64,
    ) -> c_int {
        barrier(|| {
            let p = git_arg!(p, "p", -1);
            // SAFETY: `obj` is writable and stays put until disposed, `p`
            // outlives it (this export's contract, `open_in`'s).
            status(unsafe { GitPackfileStream::open_in(obj, p, curpos) }.map(|_| ()))
        })
    }

    /// `git_packfile_stream_read`.
    ///
    /// # Safety
    ///
    /// `obj` is an open stream that nothing else uses during the call;
    /// `buffer` is writable for `len` bytes and overlaps neither the stream
    /// nor the pack.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_packfile_stream_read(
        obj: *mut GitPackfileStream,
        buffer: *mut c_void,
        len: usize,
    ) -> isize {
        barrier(|| {
            // SAFETY: an open stream and a writable buffer (this export's
            // contract), as `read` requires.
            unsafe { GitPackfileStream::read(obj, buffer.cast(), len) }
        })
    }

    /// `git_packfile_stream_dispose`.
    ///
    /// # Safety
    ///
    /// `obj` is an open stream, disposed once.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_packfile_stream_dispose(obj: *mut GitPackfileStream) {
        // SAFETY: an open stream, disposed once (this export's contract),
        // as `dispose` requires.
        barrier(|| unsafe { GitPackfileStream::dispose(obj) })
    }

    /// `git_pack_entry_find`.
    ///
    /// # Safety
    ///
    /// `e` is NULL or writable; `p` is NULL or a live pack; `short_oid`
    /// NULL or a readable id, which may lie in `*e` (C writes `e` last).
    ///
    /// (`p` stays a raw pointer: the entry hands it back to C. `short_oid`
    /// too, read once before `e` is written: as a reference it would be
    /// protected for the call, and the write to `e` would then be
    /// undefined behaviour when it covers the id.)
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pack_entry_find(
        mut e: Out<'_, GitPackEntry>,
        p: *mut GitPackFile,
        short_oid: *const Oid,
        len: usize,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!short_oid.is_null(), "short_oid", -1);
            // SAFETY: `short_oid` is non-null, so a readable, aligned id
            // (this export's contract), copied before `e` is written; no
            // reference to it is kept.
            let short_oid = &unsafe { short_oid.read() };
            git_assert_arg!(!p.is_null(), "p", -1);
            // SAFETY: a live pack (this export's contract; not NULL, checked
            // above), only read through `&` (`GitPackFile` is `Sync` with
            // interior mutability).
            let pack = unsafe { &*p };
            match pack.entry_find(short_oid, len) {
                Ok((offset, found)) => {
                    e.write(GitPackEntry {
                        offset,
                        id: found,
                        p,
                    });
                    0
                }
                Err(err) => err.raw(),
            }
        })
    }

    /// `git_pack_foreach_entry`.
    ///
    /// # Safety
    ///
    /// `p` is a live pack; `cb` may be called with `data` (a
    /// `git_odb_foreach_cb` contract).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pack_foreach_entry(
        p: Option<&GitPackFile>,
        cb: crate::sys::midx::OdbForeachCb,
        data: *mut c_void,
    ) -> c_int {
        barrier(|| {
            let p = git_arg!(p, "p", -1);
            // SAFETY: `cb` may be called with `data` and an id (this
            // export's contract); each id is readable for its call.
            p.foreach_entry(|id| unsafe { cb(id, data) })
        })
    }

    /// `git_pack_foreach_entry_offset_cb`.
    pub type ForeachEntryOffsetCb =
        unsafe extern "C" fn(id: *const Oid, offset: i64, payload: *mut c_void) -> c_int;

    /// `git_pack_foreach_entry_offset`.
    ///
    /// # Safety
    ///
    /// `p` is a live pack; `cb` may be called with `data`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_pack_foreach_entry_offset(
        p: Option<&GitPackFile>,
        cb: ForeachEntryOffsetCb,
        data: *mut c_void,
    ) -> c_int {
        barrier(|| {
            let p = git_arg!(p, "p", -1);
            let r = p.foreach_entry_offset(|id, off| {
                // SAFETY: `cb` may be called with `data` (this export's
                // contract), an id readable for the call, and an offset.
                unsafe { cb(id, off, data) }
            });
            match r {
                Ok(()) => 0,
                Err(e) => e,
            }
        })
    }
}

crate::c_layout_test! {
    headers: ["pack.h"],
    types: [
        GitPackFilePrefix => "struct git_pack_file" { mwf, refcount, has_cache, idx_cache, pack_name },
        GitPackEntry => "struct git_pack_entry" { offset, id, p },
        GitPackfileStream => "git_packfile_stream" { curpos, done, zstream, p, mw },
        GitRawobj => "git_rawobj" { data, len, type_ as "type" },
    ],
    asserts: [
        "GIT_PACKFILE_OFS_DELTA == 6", "GIT_PACKFILE_REF_DELTA == 7",
        "PACK_SIGNATURE == 0x5041434b", "PACK_IDX_SIGNATURE == 0xff744f63",
        "GIT_OBJECT_COMMIT == 1 && GIT_OBJECT_TAG == 4 && GIT_OBJECT_INVALID == -1",
        "sizeof(git_object_t) == 4",
        "sizeof(git_pack_oidmap) == 40 && _Alignof(git_pack_oidmap) == 8",
    ],
}

const _: () = assert!(size_of::<PackOidMap>() == 40 && align_of::<PackOidMap>() == 8);

#[cfg(test)]
mod tests {
    use super::*;

    /// An index (v1 or v2) over sorted `ids` with `offsets`, as git writes
    /// it: v2 puts offsets of 2^31 and up in the 64-bit table.
    pub(crate) fn build_index(version: u32, ids: &[Vec<u8>], offsets: &[u64]) -> Vec<u8> {
        let oid_size = ids.first().map_or(20, Vec::len);
        let mut fan = [0u32; 256];
        for id in ids {
            for f in &mut fan[id[0] as usize..] {
                *f += 1;
            }
        }
        let mut out = Vec::new();
        if version == 2 {
            out.extend_from_slice(&PACK_IDX_SIGNATURE.to_be_bytes());
            out.extend_from_slice(&2u32.to_be_bytes());
        }
        for f in fan {
            out.extend_from_slice(&f.to_be_bytes());
        }
        if version == 1 {
            for (id, &off) in ids.iter().zip(offsets) {
                out.extend_from_slice(&(off as u32).to_be_bytes());
                out.extend_from_slice(id);
            }
        } else {
            for id in ids {
                out.extend_from_slice(id);
            }
            for i in 0..ids.len() {
                out.extend_from_slice(&(i as u32 * 7).to_be_bytes()); // crc
            }
            let mut large = Vec::new();
            for &off in offsets {
                if off >= 0x8000_0000 {
                    out.extend_from_slice(&(0x8000_0000 | (large.len() / 8) as u32).to_be_bytes());
                    large.extend_from_slice(&off.to_be_bytes());
                } else {
                    out.extend_from_slice(&(off as u32).to_be_bytes());
                }
            }
            out.extend_from_slice(&large);
        }
        out.extend(core::iter::repeat_n(0xab, 2 * oid_size));
        out
    }

    fn ids(n: usize, size: usize) -> Vec<Vec<u8>> {
        let mut v: Vec<Vec<u8>> = (0..n)
            .map(|i| {
                let mut id = vec![0u8; size];
                let x = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
                id[..8].copy_from_slice(&x.to_be_bytes());
                id[size - 1] = i as u8;
                id
            })
            .collect();
        v.sort();
        v
    }

    fn key(id: &[u8]) -> [u8; GIT_OID_MAX_SIZE] {
        let mut k = [0u8; GIT_OID_MAX_SIZE];
        k[..id.len()].copy_from_slice(id);
        k
    }

    #[test]
    fn parse_checks_version_fanout_and_size() {
        let ids = ids(10, 20);
        let offs: Vec<u64> = (0..10).map(|i| 12 + i * 100).collect();
        for v in [1, 2] {
            let idx = build_index(v, &ids, &offs);
            let view = IndexView::parse(&idx, 20).unwrap();
            assert_eq!((view.version(), view.num_objects()), (v, 10));

            // v1's size is exact; v2's has room for a large-offset table.
            let mut bad = idx.clone();
            bad.push(0);
            if v == 1 {
                assert_eq!(IndexView::parse(&bad, 20).err(), Some("index is corrupted"));
            } else {
                assert!(IndexView::parse(&bad, 20).is_ok());
            }
            let bad = &idx[..idx.len() - 1];
            let want = if v == 1 {
                "index is corrupted"
            } else {
                "wrong index size"
            };
            assert_eq!(IndexView::parse(bad, 20).err(), Some(want));

            let mut bad = idx.clone();
            let fan = if v == 2 { 8 } else { 0 };
            bad[fan + 4 * 100..fan + 4 * 101].copy_from_slice(&1000u32.to_be_bytes());
            assert_eq!(
                IndexView::parse(&bad, 20).err(),
                Some("index is non-monotonic")
            );
        }
        let mut idx = build_index(2, &ids, &offs);
        idx[7] = 3;
        assert_eq!(
            IndexView::parse(&idx, 20).err(),
            Some("unsupported index version")
        );
        // v2 allows a large-offset table of up to nr - 1 entries.
        let mut idx = build_index(2, &ids, &offs);
        let trailer = idx.split_off(idx.len() - 40);
        idx.extend(core::iter::repeat_n(0, 9 * 8));
        idx.extend_from_slice(&trailer);
        assert!(IndexView::parse(&idx, 20).is_ok());
        idx.extend_from_slice(&[0; 8]);
        assert_eq!(IndexView::parse(&idx, 20).err(), Some("wrong index size"));
    }

    #[test]
    fn lookup_finds_ids_and_prefixes() {
        for (size, v) in [(20, 1), (20, 2), (32, 2), (32, 1)] {
            let ids = ids(300, size);
            let offs: Vec<u64> = (0..300).map(|i| 12 + i * 3).collect();
            let idx = build_index(v, &ids, &offs);
            let view = IndexView::parse(&idx, size).unwrap();
            for (i, id) in ids.iter().enumerate() {
                assert_eq!(view.lookup(&key(id), size * 2), Lookup::Found(i as u32));
                assert_eq!(view.oid_at(i as u32), &id[..]);
                assert_eq!(view.nth_offset(i as u32), offs[i] as i64);
            }
            // An absent full id.
            let mut absent = ids[17].clone();
            absent[size - 1] ^= 0xff;
            assert_eq!(view.lookup(&key(&absent), size * 2), Lookup::NotFound);
            // A unique prefix: the first 16 hex digits differ between ids.
            let mut p = key(&ids[42]);
            p[8..].fill(0);
            assert_eq!(view.lookup(&p, 16), Lookup::Found(42));
            // An odd-length prefix matches on the high nibble only.
            let mut p = key(&ids[42]);
            p[7] &= 0xf0;
            p[8..].fill(0);
            assert_eq!(view.lookup(&p, 15), Lookup::Found(42));
            // A prefix shared by neighbours is ambiguous; a 1-digit one too.
            let mut p = [0u8; GIT_OID_MAX_SIZE];
            p[0] = ids[150][0] & 0xf0;
            assert_eq!(view.lookup(&p, 1), Lookup::Ambiguous);
        }
    }

    /// Any index that parses can be searched and iterated without a panic,
    /// whatever its ids and offsets (corrupted indexes keep their layout).
    #[test]
    fn random_indexes_do_not_panic() {
        let mut seed = 7u64;
        let mut rnd = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as u32
        };
        let rounds = if cfg!(miri) { 20 } else { 2000 };
        for _ in 0..rounds {
            let size = if rnd() % 2 == 0 { 20 } else { 32 };
            let n = (rnd() % 40) as usize;
            let mut ids: Vec<Vec<u8>> = (0..n)
                .map(|_| (0..size).map(|_| rnd() as u8).collect())
                .collect();
            ids.sort();
            let offs: Vec<u64> = (0..n).map(|_| u64::from(rnd()) << (rnd() % 8)).collect();
            let v = 1 + rnd() % 2;
            let mut idx = build_index(v, &ids, &offs);
            // Scribble over the ids and offsets (not the fanout).
            let start = if v == 2 { 8 + 1024 } else { 1024 };
            for _ in 0..rnd() % 8 {
                if idx.len() > start {
                    let at = start + rnd() as usize % (idx.len() - start);
                    idx[at] = rnd() as u8;
                }
            }
            let Ok(view) = IndexView::parse(&idx, size) else {
                continue;
            };
            for _ in 0..20 {
                let mut k = [0u8; GIT_OID_MAX_SIZE];
                k.iter_mut().take(size).for_each(|b| *b = rnd() as u8);
                if n > 0 && rnd() % 2 == 0 {
                    k[..size].copy_from_slice(&ids[rnd() as usize % n]);
                }
                let len = 1 + rnd() as usize % (2 * size);
                if let Lookup::Found(p) = view.lookup(&k, len) {
                    let _ = view.nth_offset(p);
                    let _ = view.oid_at(p);
                }
            }
            let _ = view.for_each_offset(|_, _| Ok::<(), ()>(()), || ());
            let _ = view.positions_by_offset();
        }
    }

    #[test]
    fn large_offsets() {
        let ids = ids(4, 20);
        let offs = vec![12, 0x8000_0000, 0x1_2345_6789, 99];
        let idx = build_index(2, &ids, &offs);
        let view = IndexView::parse(&idx, 20).unwrap();
        for (i, &o) in offs.iter().enumerate() {
            assert_eq!(view.nth_offset(i as u32), o as i64);
        }
        let mut seen = Vec::new();
        view.for_each_offset(
            |id, off| {
                seen.push((id.to_vec(), off));
                Ok::<(), ()>(())
            },
            || (),
        )
        .unwrap();
        assert_eq!(
            seen,
            ids.iter()
                .cloned()
                .zip(offs.iter().map(|&o| o as i64))
                .collect::<Vec<_>>()
        );

        // A large offset index past the table. C's lookup refuses it only
        // within 8 bytes of the end (`index >= end - 8`: it reads the
        // checksum before that), its iteration within the checksum.
        let mut bad = idx.clone();
        let at = 8 + 1024 + 4 * 24 + 4;
        bad[at..at + 4].copy_from_slice(&0x8000_0005u32.to_be_bytes());
        let view = IndexView::parse(&bad, 20).unwrap();
        assert_eq!(view.nth_offset(1), 0xabab_abab_abab_abab_u64 as i64);
        assert_eq!(
            view.for_each_offset(|_, _| Ok(()), || "invalid"),
            Err("invalid")
        );
        bad[at..at + 4].copy_from_slice(&0x8000_0006u32.to_be_bytes());
        let view = IndexView::parse(&bad, 20).unwrap();
        assert_eq!(view.nth_offset(1), -1);
        // A 64-bit offset with the top bit set is negative: C's callers call
        // that corrupt.
        let mut bad = idx;
        let at = 8 + 1024 + 4 * 28;
        bad[at..at + 8].copy_from_slice(&u64::MAX.to_be_bytes());
        let view = IndexView::parse(&bad, 20).unwrap();
        assert_eq!(view.nth_offset(1), -1);
    }

    /// C's `(off32 & 0x7fffffff) * 8` is 32-bit and wraps (L-PACK-6).
    #[test]
    fn large_offset_index_wraps_as_c() {
        let ids = ids(4, 20);
        let offs = vec![12, 0x8000_0000, 0x1_2345_6789, 99];
        let mut idx = build_index(2, &ids, &offs);
        let at = 8 + 1024 + 4 * 24 + 4 * 3;
        // 2^29 * 8 == 2^32: entry 0 of the 64-bit table.
        idx[at..at + 4].copy_from_slice(&(0x8000_0000u32 | 1 << 29).to_be_bytes());
        let view = IndexView::parse(&idx, 20).unwrap();
        assert_eq!(view.nth_offset(3), 0x8000_0000);
    }

    /// A prefix longer than the hash is the full id (L-PACK-5: it panicked).
    #[test]
    fn long_prefixes_are_the_full_id() {
        let ids = ids(50, 20);
        let offs: Vec<u64> = (0..50).map(|i| 12 + i).collect();
        let idx = build_index(2, &ids, &offs);
        let view = IndexView::parse(&idx, 20).unwrap();
        for len in [40, 41, 63, 64, 1000] {
            assert_eq!(view.lookup(&key(&ids[7]), len), Lookup::Found(7));
        }
    }

    /// The cache from several threads: entries in use are never evicted,
    /// and what one thread adds the others get.
    #[test]
    fn base_cache_is_shared_between_threads() {
        let _g = crate::util::test_support::init();
        let cache = PackCache::new();
        cache.lock().memory_limit = 2000;
        let n = if cfg!(miri) { 30 } else { 2000 };
        std::thread::scope(|s| {
            for t in 0..4i64 {
                let cache = &cache;
                s.spawn(move || {
                    for i in 0..n {
                        let off = (i * 7 + t) % 64;
                        let held = cache.get(off).or_else(|| {
                            let data = GitBytes::copy_from(&[off as u8; 100]).unwrap();
                            cache.add(data, GIT_OBJECT_BLOB, off).ok()
                        });
                        if let Some(e) = held {
                            assert_eq!(e.data.as_bytes(), &[off as u8; 100]);
                        }
                    }
                });
            }
        });
        let c = cache.lock();
        assert_eq!(
            c.memory_used,
            c.entries.values().map(|e| e.data.len()).sum::<usize>()
        );
    }

    #[test]
    fn pack_order_is_a_stable_sort_by_offset() {
        let ids = ids(6, 20);
        let offs = vec![500, 12, 300, 12, 0x9000_0000, 40];
        for v in [1, 2] {
            let offs = if v == 1 {
                vec![500, 12, 300, 12, 7, 40]
            } else {
                offs.clone()
            };
            let idx = build_index(v, &ids, &offs);
            let view = IndexView::parse(&idx, 20).unwrap();
            let pos = view.positions_by_offset();
            // v2 sorts the 32-bit entries: a large offset sorts by its
            // (MSB-set) table index, after every small one.
            let want: Vec<u32> = if v == 1 {
                vec![4, 1, 3, 5, 2, 0]
            } else {
                vec![1, 3, 5, 2, 0, 4]
            };
            assert_eq!(pos, want, "v{v}");
        }
    }

    #[test]
    fn lookup_id_is_cs() {
        let table: Vec<u8> = (0u8..10).flat_map(|i| [i * 10, 0, 0]).collect();
        assert_eq!(lookup_id(&table, 3, 0, 10, &[30, 0], 2), 3);
        assert_eq!(lookup_id(&table, 3, 0, 10, &[31, 0], 2), -5);
        assert_eq!(lookup_id(&table, 3, 0, 10, &[0, 0], 2), 0);
        assert_eq!(lookup_id(&table, 3, 0, 10, &[255, 0], 2), -11);
        assert_eq!(lookup_id(&table, 3, 4, 4, &[30, 0], 2), -5);
        assert_eq!(lookup_id(&[], 3, 0, 0, &[1, 2], 2), -1);
    }

    #[test]
    fn raw_ncmp_reads_what_c_reads() {
        assert!(!raw_ncmp(&[0x12, 0x34], &[0x12, 0x3f], 3));
        assert!(raw_ncmp(&[0x12, 0x34], &[0x12, 0x4f], 3));
        assert!(!raw_ncmp(&[0x12], &[0x12], 2));
        assert!(!raw_ncmp(&[], &[], 0));
    }

    #[test]
    fn object_headers_round_trip() {
        for &size in &[
            0usize,
            1,
            15,
            16,
            127,
            128,
            1 << 20,
            usize::MAX >> 4,
            usize::MAX,
        ] {
            for kind in [GIT_OBJECT_COMMIT, GIT_OBJECT_BLOB, GIT_PACKFILE_REF_DELTA] {
                let mut hdr = [0u8; 10];
                let n = object_header(&mut hdr, size, kind).unwrap();
                let h = unpack_header1(&hdr[..n]).unwrap();
                assert_eq!((h.used, h.size, h.kind), (n, size, kind));
            }
        }
        let _g = crate::util::test_support::init();
        let mut hdr = [0u8; 10];
        assert!(object_header(&mut hdr, 1, 0).is_err());
        assert_eq!(
            errors::last_message().unwrap(),
            b"invalid argument: 'type >= GIT_OBJECT_COMMIT && type <= GIT_PACKFILE_REF_DELTA'"
        );
    }

    #[test]
    fn unpack_header1_errors() {
        let _g = crate::util::test_support::init();
        assert_eq!(unpack_header1(&[0x95, 0x80]), Err(ErrorCode::BUFS));
        assert_eq!(errors::last_message().unwrap(), b"buffer too small");
        assert_eq!(unpack_header1(&[]), Err(ErrorCode::BUFS));
        // 4 + 9 * 7 = 67 bits: the tenth continuation byte is refused.
        let long = [0xff; 12];
        assert_eq!(unpack_header1(&long), Err(ErrorCode::GENERIC));
        assert_eq!(errors::last_message().unwrap(), b"packfile corrupted");
        // Nine continuations still decode (bits past 64 are lost, as C's
        // `unsigned long`).
        let mut ok = [0xffu8; 11];
        ok[10] = 0x01;
        assert!(unpack_header1(&ok[1..]).is_ok());
    }

    #[test]
    fn ofs_distances() {
        let _g = crate::util::test_support::init();
        assert_eq!(decode_ofs(&[0x05]), Ok((5, 1)));
        // One continuation: ((c0 & 127) + 1) << 7 + (c1 & 127).
        assert_eq!(decode_ofs(&[0x80, 0x00]), Ok((128, 2)));
        assert_eq!(decode_ofs(&[0x81, 0x7f]), Ok(((2 << 7) + 127, 2)));
        assert_eq!(decode_ofs(&[0x81]), Err(ErrorCode::BUFS));
        assert_eq!(decode_ofs(&[]), Err(ErrorCode::BUFS));
        assert_eq!(decode_ofs(&[0xff; 20]), Err(ErrorCode::GENERIC));
        assert_eq!(
            errors::last_message().unwrap(),
            b"invalid pack file - overflow"
        );
    }

    #[test]
    fn cycles_are_found() {
        let _g = crate::util::test_support::init();
        // A chain 100 -> 90 -> ... -> 10 -> 50 -> 40 -> ... loops.
        let next = |o: i64| if o == 10 { 50 } else { o - 10 };
        let mut c = CycleCheck::new(100);
        let mut o = 100;
        let mut steps = 0;
        let r = loop {
            o = next(o);
            steps += 1;
            if let Err(e) = c.step(o) {
                break e;
            }
            assert!(steps < 100);
        };
        assert_eq!(r, ErrorCode::GENERIC);
        assert_eq!(
            errors::last_message().unwrap(),
            b"invalid pack file - delta chain loops"
        );
        // A long chain that does not loop is not refused.
        let n = if cfg!(miri) { 5_000 } else { 1_000_000 };
        let mut c = CycleCheck::new(n);
        for o in (0..n).rev() {
            c.step(o).unwrap();
        }
    }

    #[test]
    fn chain_spills_to_the_heap() {
        let _g = crate::util::test_support::init();
        let mut c = Chain::new();
        for i in 0..200 {
            c.push(ChainElem {
                base_key: i,
                ..ChainElem::default()
            })
            .unwrap()
            .offset = i * 2;
            assert_eq!(chain_last(&mut c).base_key, i);
        }
        for i in 0..200 {
            assert_eq!(
                (c.get(i).base_key, c.get(i).offset),
                (i as i64, i as i64 * 2)
            );
        }
    }

    #[test]
    fn base_cache_evicts_unused_entries() {
        let _g = crate::util::test_support::init();
        let cache = PackCache::new();
        cache.lock().memory_limit = 100;
        let big = GitBytes::copy_from(&[1; 2 * GIT_PACK_CACHE_SIZE_LIMIT]).unwrap();
        assert!(cache.add(big, GIT_OBJECT_BLOB, 1).is_err());
        let a = cache
            .add(GitBytes::copy_from(&[1; 40]).unwrap(), GIT_OBJECT_BLOB, 10)
            .unwrap();
        assert!(
            cache
                .add(GitBytes::copy_from(&[2; 40]).unwrap(), GIT_OBJECT_BLOB, 10)
                .is_err()
        );
        let b = cache
            .add(GitBytes::copy_from(&[3; 40]).unwrap(), GIT_OBJECT_TREE, 20)
            .unwrap();
        drop(b);
        // 120 > 100: the unused entry (20) goes, the used one (10) stays.
        let c = cache
            .add(GitBytes::copy_from(&[4; 40]).unwrap(), GIT_OBJECT_TAG, 30)
            .unwrap();
        assert!(cache.get(20).is_none());
        assert_eq!(cache.get(10).unwrap().data.as_bytes(), &[1; 40]);
        // All in use and over the limit: added anyway (C loops forever).
        let d = cache
            .add(GitBytes::copy_from(&[5; 40]).unwrap(), GIT_OBJECT_TAG, 40)
            .unwrap();
        assert_eq!(cache.lock().memory_used, 120);
        assert_eq!(d.kind, GIT_OBJECT_TAG);
        drop((a, c, d));
    }

    #[test]
    fn git_bytes() {
        let _g = crate::util::test_support::init();
        let b = GitBytes::copy_from(b"abc").unwrap();
        assert_eq!(b.as_bytes(), b"abc");
        let (p, n) = b.into_raw();
        // SAFETY: the buffer `into_raw` gave up: 3 bytes and a NUL.
        let b = unsafe { GitBytes::from_raw(NonNull::new(p).unwrap(), n) };
        // SAFETY: as above: byte 3 is the NUL.
        let nul = unsafe { p.add(3) };
        // SAFETY: as above.
        assert_eq!(unsafe { *nul }, 0);
        assert!(!b.is_empty());
        assert!(GitBytes::copy_from(b"").unwrap().is_empty());
    }
}
