//! Port of `src/util/sortedcache.c`: `git_sortedcache`, the parsed contents
//! of a backing file (`packed-refs`), looked up by key and walked in sorted
//! order, under a reader-writer lock that the caller takes and releases.
//!
//! `git_sortedcache` is opaque (kind 1): the C that stays (`refdb_fs.c`, the
//! tests) only passes pointers to it, and reads its path through
//! `git_sortedcache_path`. It is an `Arc<Shared>` whose pointer C holds;
//! `git_sortedcache_incref` / `git_sortedcache_free` are the `Arc`'s count.
//!
//! Each item is a zeroed block of the cache's arena (C's `git_pool`) whose
//! key, NUL-terminated, starts `item_path_offset` bytes in; C fills the
//! bytes before it. The arena, the sorted vector of items (a [`GitVector`]
//! with the caller's comparison) and the key map live in [`Core`], behind
//! the cache's `git_rwlock`:
//!
//! - the C face follows the C contract: the write lock (or exclusive use)
//!   for changes, a read lock (or the write lock) for lookups;
//! - the safe API ([`SortedCache`]) takes the lock in guards.
//!
//! The comparison and free callbacks are C. No Rust reference into the
//! cache is held while a free or copy callback runs; the comparison runs
//! during sorts and must not use the cache.

use core::cell::UnsafeCell;
use core::ffi::{CStr, c_char, c_int, c_void};
use core::hash::{BuildHasherDefault, Hash, Hasher};
use core::marker::PhantomData;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::ffi::CString;
use std::sync::Arc;

use crate::git_error_set;
use crate::sys::pending::{self, FileStamp};
use crate::util::alloc;
use crate::util::errors::{self, Error, ErrorClass, ErrorCode, Result};
use crate::util::integer::alloc_add;
use crate::util::str::GitStr;
use crate::util::thread::GitRwLock;
use crate::util::vector::{GitVector, VectorCmp};

/// `git_sortedcache_free_item_fn`.
pub type FreeItemFn = unsafe extern "C" fn(payload: *mut c_void, item: *mut c_void);
/// The `copy_item` of `git_sortedcache_copy`.
pub type CopyItemFn = unsafe extern "C" fn(
    payload: *mut c_void,
    tgt_item: *mut c_void,
    src_item: *mut c_void,
) -> c_int;

// ---------------------------------------------------------------------------
// The arena (git_pool with an item size of 1)
// ---------------------------------------------------------------------------

/// `sizeof(git_pool_page)`: C allocates each page with its header.
const POOL_PAGE_HEADER: usize = 24;

/// `git_pool`'s page size: the system page less the `malloc` overhead and
/// the page header (`git_pool_global_init`).
fn pool_page_size() -> usize {
    static SIZE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *SIZE.get_or_init(|| {
        // SAFETY: `sysconf` has no preconditions.
        let n = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        let page = usize::try_from(n).ok().filter(|&n| n > 0).unwrap_or(4096);
        page - (2 * core::mem::size_of::<*const ()>()) - POOL_PAGE_HEADER
    })
}

struct Page {
    /// From `git__malloc`, `POOL_PAGE_HEADER + size` bytes, 8-aligned.
    base: NonNull<u8>,
    size: usize,
    avail: usize,
}

/// Blocks that live until the arena is cleared, as `git_pool` hands out:
/// the same page sizes and allocation pattern, through `git__allocator`.
#[derive(Default)]
struct Arena {
    /// The newest page last (C's list head).
    pages: Vec<Page>,
}

impl Arena {
    /// `git_pool_mallocz`: `size` zeroed bytes (a multiple of 8, so blocks
    /// stay 8-aligned), or the out-of-memory error.
    fn alloc_zeroed(&mut self, size: usize) -> Result<NonNull<u8>> {
        let p = match self.pages.last_mut() {
            Some(page) if page.avail >= size => {
                let off = POOL_PAGE_HEADER + page.size - page.avail;
                page.avail -= size;
                // SAFETY: `off + size <= POOL_PAGE_HEADER + page.size`, within
                // the page's allocation.
                unsafe { page.base.add(off) }
            }
            _ => self.alloc_page(size)?,
        };
        // SAFETY: `size` bytes at `p` belong to this block alone.
        unsafe { p.as_ptr().write_bytes(0, size) };
        Ok(p)
    }

    fn alloc_page(&mut self, size: usize) -> Result<NonNull<u8>> {
        let page_size = pool_page_size();
        let new_page_size = if size <= page_size { page_size } else { size };
        let alloc_size = new_page_size
            .checked_add(POOL_PAGE_HEADER)
            .ok_or_else(errors::oom)?;
        self.pages.try_reserve(1).map_err(|_| errors::oom())?;
        let base =
            NonNull::new(alloc::malloc(alloc_size).cast::<u8>()).ok_or(ErrorCode::GENERIC)?;
        self.pages.push(Page {
            base,
            size: new_page_size,
            avail: new_page_size - size,
        });
        // SAFETY: the header is within the allocation.
        Ok(unsafe { base.add(POOL_PAGE_HEADER) })
    }

    /// `git_pool_clear`.
    fn clear(&mut self) {
        for page in self.pages.drain(..) {
            // SAFETY: allocated by `git__malloc` in `alloc_page`, freed once.
            unsafe { alloc::free(page.base.as_ptr().cast()) };
        }
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        self.clear();
    }
}

// ---------------------------------------------------------------------------
// The key map (git_hashmap_str)
// ---------------------------------------------------------------------------

/// A key in an item: `len` bytes at `ptr`, which stay allocated and
/// unwritten while the key is in the map (the arena outlives the map's
/// entries: `Core::clear` empties the map first, and the key bytes are the
/// cache's, which C does not write).
#[derive(Clone, Copy)]
struct KeyRef {
    ptr: NonNull<u8>,
    len: usize,
}

impl KeyRef {
    fn bytes(&self) -> &[u8] {
        // SAFETY: the type's invariant: `len` live, unwritten bytes.
        unsafe { core::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl core::borrow::Borrow<[u8]> for KeyRef {
    fn borrow(&self) -> &[u8] {
        self.bytes()
    }
}
impl PartialEq for KeyRef {
    fn eq(&self, other: &Self) -> bool {
        self.bytes() == other.bytes()
    }
}
impl Eq for KeyRef {}
impl Hash for KeyRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bytes().hash(state);
    }
}

/// A fast multiplicative hasher (as rustc's FxHasher); the C's string
/// hashmap is not DoS-resistant either.
#[derive(Default)]
struct FxHasher(u64);

const FX_K: u64 = 0xf135_7aea_2e62_a9c5;

impl FxHasher {
    #[inline]
    fn add(&mut self, w: u64) {
        self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(FX_K);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rest) = bytes.as_chunks::<8>();
        for c in chunks {
            self.add(u64::from_le_bytes(*c));
        }
        if !rest.is_empty() {
            let mut w = [0u8; 8];
            w[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(w) ^ ((rest.len() as u64) << 59));
        }
    }
    #[inline]
    fn write_usize(&mut self, n: usize) {
        self.add(n as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

type KeyMap = HashMap<KeyRef, NonNull<u8>, BuildHasherDefault<FxHasher>>;

// ---------------------------------------------------------------------------
// The core: arena, items, map
// ---------------------------------------------------------------------------

/// The state behind the lock. Item pointers are blocks of `arena`, valid
/// until the next `clear`.
struct Core {
    item_path_offset: usize,
    arena: Arena,
    items: GitVector,
    map: KeyMap,
}

impl Core {
    /// The key of `item`.
    ///
    /// # Safety
    ///
    /// `item` is a block of this core's arena, from `upsert`.
    unsafe fn key_of(&self, item: *mut c_void) -> &CStr {
        // SAFETY: the block holds a NUL-terminated key at the offset,
        // written by `upsert` and never by C.
        unsafe { CStr::from_ptr(item.cast::<c_char>().wrapping_add(self.item_path_offset)) }
    }

    fn lookup(&self, key: &[u8]) -> Option<NonNull<u8>> {
        self.map.get(key).copied()
    }

    /// `git_sortedcache_upsert`: the item for `key` (without NUL bytes),
    /// inserted zeroed (but for its key) if new. `Ok((item, true))` when
    /// inserted.
    fn upsert(&mut self, key: &[u8]) -> Result<(NonNull<u8>, bool)> {
        if let Some(item) = self.lookup(key) {
            return Ok((item, false));
        }
        let itemlen = alloc_add(alloc_add(self.item_path_offset, key.len())?, 1)?;
        let itemlen = alloc_add(itemlen, 7)? & !7;
        let item = self.arena.alloc_zeroed(itemlen)?;

        // As in C, a failed insert below abandons the block in the arena.
        // SAFETY: the block has `offset + len + 1` bytes; the key goes after
        // the offset (its NUL is already there, from the zeroing).
        let key_ptr = unsafe { item.add(self.item_path_offset) };
        // SAFETY: `key.len()` bytes within the block, disjoint from `key`.
        unsafe { core::ptr::copy_nonoverlapping(key.as_ptr(), key_ptr.as_ptr(), key.len()) };

        if self.map.try_reserve(1).is_err() {
            return Err(errors::oom());
        }
        let kref = KeyRef {
            ptr: key_ptr,
            len: key.len(),
        };
        self.map.insert(kref, item);
        if let Err(e) = self.items.insert(item.as_ptr().cast()) {
            self.map.remove(key);
            return Err(e);
        }
        Ok((item, true))
    }

    /// Sort the items (`git_vector_sort`).
    ///
    /// # Safety
    ///
    /// The items' comparison function is sound to call on the items.
    unsafe fn sort(&mut self) {
        // SAFETY: forwarded.
        unsafe { self.items.sort() };
    }

    /// `git_sortedcache_lookup_index` (after the items are sorted).
    fn index_of(&self, key: &[u8]) -> core::result::Result<usize, usize> {
        self.items.bsearch_by(|item| {
            // SAFETY: the items are blocks of this arena.
            let k = unsafe { self.key_of(item) };
            crate::util::util::strcmp(key, k.to_bytes())
        })
    }

    /// Remove the item at `pos` from the vector and the map (the block
    /// stays in the arena). `None` when out of range.
    fn take(&mut self, pos: usize) -> Option<NonNull<u8>> {
        let item = NonNull::new(self.items.get(pos)?.cast::<u8>())?;
        let _ = self.items.remove(pos);
        // SAFETY: a block of this arena, whose key stays allocated (the
        // block is not freed here).
        let key = unsafe { self.key_of(item.as_ptr().cast()) };
        let kref = KeyRef {
            ptr: NonNull::from(key.to_bytes()).cast(),
            len: key.to_bytes().len(),
        };
        self.map.remove(kref.bytes());
        Some(item)
    }
}

/// The cache: what `git_sortedcache *` points to (an `Arc`'s contents).
pub struct Shared {
    lock: GitRwLock,
    free_item: Option<FreeItemFn>,
    free_item_payload: *mut c_void,
    /// `git_vector_length(&items)`, readable without the lock.
    count: AtomicUsize,
    stamp: UnsafeCell<FileStamp>,
    core: UnsafeCell<Core>,
    path: CString,
}

// SAFETY: `core` and `stamp` are only reached under the rwlock (or with
// exclusive use), per the C contract and the guards of the safe API; the
// payload and callbacks are the caller's, which C shares across threads
// as the C did (the refdb's cache is used from several threads).
unsafe impl Sync for Shared {}
// SAFETY: as above; the arena's memory is `git__allocator`'s, freed from
// any thread.
unsafe impl Send for Shared {}

impl Shared {
    /// `git_sortedcache_new`.
    fn new(
        item_path_offset: usize,
        free_item: Option<FreeItemFn>,
        free_item_payload: *mut c_void,
        item_cmp: Option<VectorCmp>,
        path: &[u8],
    ) -> Result<Arc<Shared>> {
        // C sizes `sizeof(git_sortedcache) + pathlen + 1`.
        alloc_add(alloc_add(core::mem::size_of::<Shared>(), path.len())?, 1)?;
        let items = GitVector::with_capacity(4, item_cmp)?;
        let path = CString::new(crate::util::net::c_prefix(path)).unwrap_or_default();
        Ok(Arc::new(Shared {
            lock: GitRwLock::new(),
            free_item,
            free_item_payload,
            count: AtomicUsize::new(0),
            stamp: UnsafeCell::new(FileStamp::default()),
            core: UnsafeCell::new(Core {
                item_path_offset,
                arena: Arena::default(),
                items,
                map: KeyMap::default(),
            }),
            path,
        }))
    }

    /// The shared core.
    ///
    /// # Safety
    ///
    /// The caller holds a read or the write lock (or has exclusive use),
    /// and nothing changes the core while the reference lives.
    #[allow(clippy::mut_from_ref)]
    unsafe fn core(&self) -> &Core {
        // SAFETY: the caller's contract.
        unsafe { &*self.core.get() }
    }

    /// The core, exclusively.
    ///
    /// # Safety
    ///
    /// The caller holds the write lock (or has exclusive use), and no
    /// other reference to the core lives meanwhile.
    #[allow(clippy::mut_from_ref)]
    unsafe fn core_mut(&self) -> &mut Core {
        // SAFETY: the caller's contract.
        unsafe { &mut *self.core.get() }
    }

    fn sync_count(&self, core: &Core) {
        self.count.store(core.items.len(), Ordering::Release);
    }

    /// `git_sortedcache_path`.
    pub fn path(&self) -> &CStr {
        &self.path
    }

    /// `git_sortedcache_entrycount` (without the lock: may be stale).
    pub fn entrycount(&self) -> usize {
        self.count.load(Ordering::Acquire)
    }

    /// `git_sortedcache_wlock`. pthread reports failure with a positive
    /// value, which the C's `< 0` test never sees; so here.
    fn wlock(&self) -> Result<()> {
        // SAFETY: an initialized rwlock at a stable address (inside the Arc).
        let r = unsafe { libc::pthread_rwlock_wrlock(self.lock.as_ptr()) };
        if r < 0 {
            let _ = git_error_set!(ErrorClass::OS, "unable to acquire write lock on cache");
            return Err(ErrorCode::GENERIC);
        }
        Ok(())
    }

    /// `git_sortedcache_wunlock`: sort, then release the write lock.
    ///
    /// # Safety
    ///
    /// The calling thread holds the write lock; the comparison is sound on
    /// the items.
    unsafe fn wunlock(&self) {
        {
            // SAFETY: the write lock is held; the reference ends here.
            let core = unsafe { self.core_mut() };
            // SAFETY: forwarded.
            unsafe { core.sort() };
        }
        // SAFETY: held by this thread.
        unsafe { libc::pthread_rwlock_unlock(self.lock.as_ptr()) };
    }

    /// `git_sortedcache_rlock`.
    fn rlock(&self) -> Result<()> {
        // SAFETY: an initialized rwlock at a stable address.
        let r = unsafe { libc::pthread_rwlock_rdlock(self.lock.as_ptr()) };
        if r < 0 {
            let _ = git_error_set!(ErrorClass::OS, "unable to acquire read lock on cache");
            return Err(ErrorCode::GENERIC);
        }
        Ok(())
    }

    /// `git_sortedcache_runlock`.
    ///
    /// # Safety
    ///
    /// The calling thread holds a read lock.
    unsafe fn runlock(&self) {
        // SAFETY: held by this thread.
        unsafe { libc::pthread_rwlock_unlock(self.lock.as_ptr()) };
    }

    /// `sortedcache_clear`: forget the keys, call `free_item` on every
    /// item (in order), drop them and free the arena.
    ///
    /// # Safety
    ///
    /// The write lock is held (or exclusive use); `free_item` is sound to
    /// call with the payload and each item.
    unsafe fn clear_items(&self) {
        {
            // SAFETY: the write lock; the reference ends here.
            let core = unsafe { self.core_mut() };
            core.map.clear();
        }
        if let Some(free_item) = self.free_item {
            let mut i = 0;
            loop {
                // SAFETY: the lock is held; the shared reference ends before
                // the callback runs.
                let item = unsafe { self.core() }.items.get(i);
                let Some(item) = item else { break };
                // SAFETY: the caller vouches for the callback.
                unsafe { free_item(self.free_item_payload, item) };
                i += 1;
            }
        }
        // SAFETY: the write lock; no other reference.
        let core = unsafe { self.core_mut() };
        core.items.clear();
        core.arena.clear();
        self.sync_count(core);
    }

    /// `git_sortedcache_upsert`.
    ///
    /// # Safety
    ///
    /// The write lock is held (or exclusive use).
    unsafe fn upsert(&self, key: &[u8]) -> Result<(NonNull<u8>, bool)> {
        // SAFETY: the caller's contract.
        let core = unsafe { self.core_mut() };
        let r = core.upsert(key);
        self.sync_count(core);
        r
    }

    /// `git_sortedcache_remove`.
    ///
    /// # Safety
    ///
    /// The write lock is held (or exclusive use); `free_item` is sound.
    unsafe fn remove(&self, pos: usize) -> Result<()> {
        let item = {
            // SAFETY: the caller's contract; the reference ends here.
            let core = unsafe { self.core_mut() };
            let item = core.take(pos);
            self.sync_count(core);
            item
        };
        let Some(item) = item else {
            let _ = git_error_set!(ErrorClass::INVALID, "removing item out of range");
            return Err(ErrorCode::NOT_FOUND);
        };
        if let Some(free_item) = self.free_item {
            // SAFETY: the caller vouches for the callback.
            unsafe { free_item(self.free_item_payload, item.as_ptr().cast()) };
        }
        Ok(())
    }

    /// `git_sortedcache_lockandload`: take the write lock; if the file
    /// changed, read it into `buf` (if any) and return `Ok(true)` with the
    /// lock held; otherwise release the lock (`Ok(false)` or the error).
    ///
    /// # Safety
    ///
    /// The comparison is sound on the items (the lock is released with a
    /// sort on the paths that do not return `Ok(true)`).
    unsafe fn lockandload(&self, buf: Option<&mut GitStr>) -> Result<bool> {
        self.wlock()?;
        // SAFETY: forwarded.
        let r = unsafe { self.load_locked(buf) };
        if !matches!(r, Ok(true)) {
            // SAFETY: the lock is held; forwarded.
            unsafe { self.wunlock() };
        }
        r
    }

    /// # Safety
    ///
    /// The write lock is held.
    unsafe fn load_locked(&self, buf: Option<&mut GitStr>) -> Result<bool> {
        // SAFETY: the write lock is held; the reference ends with the call.
        let stamp = unsafe { &mut *self.stamp.get() };
        let r = pending::futils_filestamp_check(stamp, &self.path);
        if r <= 0 {
            return ErrorCode::check(r).map(|_| false);
        }

        let fd = pending::futils_open_ro(&self.path)?;
        let r = load_fd(fd, buf);
        // SAFETY: `fd` is ours, closed once.
        unsafe { libc::close(fd) };
        r.map(|()| true)
    }

    /// `git_sortedcache_updated`.
    ///
    /// # Safety
    ///
    /// The write lock is held.
    unsafe fn updated(&self) {
        // SAFETY: the write lock is held.
        let stamp = unsafe { &mut *self.stamp.get() };
        pending::futils_filestamp_check(stamp, &self.path);
    }

    /// `git_sortedcache_entry`: the item at `pos` (in sorted order).
    ///
    /// # Safety
    ///
    /// A lock is held; if the items may be unsorted (after upserts, before
    /// `wunlock`), it is the write lock (or exclusive use). The comparison
    /// is sound on the items.
    unsafe fn entry(&self, pos: usize) -> Option<*mut c_void> {
        // SAFETY: a lock is held; the reference ends here.
        let sorted = unsafe { self.core() }.items.is_sorted();
        if !sorted {
            // SAFETY: unsorted: the caller holds the write lock.
            let core = unsafe { self.core_mut() };
            // SAFETY: the comparison is sound on the items.
            unsafe { core.sort() };
        }
        // SAFETY: a lock is held.
        unsafe { self.core() }.items.get(pos)
    }

    /// `git_sortedcache_lookup_index`.
    ///
    /// # Safety
    ///
    /// As [`entry`](Self::entry).
    unsafe fn lookup_index(&self, key: &[u8]) -> (Result<()>, usize) {
        let (has_cmp, sorted) = {
            // SAFETY: as in `entry`; the reference ends here.
            let core = unsafe { self.core() };
            (core.items.cmp_fn().is_some(), core.items.is_sorted())
        };
        if !has_cmp {
            // `git_vector_bsearch2`: -1 with no error.
            return (Err(ErrorCode::GENERIC), 0);
        }
        if !sorted {
            // SAFETY: unsorted: the caller holds the write lock.
            let core = unsafe { self.core_mut() };
            // SAFETY: the comparison is sound on the items.
            unsafe { core.sort() };
        }
        // SAFETY: a lock is held.
        match unsafe { self.core() }.index_of(key) {
            Ok(p) => (Ok(()), p),
            Err(p) => (Err(ErrorCode::NOT_FOUND), p),
        }
    }
}

impl Drop for Shared {
    /// `sortedcache_free`: take the write lock (everyone else is done),
    /// clear (calling `free_item`), release and destroy the lock.
    fn drop(&mut self) {
        if self.wlock().is_err() {
            return;
        }
        // SAFETY: the write lock is held; the C contract makes `free_item`
        // sound for the cache's items until the cache is freed.
        unsafe { self.clear_items() };
        // SAFETY: held by this thread.
        unsafe { libc::pthread_rwlock_unlock(self.lock.as_ptr()) };
        // SAFETY: unlocked, and never used again.
        unsafe { self.lock.destroy() };
    }
}

/// The rest of `lockandload` after opening: stat, size check, read.
fn load_fd(fd: c_int, buf: Option<&mut GitStr>) -> Result<()> {
    // SAFETY: `stat` is plain data; all-zero is a valid value.
    let mut st: libc::stat = unsafe { core::mem::zeroed() };
    // SAFETY: `fd` is open; `st` is writable.
    if unsafe { libc::fstat(fd, &raw mut st) } < 0 {
        let _ = git_error_set!(ErrorClass::OS, "failed to stat file");
        return Err(ErrorCode::GENERIC);
    }
    let Ok(size) = usize::try_from(st.st_size) else {
        let _ = git_error_set!(
            ErrorClass::INVALID,
            "unable to load file larger than size_t"
        );
        return Err(ErrorCode::GENERIC);
    };
    match buf {
        Some(buf) => pending::futils_readbuffer_fd(buf, fd, size),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// The safe API
// ---------------------------------------------------------------------------

/// A sorted cache of `T` by byte-string key, with the contents of a backing
/// file loaded on demand (`git_sortedcache`).
///
/// Items are kept in key order (bytewise). Clones share the cache (C's
/// reference count). Reading and writing take the cache's lock in guards:
/// [`read`](Self::read), [`write`](Self::write),
/// [`lock_and_load`](Self::lock_and_load).
pub struct SortedCache<T> {
    shared: Arc<Shared>,
    _t: PhantomData<T>,
}

// SAFETY: the items are reached only through the guards: shared (`&T`)
// under a read lock from any thread, which needs `T: Sync`; exclusive
// (`&mut T`, drop) under the write lock, from any thread, which needs
// `T: Send`.
unsafe impl<T: Send + Sync> Sync for SortedCache<T> {}
// SAFETY: as above.
unsafe impl<T: Send + Sync> Send for SortedCache<T> {}

impl<T> Clone for SortedCache<T> {
    /// Another reference to the same cache (`git_sortedcache_incref`).
    fn clone(&self) -> Self {
        SortedCache {
            shared: Arc::clone(&self.shared),
            _t: PhantomData,
        }
    }
}

/// The item's key offset: right after the `T`.
const fn key_offset<T>() -> usize {
    core::mem::size_of::<T>()
}

/// The comparison of two typed items: their keys, bytewise.
unsafe extern "C" fn typed_cmp<T>(a: *const c_void, b: *const c_void) -> c_int {
    // SAFETY: called by the vector on two items of a `SortedCache<T>`, each
    // a block with its key at `key_offset::<T>()`.
    let ka = unsafe { CStr::from_ptr(a.cast::<c_char>().wrapping_add(key_offset::<T>())) };
    // SAFETY: as above.
    let kb = unsafe { CStr::from_ptr(b.cast::<c_char>().wrapping_add(key_offset::<T>())) };
    crate::util::util::strcmp(ka.to_bytes(), kb.to_bytes())
}

/// The `free_item` of a typed cache: drop the `T`.
unsafe extern "C" fn typed_drop<T>(_payload: *mut c_void, item: *mut c_void) {
    // SAFETY: every item of a `SortedCache<T>` in its vector holds an
    // initialized `T` at offset 0 (written right after the upsert), dropped
    // only here, once, when the item leaves the cache.
    unsafe { core::ptr::drop_in_place(item.cast::<T>()) };
}

impl<T> SortedCache<T> {
    const ALIGN_OK: () = assert!(core::mem::align_of::<T>() <= 8, "items are 8-aligned");

    /// A cache backed by the file `path` (if any) (`git_sortedcache_new`).
    pub fn new(path: Option<&[u8]>) -> core::result::Result<Self, Error> {
        let () = Self::ALIGN_OK;
        let shared = Shared::new(
            key_offset::<T>(),
            Some(typed_drop::<T>),
            core::ptr::null_mut(),
            Some(typed_cmp::<T>),
            path.unwrap_or(b""),
        )
        .map_err(Error::from)?;
        Ok(SortedCache {
            shared,
            _t: PhantomData,
        })
    }

    /// The backing file's path (`git_sortedcache_path`), empty if none.
    pub fn path(&self) -> &[u8] {
        self.shared.path().to_bytes()
    }

    /// The number of items, without the lock (it may change at once).
    pub fn len(&self) -> usize {
        self.shared.entrycount()
    }

    /// Whether there are no items (without the lock).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Take a read lock (`git_sortedcache_rlock`).
    pub fn read(&self) -> ReadGuard<'_, T> {
        let _ = self.shared.rlock();
        ReadGuard {
            cache: self,
            _not_send: PhantomData,
        }
    }

    /// Take the write lock (`git_sortedcache_wlock`); the items are sorted
    /// when it is released.
    pub fn write(&self) -> WriteGuard<'_, T> {
        let _ = self.shared.wlock();
        WriteGuard {
            cache: self,
            _not_send: PhantomData,
        }
    }

    /// `git_sortedcache_lockandload`: if the backing file changed since the
    /// last load (or `updated`), its contents with the write lock held, to
    /// refill the cache; `None` if unchanged. A missing file is an error
    /// with code [`ErrorCode::NOT_FOUND`] (without a message, as C).
    pub fn lock_and_load(&self) -> core::result::Result<Option<Loaded<'_, T>>, Error> {
        let mut buf = GitStr::new();
        // SAFETY: the comparison is `typed_cmp::<T>`, sound on our items.
        match unsafe { self.shared.lockandload(Some(&mut buf)) } {
            Ok(true) => Ok(Some((
                WriteGuard {
                    cache: self,
                    _not_send: PhantomData,
                },
                buf.as_bytes().to_vec(),
            ))),
            Ok(false) => Ok(None),
            Err(e) => Err(Error::from(e)),
        }
    }
}

impl<T: Clone> SortedCache<T> {
    /// A new cache with copies of the items (`git_sortedcache_copy` with a
    /// read lock on this one).
    pub fn try_clone(&self) -> core::result::Result<SortedCache<T>, Error> {
        let path = self.path();
        let copy = SortedCache::<T>::new(if path.is_empty() { None } else { Some(path) })?;
        {
            let src = self.read();
            let mut dst = copy.write();
            for (key, item) in src.iter() {
                dst.upsert_with(key, || item.clone())?;
            }
        }
        Ok(copy)
    }
}

/// What [`SortedCache::lock_and_load`] gives when the file changed: the
/// write lock and the file's contents.
pub type Loaded<'a, T> = (WriteGuard<'a, T>, Vec<u8>);

/// A held read lock of a [`SortedCache`].
#[must_use]
pub struct ReadGuard<'a, T> {
    cache: &'a SortedCache<T>,
    _not_send: PhantomData<*const ()>,
}

/// The held write lock of a [`SortedCache`].
#[must_use]
pub struct WriteGuard<'a, T> {
    cache: &'a SortedCache<T>,
    _not_send: PhantomData<*const ()>,
}

/// Lookups, for both guards. The items are sorted whenever a read lock is
/// held (each write lock is released with a sort); under the write lock
/// the lookups by position sort first, as C.
macro_rules! lookups {
    () => {
        /// The item for `key` (`git_sortedcache_lookup`).
        pub fn get(&self, key: &[u8]) -> Option<&T> {
            let key = crate::util::net::c_prefix(key);
            // SAFETY: the guard holds a lock; the reference is bound to it.
            let item = unsafe { self.cache.shared.core() }.lookup(key)?;
            // SAFETY: an item of this cache holds an initialized `T`, which
            // no one writes while this lock (and the borrow of the guard)
            // lives.
            Some(unsafe { &*item.as_ptr().cast::<T>() })
        }

        /// The number of items (`git_sortedcache_entrycount`).
        pub fn len(&self) -> usize {
            // SAFETY: the guard holds a lock.
            unsafe { self.cache.shared.core() }.items.len()
        }

        /// Whether there are no items.
        pub fn is_empty(&self) -> bool {
            self.len() == 0
        }

        /// The key and item at `pos`, in key order (`git_sortedcache_entry`).
        pub fn entry(&self, pos: usize) -> Option<(&[u8], &T)> {
            // SAFETY: the guard holds a lock; the items are sorted under a
            // read lock, and the write guard has exclusive use; the
            // comparison is `typed_cmp::<T>`.
            let item = unsafe { self.cache.shared.entry(pos) }?;
            // SAFETY: an item of this cache (see `get`), its key alive as
            // long as the lock.
            unsafe {
                let core = self.cache.shared.core();
                Some((core.key_of(item).to_bytes(), &*item.cast::<T>()))
            }
        }

        /// The position of `key` in key order (`git_sortedcache_lookup_index`).
        pub fn index_of(&self, key: &[u8]) -> Option<usize> {
            let key = crate::util::net::c_prefix(key);
            // SAFETY: as in `entry`.
            let (r, pos) = unsafe { self.cache.shared.lookup_index(key) };
            r.ok().map(|()| pos)
        }

        /// The keys and items, in key order.
        pub fn iter(&self) -> impl Iterator<Item = (&[u8], &T)> + '_ {
            (0..self.len()).filter_map(move |i| self.entry(i))
        }
    };
}

impl<T> ReadGuard<'_, T> {
    lookups!();
}

impl<T> WriteGuard<'_, T> {
    lookups!();

    /// The item for `key`, inserted as `init()` if absent
    /// (`git_sortedcache_upsert`).
    pub fn upsert_with(
        &mut self,
        key: &[u8],
        init: impl FnOnce() -> T,
    ) -> core::result::Result<&mut T, Error> {
        let key = crate::util::net::c_prefix(key);
        // SAFETY: the write lock is held.
        let existing = unsafe { self.cache.shared.core() }.lookup(key);
        let item = match existing {
            Some(item) => item,
            None => {
                let value = init();
                // SAFETY: the write lock is held.
                let (item, inserted) =
                    unsafe { self.cache.shared.upsert(key) }.map_err(Error::from)?;
                debug_assert!(inserted);
                // SAFETY: a fresh block, 8-aligned (`ALIGN_OK`), with room
                // for a `T` before the key; written before anything can
                // drop it.
                unsafe { item.as_ptr().cast::<T>().write(value) };
                item
            }
        };
        // SAFETY: an initialized `T` of this cache; the write guard is
        // borrowed mutably for the reference's life.
        Ok(unsafe { &mut *item.as_ptr().cast::<T>() })
    }

    /// The item for `key`, mutably.
    pub fn get_mut(&mut self, key: &[u8]) -> Option<&mut T> {
        let key = crate::util::net::c_prefix(key);
        // SAFETY: the write lock is held.
        let item = unsafe { self.cache.shared.core() }.lookup(key)?;
        // SAFETY: as in `upsert_with`.
        Some(unsafe { &mut *item.as_ptr().cast::<T>() })
    }

    /// Remove (and drop) the item at `pos` (`git_sortedcache_remove`).
    pub fn remove(&mut self, pos: usize) -> core::result::Result<(), Error> {
        // SAFETY: the write lock is held; the free callback drops the `T`.
        unsafe { self.cache.shared.remove(pos) }.map_err(Error::from)
    }

    /// Remove (and drop) every item (`git_sortedcache_clear`).
    pub fn clear(&mut self) {
        // SAFETY: the write lock is held; the free callback drops the `T`s.
        unsafe { self.cache.shared.clear_items() };
    }

    /// Record the backing file's current state as loaded
    /// (`git_sortedcache_updated`).
    pub fn updated(&mut self) {
        // SAFETY: the write lock is held.
        unsafe { self.cache.shared.updated() };
    }
}

impl<T> Drop for ReadGuard<'_, T> {
    fn drop(&mut self) {
        // SAFETY: this guard holds a read lock taken on this thread.
        unsafe { self.cache.shared.runlock() };
    }
}

impl<T> Drop for WriteGuard<'_, T> {
    fn drop(&mut self) {
        // SAFETY: this guard holds the write lock, taken on this thread; the
        // comparison is `typed_cmp::<T>`.
        unsafe { self.cache.shared.wunlock() };
    }
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `sortedcache.c`.
///
/// Every `git_sortedcache *` is one `git_sortedcache_new` returned (or
/// `git_sortedcache_copy`), not yet freed by its last reference. The
/// functions marked as needing a lock must be called with it held by the
/// calling thread, as `sortedcache.h` documents; `item_cmp` and `free_item`
/// must be sound on the items, and must not use the cache.
pub mod ffi {
    use super::*;
    use crate::util::export::{barrier, cstr_bytes, write_opt};

    /// The shared cache behind a C pointer.
    ///
    /// # Safety
    ///
    /// `sc` is a live cache (see the module).
    unsafe fn shared<'a>(sc: *const Shared) -> &'a Shared {
        // SAFETY: the caller's contract: an `Arc`'s live contents.
        unsafe { &*sc }
    }

    /// `git_sortedcache_new`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `path` is NULL or a NUL-terminated string;
    /// `free_item` and `item_cmp` follow the module's contract.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_new(
        out: *mut *const Shared,
        item_path_offset: usize,
        free_item: Option<FreeItemFn>,
        free_item_payload: *mut c_void,
        item_cmp: Option<VectorCmp>,
        path: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: NULL or a NUL-terminated string.
            let path = unsafe { cstr_bytes(path) };
            match Shared::new(
                item_path_offset,
                free_item,
                free_item_payload,
                item_cmp,
                path,
            ) {
                Ok(sc) => {
                    // SAFETY: `out` is writable.
                    unsafe { out.write(Arc::into_raw(sc)) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_sortedcache_copy`.
    ///
    /// # Safety
    ///
    /// `out` is writable; `src` a live cache, read-locked by the caller
    /// unless `lock`; `copy_item` (or, if NULL, a `memcpy` of the
    /// `item_path_offset` bytes before the key) is sound on its items and
    /// fresh ones.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_copy(
        out: *mut *const Shared,
        src: *const Shared,
        lock: bool,
        copy_item: Option<CopyItemFn>,
        payload: *mut c_void,
    ) -> c_int {
        barrier(|| {
            // SAFETY: a live cache.
            let s = unsafe { shared(src) };
            let (cmp, offset) = {
                // SAFETY: reads fields that never change.
                let core = unsafe { s.core() };
                (core.items.cmp_fn(), core.item_path_offset)
            };
            let tgt = match Shared::new(
                offset,
                s.free_item,
                s.free_item_payload,
                cmp,
                s.path.to_bytes(),
            ) {
                Ok(t) => t,
                Err(e) => return e.raw(),
            };
            if lock && s.rlock().is_err() {
                return -1;
            }

            let mut error = 0;
            let mut i = 0;
            loop {
                // SAFETY: `src` is read-locked; the reference ends here.
                let src_item = unsafe { s.core() }.items.get(i);
                let Some(src_item) = src_item else { break };
                i += 1;
                // SAFETY: `src` is read-locked.
                let src_core = unsafe { s.core() };
                // SAFETY: an item of `src`; its key lives under the lock.
                let key = unsafe { src_core.key_of(src_item) }.to_bytes();
                // SAFETY: `tgt` is ours alone.
                let tgt_item = match unsafe { tgt.upsert(key) } {
                    Ok((t, _)) => t,
                    Err(e) => {
                        error = e.raw();
                        break;
                    }
                };
                let r = match copy_item {
                    // SAFETY: the caller vouches for `copy_item`.
                    Some(f) => unsafe { f(payload, tgt_item.as_ptr().cast(), src_item) },
                    None => {
                        // `sortedcache_copy_item`: the bytes before the key.
                        // SAFETY: both blocks have `offset` bytes before
                        // their keys; distinct blocks.
                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                src_item.cast::<u8>(),
                                tgt_item.as_ptr(),
                                offset,
                            )
                        };
                        0
                    }
                };
                if r < 0 {
                    error = r;
                    break;
                }
            }

            if lock {
                // SAFETY: taken above on this thread.
                unsafe { s.runlock() };
            }
            if error != 0 {
                // `git_sortedcache_free(tgt)`: drops the last reference.
                drop(tgt);
                // SAFETY: `out` is writable.
                unsafe { out.write(core::ptr::null()) };
                return error;
            }
            // C leaves the copy unsorted until it is first read or
            // unlocked; sorting now gives the same order (rust/DIVERGENCES.md).
            // SAFETY: `tgt` is ours alone.
            let core = unsafe { tgt.core_mut() };
            // SAFETY: the comparison is `src`'s, sound on the items.
            unsafe { core.sort() };
            // SAFETY: `out` is writable.
            unsafe { out.write(Arc::into_raw(tgt)) };
            0
        })
    }

    /// `git_sortedcache_free`: drop a reference; the last frees the cache
    /// (calling `free_item` on the items).
    ///
    /// # Safety
    ///
    /// `sc` is NULL or a live cache whose reference the caller gives up;
    /// no lock on it is held.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_free(sc: *const Shared) {
        barrier(|| {
            if !sc.is_null() {
                // SAFETY: a reference from `Arc::into_raw`, given up.
                unsafe { Arc::decrement_strong_count(sc) };
            }
        })
    }

    /// `git_sortedcache_incref`.
    ///
    /// # Safety
    ///
    /// `sc` is a live cache.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_incref(sc: *const Shared) {
        // SAFETY: a live `Arc` pointer.
        barrier(|| unsafe { Arc::increment_strong_count(sc) })
    }

    /// `git_sortedcache_path`: valid as long as the cache.
    ///
    /// # Safety
    ///
    /// `sc` is a live cache.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_path(sc: *const Shared) -> *const c_char {
        // SAFETY: a live cache.
        barrier(|| unsafe { shared(sc) }.path().as_ptr())
    }

    /// `git_sortedcache_wlock`.
    ///
    /// # Safety
    ///
    /// `sc` is a live cache, not locked by this thread.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_wlock(sc: *const Shared) -> c_int {
        // SAFETY: a live cache.
        barrier(|| crate::util::export::status(unsafe { shared(sc) }.wlock()))
    }

    /// `git_sortedcache_wunlock`: sort the items, release the write lock.
    ///
    /// # Safety
    ///
    /// `sc` is a live cache write-locked by this thread.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_wunlock(sc: *const Shared) {
        barrier(|| {
            // SAFETY: a live cache.
            let s = unsafe { shared(sc) };
            // SAFETY: the caller's contract (the lock, the callbacks).
            unsafe { s.wunlock() }
        })
    }

    /// `git_sortedcache_rlock`.
    ///
    /// # Safety
    ///
    /// `sc` is a live cache not write-locked by this thread.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_rlock(sc: *const Shared) -> c_int {
        // SAFETY: a live cache.
        barrier(|| crate::util::export::status(unsafe { shared(sc) }.rlock()))
    }

    /// `git_sortedcache_runlock`.
    ///
    /// # Safety
    ///
    /// `sc` is a live cache read-locked by this thread.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_runlock(sc: *const Shared) {
        barrier(|| {
            // SAFETY: a live cache.
            let s = unsafe { shared(sc) };
            // SAFETY: the caller's contract (the lock, the callbacks).
            unsafe { s.runlock() }
        })
    }

    /// `git_sortedcache_lockandload`: 1 (file changed, loaded into `buf`,
    /// write lock held), 0 (unchanged, unlocked) or an error (unlocked).
    ///
    /// # Safety
    ///
    /// `sc` is a live cache not locked by this thread; `buf` is NULL or a
    /// valid `git_str`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_lockandload(
        sc: *const Shared,
        buf: *mut GitStr,
    ) -> c_int {
        barrier(|| {
            // SAFETY: NULL or a valid `git_str` in exclusive use.
            let buf = unsafe { buf.as_mut() };
            // SAFETY: a live cache.
            let s = unsafe { shared(sc) };
            // SAFETY: the comparison is the caller's, sound on the items.
            match unsafe { s.lockandload(buf) } {
                Ok(true) => 1,
                Ok(false) => 0,
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_sortedcache_updated`.
    ///
    /// # Safety
    ///
    /// `sc` is a live cache write-locked by this thread.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_updated(sc: *const Shared) {
        barrier(|| {
            // SAFETY: a live cache.
            let s = unsafe { shared(sc) };
            // SAFETY: the caller's contract (the lock, the callbacks).
            unsafe { s.updated() }
        })
    }

    /// `git_sortedcache_clear`.
    ///
    /// # Safety
    ///
    /// `sc` is a live cache; write-locked by this thread unless `wlock`
    /// (then not locked by it).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_clear(sc: *const Shared, wlock: bool) -> c_int {
        barrier(|| {
            // SAFETY: a live cache.
            let s = unsafe { shared(sc) };
            if wlock && s.wlock().is_err() {
                return -1;
            }
            // SAFETY: the write lock is held (taken here or by the caller).
            unsafe { s.clear_items() };
            if wlock {
                // SAFETY: taken above.
                unsafe { s.wunlock() };
            }
            0
        })
    }

    /// `git_sortedcache_upsert`.
    ///
    /// # Safety
    ///
    /// `sc` is a live cache write-locked by this thread; `out` is NULL or
    /// writable; `key` a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_upsert(
        out: *mut *mut c_void,
        sc: *const Shared,
        key: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: a NUL-terminated string.
            let key = unsafe { cstr_bytes(key) };
            // SAFETY: a live cache.
            let s = unsafe { shared(sc) };
            // SAFETY: the write lock is held.
            let r = unsafe { s.upsert(key) };
            let (item, code) = match r {
                Ok((item, _)) => (item.as_ptr().cast(), 0),
                Err(e) => (core::ptr::null_mut(), e.raw()),
            };
            // SAFETY: NULL or writable.
            unsafe { write_opt(out, item) };
            code
        })
    }

    /// `git_sortedcache_remove`.
    ///
    /// # Safety
    ///
    /// `sc` is a live cache write-locked by this thread.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_remove(sc: *const Shared, pos: usize) -> c_int {
        barrier(|| {
            // SAFETY: a live cache.
            let s = unsafe { shared(sc) };
            // SAFETY: the caller's contract (the write lock, the callback).
            crate::util::export::status(unsafe { s.remove(pos) })
        })
    }

    /// `git_sortedcache_lookup`: the item, or NULL.
    ///
    /// # Safety
    ///
    /// `sc` is a live cache locked by this thread; `key` a NUL-terminated
    /// string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_lookup(
        sc: *const Shared,
        key: *const c_char,
    ) -> *mut c_void {
        barrier(|| {
            // SAFETY: a live cache.
            let s = unsafe { shared(sc) };
            // SAFETY: a NUL-terminated string.
            let key = unsafe { cstr_bytes(key) };
            // SAFETY: a lock is held.
            match unsafe { s.core() }.lookup(key) {
                Some(p) => p.as_ptr().cast(),
                None => core::ptr::null_mut(),
            }
        })
    }

    /// `git_sortedcache_entrycount` (no lock needed).
    ///
    /// # Safety
    ///
    /// `sc` is a live cache.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_entrycount(sc: *const Shared) -> usize {
        // SAFETY: a live cache.
        barrier(|| unsafe { shared(sc) }.entrycount())
    }

    /// `git_sortedcache_entry`: the item at `pos` in sorted order, or NULL.
    ///
    /// # Safety
    ///
    /// `sc` is a live cache locked by this thread (write-locked, or in
    /// exclusive use, if items were upserted since the last unlock).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_entry(sc: *const Shared, pos: usize) -> *mut c_void {
        barrier(|| {
            // SAFETY: a live cache.
            let s = unsafe { shared(sc) };
            // SAFETY: the caller's contract (the lock, the callbacks).
            unsafe { s.entry(pos) }.unwrap_or(core::ptr::null_mut())
        })
    }

    /// `git_sortedcache_lookup_index`: 0 and the position, or
    /// `GIT_ENOTFOUND` (without a message) and where it would go; -1 if the
    /// cache has no comparison function.
    ///
    /// # Safety
    ///
    /// As [`git_sortedcache_entry`]; `out` is writable, `key` a
    /// NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_sortedcache_lookup_index(
        out: *mut usize,
        sc: *const Shared,
        key: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: a live cache.
            let s = unsafe { shared(sc) };
            // SAFETY: a NUL-terminated string.
            let key = unsafe { cstr_bytes(key) };
            // SAFETY: the caller's contract.
            let (r, pos) = unsafe { s.lookup_index(key) };
            if r != Err(ErrorCode::GENERIC) {
                // SAFETY: writable (`git__bsearch` writes it either way).
                unsafe { write_opt(out, pos) };
            }
            crate::util::export::status(r)
        })
    }
}
