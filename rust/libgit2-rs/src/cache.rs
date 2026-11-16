//! Port of `src/libgit2/cache.c`: `git_cache`, the object cache of a
//! repository (`repo->objects`) and of an odb without one.
//!
//! # Types
//!
//! - [`GitCachedObj`] is `git_cached_obj`, mirrored (kind 3): it is the
//!   header of `git_odb_object` (this WP) and of `git_object` (l-obj), which
//!   C embeds by value, sizes and reads (`obj->cached.type`, `.oid`, `.size`).
//! - [`GitCache`] is `git_cache`, mirrored (kind 3): `git_repository`
//!   embeds it by value (`repo->objects`), and C passes `&repo->objects` to
//!   the exports. Its map (`git_cache_oidmap`, keys pointing into the
//!   entries' own oids) is reached only under its rwlock, as in C; the
//!   memory it accounts (`used_memory`) is an atomic `ssize_t`.
//!
//! The globals `git_cache__enabled`, `git_cache__max_storage` and
//! `git_cache__current_storage` are C's (settings.c and the tests read and
//! write them), with interior mutability. The eviction policy, the limits
//! per type and the store/replace rules are the C's.

use core::cell::UnsafeCell;
use core::ffi::{c_int, c_void};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};

use crate::hashmap_oid::OidPtr;
use crate::oid::Oid;
use crate::util::alloc;
use crate::util::errors::{ErrorClass, Result};
use crate::util::hashmap::{GitHashMap, HashMapIter};
use crate::util::thread::{Atomic32, GitRwLock};

/// `GIT_CACHE_STORE_ANY`.
pub const GIT_CACHE_STORE_ANY: u16 = 0;
/// `GIT_CACHE_STORE_RAW`: a `git_odb_object`.
pub const GIT_CACHE_STORE_RAW: u16 = 1;
/// `GIT_CACHE_STORE_PARSED`: a `git_object`.
pub const GIT_CACHE_STORE_PARSED: u16 = 2;

/// `git_cached_obj`, mirrored: the header of every cached object.
///
/// `type`, `flags` and `size` are set while the object is private to its
/// maker; once it is shared, only `refcount` changes.
#[repr(C)]
#[derive(Debug)]
pub struct GitCachedObj {
    /// The object's id.
    pub oid: Oid,
    /// Its `git_object_t`.
    pub type_: i16,
    /// `GIT_CACHE_STORE_*`.
    pub flags: u16,
    /// Its size (what the cache accounts).
    pub size: usize,
    /// The count of references.
    pub refcount: Atomic32,
}

/// `git_cache_oidmap` (`GIT_HASHMAP_OID_STRUCT(git_cache_oidmap,
/// git_cached_obj *)`): keys point at the entries' own oids.
pub type CacheMap = GitHashMap<OidPtr, *mut GitCachedObj>;

/// `git_cache`, mirrored.
///
/// Invariant: `map` is read under `lock` (read or write) and written under
/// its write lock only; every value is a live cached object to which the
/// map holds one reference, keyed by a pointer to its own `oid`.
#[repr(C)]
pub struct GitCache {
    map: UnsafeCell<CacheMap>,
    lock: GitRwLock,
    used_memory: AtomicIsize,
}

// SAFETY: the map is reached under the rwlock only (the invariant); the
// entries are refcounted with atomics and freed by whichever thread drops
// the last reference, as in C.
unsafe impl Sync for GitCache {}
// SAFETY: as above; nothing in it is tied to a thread.
unsafe impl Send for GitCache {}

impl core::fmt::Debug for GitCache {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GitCache")
            .field("size", &self.size())
            .field("used_memory", &self.used_memory())
            .finish_non_exhaustive()
    }
}

/// `git_cache__enabled`.
#[unsafe(no_mangle)]
pub static git_cache__enabled: AtomicBool = AtomicBool::new(true);

/// `git_cache__max_storage` (256 MiB).
#[unsafe(no_mangle)]
pub static git_cache__max_storage: AtomicIsize = AtomicIsize::new(256 * 1024 * 1024);

/// `git_cache__current_storage` (`git_atomic_ssize`): the memory all caches
/// account.
#[unsafe(no_mangle)]
pub static git_cache__current_storage: AtomicIsize = AtomicIsize::new(0);

/// `git_cache__max_object_size`: objects of a type are cached when smaller.
static MAX_OBJECT_SIZE: [AtomicUsize; 8] = [
    AtomicUsize::new(0),    // GIT_OBJECT__EXT1
    AtomicUsize::new(4096), // GIT_OBJECT_COMMIT
    AtomicUsize::new(4096), // GIT_OBJECT_TREE
    AtomicUsize::new(0),    // GIT_OBJECT_BLOB
    AtomicUsize::new(4096), // GIT_OBJECT_TAG
    AtomicUsize::new(0),    // GIT_OBJECT__EXT2
    AtomicUsize::new(0),    // GIT_OBJECT_OFS_DELTA
    AtomicUsize::new(0),    // GIT_OBJECT_REF_DELTA
];

/// Whether caching is enabled (`git_cache__enabled`).
pub fn enabled() -> bool {
    git_cache__enabled.load(Ordering::Relaxed)
}

/// Enable or disable caching (`GIT_OPT_ENABLE_CACHING`).
pub fn set_enabled(on: bool) {
    git_cache__enabled.store(on, Ordering::Relaxed);
}

/// The memory all caches account (`GIT_OPT_GET_CACHED_MEMORY`'s first value).
pub fn current_storage() -> isize {
    git_cache__current_storage.load(Ordering::SeqCst)
}

/// The limit above which a store evicts (`git_cache__max_storage`).
pub fn max_storage() -> isize {
    git_cache__max_storage.load(Ordering::Relaxed)
}

/// Set the limit (`GIT_OPT_SET_CACHE_MAX_SIZE`).
pub fn set_max_storage(max: isize) {
    git_cache__max_storage.store(max, Ordering::Relaxed);
}

/// `git_cache_set_max_object_size` (`GIT_OPT_SET_CACHE_OBJECT_LIMIT`).
pub fn set_max_object_size(type_: c_int, size: usize) -> Result<()> {
    match usize::try_from(type_).ok().and_then(|t| MAX_OBJECT_SIZE.get(t)) {
        Some(slot) => {
            slot.store(size, Ordering::Relaxed);
            Ok(())
        }
        None => Err(crate::git_error_set!(
            ErrorClass::INVALID,
            "type out of range"
        )),
    }
}

/// The limit for objects of `type_`. C indexes its array with any type the
/// object has (out of bounds for a type outside 0..8: rust/DEFECTS.md);
/// such objects are not cached.
fn max_object_size(type_: i16) -> usize {
    usize::try_from(type_)
        .ok()
        .and_then(|t| MAX_OBJECT_SIZE.get(t))
        .map_or(0, |s| s.load(Ordering::Relaxed))
}

/// `cache_should_store`.
fn should_store(type_: i16, size: usize) -> bool {
    enabled() && size < max_object_size(type_)
}

// ---------------------------------------------------------------------------
// References to cached objects
// ---------------------------------------------------------------------------

/// `git_cached_obj_incref`.
///
/// # Safety
///
/// `obj` is a live cached object to which the caller holds a reference.
#[inline]
pub unsafe fn incref(obj: NonNull<GitCachedObj>) {
    // SAFETY: the caller's contract; the count is atomic.
    unsafe { obj.as_ref() }.refcount.inc();
}

/// `git_cached_obj_decref`: drop a reference; the last one frees the
/// object as its `flags` say (an odb object, a parsed object, or plain
/// memory).
///
/// # Safety
///
/// `obj` is a live cached object, and the caller gives up one reference
/// to it; an object of `flags` 0 was allocated with `git__allocator`.
pub unsafe fn decref(obj: NonNull<GitCachedObj>) {
    // SAFETY: the caller's contract.
    if unsafe { obj.as_ref() }.refcount.dec() != 0 {
        return;
    }
    // SAFETY: this was the last reference; nothing else reads the object.
    let flags = unsafe { (*obj.as_ptr()).flags };
    match flags {
        // SAFETY: a raw object is a `git_odb_object` (this crate's).
        GIT_CACHE_STORE_RAW => unsafe { crate::odb::odb_object_free(obj.cast()) },
        // SAFETY: a parsed object is a `git_object`, freed by its module.
        GIT_CACHE_STORE_PARSED => unsafe { crate::sys::pending::object_free(obj.cast()) },
        // SAFETY: the caller's contract.
        _ => unsafe { alloc::free(obj.as_ptr().cast()) },
    }
}

// ---------------------------------------------------------------------------
// The cache
// ---------------------------------------------------------------------------

impl Default for GitCache {
    fn default() -> Self {
        Self::new()
    }
}

impl GitCache {
    /// `git_cache_init`'s result: empty, unlocked.
    pub const fn new() -> GitCache {
        GitCache {
            map: UnsafeCell::new(GitHashMap::new()),
            lock: GitRwLock::new(),
            used_memory: AtomicIsize::new(0),
        }
    }

    /// The memory this cache accounts.
    pub fn used_memory(&self) -> isize {
        self.used_memory.load(Ordering::SeqCst)
    }

    /// `git_cache_size`: the number of entries (under the read lock).
    pub fn size(&self) -> usize {
        let _g = self.lock.read();
        // SAFETY: read under the read lock (the invariant).
        unsafe { (*self.map.get()).len() }
    }

    /// The map, under the write lock held by the caller.
    ///
    /// # Safety
    ///
    /// The caller holds `self.lock`'s write lock for the reference's life.
    #[allow(clippy::mut_from_ref)]
    unsafe fn map_mut(&self) -> &mut CacheMap {
        // SAFETY: the caller's contract excludes every other access.
        unsafe { &mut *self.map.get() }
    }

    /// `clear_cache`: drop every entry. Called with the write lock.
    ///
    /// # Safety
    ///
    /// As [`map_mut`](Self::map_mut).
    unsafe fn clear_locked(&self) {
        // SAFETY: the caller's contract.
        let map = unsafe { self.map_mut() };
        if map.is_empty() {
            return;
        }
        for (_, &v) in map.iter() {
            if let Some(v) = NonNull::new(v) {
                // SAFETY: the map holds a reference to each entry.
                unsafe { decref(v) };
            }
        }
        // The keys point into freed entries now: forget them without
        // reading them.
        map.clear();
        let used = self.used_memory.swap(0, Ordering::SeqCst);
        git_cache__current_storage.fetch_sub(used, Ordering::SeqCst);
    }

    /// `git_cache_clear`.
    pub fn clear(&self) {
        let _g = self.lock.write();
        // SAFETY: the write lock is held.
        unsafe { self.clear_locked() };
    }

    /// `git_cache_dispose`, but for the `memset`: clear, free the map,
    /// destroy the lock. The cache is not used again (but re-initialized).
    pub fn dispose(&mut self) {
        self.clear();
        self.map.get_mut().dispose();
        // SAFETY: `&mut self`: nothing holds the lock, and the caller does
        // not use it again (it is re-made below, as C's memset zeroes it).
        unsafe { self.lock.destroy() };
        self.lock = GitRwLock::new();
        self.used_memory.store(0, Ordering::SeqCst);
    }

    /// `cache_evict_entries`: drop `max(size / 2048, 8)` entries in bucket
    /// order (or all, if there are fewer). Called with the write lock.
    ///
    /// # Safety
    ///
    /// As [`map_mut`](Self::map_mut).
    unsafe fn evict_locked(&self) {
        // SAFETY: the caller's contract.
        let map = unsafe { self.map_mut() };
        let size = map.len();
        let mut evict_count = (size / 2048).max(8);
        if evict_count > size {
            // SAFETY: the caller's contract.
            unsafe { self.clear_locked() };
            return;
        }
        let mut evicted: isize = 0;
        let mut iter: HashMapIter = 0;
        while evict_count > 0 {
            let Some((&key, &val)) = map.iterate(&mut iter) else {
                break;
            };
            evict_count -= 1;
            // SAFETY: the map holds a reference to the entry, alive here.
            evicted = evicted.wrapping_add(unsafe { (*val).size } as isize);
            let _ = map.remove(&key);
            if let Some(v) = NonNull::new(val) {
                // SAFETY: the map's reference, given up by the removal.
                unsafe { decref(v) };
            }
        }
        self.used_memory.fetch_sub(evicted, Ordering::SeqCst);
        git_cache__current_storage.fetch_sub(evicted, Ordering::SeqCst);
    }

    /// `cache_get`: a new reference to the entry for `oid`, if it is cached
    /// with `flags` (any, for 0).
    pub fn get(&self, oid: &Oid, flags: u16) -> Option<NonNull<GitCachedObj>> {
        if !enabled() {
            return None;
        }
        let _g = self.lock.read();
        // SAFETY: read under the read lock.
        let map = unsafe { &*self.map.get() };
        let key = OidPtr::from_ref(oid);
        let entry = NonNull::new(*map.get(&key)?)?;
        // SAFETY: the map holds a reference: the entry is live while the
        // lock is held; its `flags` are not written once it is shared.
        if flags != 0 && unsafe { (*entry.as_ptr()).flags } != flags {
            return None;
        }
        // SAFETY: as above; the new reference is the caller's.
        unsafe { incref(entry) };
        Some(entry)
    }

    /// `cache_store`: store `entry` (whose `flags` the caller set), and
    /// return the reference the caller now holds: to `entry`, or to the
    /// entry already cached for its id, which replaces it.
    ///
    /// # Safety
    ///
    /// `entry` is a live cached object of the kind its `flags` say, private
    /// to the caller, who gives up one reference to it.
    pub unsafe fn store(&self, entry: NonNull<GitCachedObj>) -> NonNull<GitCachedObj> {
        // SAFETY: the caller's contract.
        unsafe { incref(entry) };

        if !enabled() && self.used_memory() > 0 {
            self.clear();
            return entry;
        }

        // SAFETY: private to the caller: read without a lock.
        let (type_, size, flags) = unsafe {
            let e = entry.as_ptr();
            ((*e).type_, (*e).size, (*e).flags)
        };
        if !should_store(type_, size) {
            return entry;
        }

        let _g = self.lock.write();
        // SAFETY: the write lock is held for both calls.
        let map = unsafe {
            if git_cache__current_storage.load(Ordering::SeqCst) > max_storage() {
                self.evict_locked();
            }
            self.map_mut()
        };

        // SAFETY: the entry's oid lives as long as the entry, which the map
        // keeps a reference to while the key is in it.
        let key = unsafe { OidPtr::new(NonNull::from(&(*entry.as_ptr()).oid)) };
        let mut result = entry;
        match map.get(&key).copied().and_then(NonNull::new) {
            None => {
                if map.put(key, entry.as_ptr()).is_ok() {
                    // SAFETY: the map's new reference.
                    unsafe { incref(entry) };
                    self.used_memory.fetch_add(size as isize, Ordering::SeqCst);
                    git_cache__current_storage.fetch_add(size as isize, Ordering::SeqCst);
                }
            }
            Some(stored) => {
                // SAFETY: the map holds a reference to `stored`.
                let stored_flags = unsafe { (*stored.as_ptr()).flags };
                if stored_flags == flags {
                    // SAFETY: the caller's two references to `entry` (its
                    // own, and the one taken above) and the map's to
                    // `stored`.
                    unsafe {
                        decref(entry);
                        incref(stored);
                    }
                    result = stored;
                } else if stored_flags == GIT_CACHE_STORE_RAW && flags == GIT_CACHE_STORE_PARSED {
                    if map.put(key, entry.as_ptr()).is_ok() {
                        // SAFETY: the map gives up `stored`, and takes
                        // `entry`.
                        unsafe {
                            decref(stored);
                            incref(entry);
                        }
                    } else {
                        // SAFETY: as in the first branch.
                        unsafe {
                            decref(entry);
                            incref(stored);
                        }
                        result = stored;
                    }
                }
            }
        }
        result
    }

    /// The cached entries' ids (under the read lock), for tests and the
    /// safe API.
    pub fn ids(&self) -> Vec<Oid> {
        let _g = self.lock.read();
        // SAFETY: read under the read lock.
        let map = unsafe { &*self.map.get() };
        map.keys().map(|k| *k.get()).collect()
    }
}

impl OidPtr {
    /// A key for a lookup: valid while `oid` is borrowed.
    #[inline]
    pub(crate) fn from_ref(oid: &Oid) -> OidPtr {
        // SAFETY: the key is used for one lookup while `oid` is borrowed,
        // and is not stored.
        unsafe { OidPtr::new(NonNull::from(oid)) }
    }
}

/// The C ABI of `cache.c`.
pub mod ffi {
    use super::*;
    use crate::util::export::barrier;

    /// The cache at `cache`.
    ///
    /// # Safety
    ///
    /// `cache` points to a cache initialized by `git_cache_init` and not
    /// disposed, alive for `'a`.
    unsafe fn cache<'a>(cache: *mut GitCache) -> &'a GitCache {
        // SAFETY: the caller's contract; every field C does not reach
        // through these exports is interior-mutable.
        unsafe { &*cache }
    }

    /// `git_cache_set_max_object_size`.
    ///
    /// # Safety
    ///
    /// None: any arguments.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_cache_set_max_object_size(type_: c_int, size: usize) -> c_int {
        barrier(|| crate::util::export::status(set_max_object_size(type_, size)))
    }

    /// `git_cache_init`.
    ///
    /// # Safety
    ///
    /// `cache` is writable for a `git_cache` (its contents are ignored) and
    /// not in use.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_cache_init(cache: *mut GitCache) -> c_int {
        // SAFETY: the caller's contract; nothing is read.
        unsafe { cache.write(GitCache::new()) };
        0
    }

    /// `git_cache_dispose`.
    ///
    /// # Safety
    ///
    /// `cache` was initialized by `git_cache_init` (or zeroed after a
    /// dispose) and nothing else uses it during or after the call (unless
    /// re-initialized).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_cache_dispose(cache: *mut GitCache) {
        barrier(|| {
            // SAFETY: the caller's contract: exclusive.
            let c = unsafe { &mut *cache };
            c.dispose();
            // `git__memzero`: the map is empty and has no arrays, the lock
            // destroyed; zero the bytes, as C.
            // SAFETY: the reference has ended; the cache's bytes are
            // writable and its zero pattern is valid.
            unsafe { core::ptr::write_bytes(cache.cast::<u8>(), 0, size_of::<GitCache>()) };
        })
    }

    /// `git_cache_clear`.
    ///
    /// # Safety
    ///
    /// As [`cache`]; other threads may use the cache meanwhile.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_cache_clear(c: *mut GitCache) {
        // SAFETY: the caller's contract.
        barrier(|| unsafe { cache(c) }.clear())
    }

    /// `git_cache_size`. C read the map's size without the lock; this
    /// takes the read lock (rust/DIVERGENCES.md).
    ///
    /// # Safety
    ///
    /// As [`git_cache_clear`]; the calling thread does not hold the
    /// cache's write lock.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_cache_size(c: *mut GitCache) -> usize {
        // SAFETY: the caller's contract.
        barrier(|| unsafe { cache(c) }.size())
    }

    /// `git_cache_store_raw`.
    ///
    /// # Safety
    ///
    /// As [`git_cache_clear`]; `entry` is a live `git_odb_object` private
    /// to the caller, who gives up one reference to it.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_cache_store_raw(c: *mut GitCache, entry: *mut c_void) -> *mut c_void {
        barrier(|| {
            let Some(e) = NonNull::new(entry.cast::<GitCachedObj>()) else {
                return core::ptr::null_mut();
            };
            // SAFETY: private to the caller.
            unsafe { (*e.as_ptr()).flags = GIT_CACHE_STORE_RAW };
            // SAFETY: the caller's contract.
            unsafe { cache(c).store(e) }.as_ptr().cast()
        })
    }

    /// `git_cache_store_parsed`.
    ///
    /// # Safety
    ///
    /// As [`git_cache_store_raw`], for a `git_object`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_cache_store_parsed(c: *mut GitCache, entry: *mut c_void) -> *mut c_void {
        barrier(|| {
            let Some(e) = NonNull::new(entry.cast::<GitCachedObj>()) else {
                return core::ptr::null_mut();
            };
            // SAFETY: private to the caller.
            unsafe { (*e.as_ptr()).flags = GIT_CACHE_STORE_PARSED };
            // SAFETY: the caller's contract.
            unsafe { cache(c).store(e) }.as_ptr().cast()
        })
    }

    fn get(c: *mut GitCache, oid: *const Oid, flags: u16) -> *mut c_void {
        barrier(|| {
            // SAFETY: the exports' contract: a live cache and oid.
            let (c, oid) = unsafe { (cache(c), &*oid) };
            c.get(oid, flags).map_or(core::ptr::null_mut(), |e| e.as_ptr().cast())
        })
    }

    /// `git_cache_get_raw`.
    ///
    /// # Safety
    ///
    /// As [`git_cache_clear`]; `oid` is a readable `git_oid`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_cache_get_raw(c: *mut GitCache, oid: *const Oid) -> *mut c_void {
        get(c, oid, GIT_CACHE_STORE_RAW)
    }

    /// `git_cache_get_parsed`.
    ///
    /// # Safety
    ///
    /// As [`git_cache_get_raw`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_cache_get_parsed(c: *mut GitCache, oid: *const Oid) -> *mut c_void {
        get(c, oid, GIT_CACHE_STORE_PARSED)
    }

    /// `git_cache_get_any`.
    ///
    /// # Safety
    ///
    /// As [`git_cache_get_raw`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_cache_get_any(c: *mut GitCache, oid: *const Oid) -> *mut c_void {
        get(c, oid, GIT_CACHE_STORE_ANY)
    }

    /// `git_cached_obj_decref`.
    ///
    /// # Safety
    ///
    /// As [`decref`]: `obj` is a live cached object whose reference the
    /// caller gives up.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_cached_obj_decref(obj: *mut c_void) {
        barrier(|| {
            if let Some(o) = NonNull::new(obj.cast::<GitCachedObj>()) {
                // SAFETY: the caller's contract.
                unsafe { decref(o) }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oid::OidType;

    fn plain_obj(i: u32, type_: i16, size: usize) -> NonNull<GitCachedObj> {
        let p = alloc::calloc(1, size_of::<GitCachedObj>()).cast::<GitCachedObj>();
        let p = NonNull::new(p).expect("alloc");
        let mut id = [0u8; 32];
        id[..4].copy_from_slice(&i.to_be_bytes());
        // SAFETY: a fresh zeroed allocation of the right size.
        unsafe {
            p.as_ptr().write(GitCachedObj {
                oid: Oid::from_raw_parts(OidType::Sha1 as u8, id),
                type_,
                flags: 0,
                size,
                refcount: Atomic32::new(1),
            })
        };
        p
    }

    #[test]
    fn store_get_and_evict() {
        let _g = crate::util::test_support::init();
        let mut cache = GitCache::new();
        let before = current_storage();
        let mut held = Vec::new();
        for i in 0..20u32 {
            let e = plain_obj(i, 1, 100);
            // SAFETY: fresh, private.
            let r = unsafe { cache.store(e) };
            assert_eq!(r, e);
            held.push(r);
        }
        assert_eq!(cache.size(), 20);
        assert_eq!(cache.used_memory(), 2000);
        let id = // SAFETY: held.
            unsafe { (*held[3].as_ptr()).oid };
        let got = cache.get(&id, 0).expect("cached");
        assert_eq!(got, held[3]);
        assert!(cache.get(&id, GIT_CACHE_STORE_RAW).is_none());
        // SAFETY: our reference from `get`.
        unsafe { decref(got) };
        // Too big (blob limit is 0): not stored.
        let big = plain_obj(99, 3, 10);
        // SAFETY: fresh, private.
        let r = unsafe { cache.store(big) };
        assert_eq!(r, big);
        assert_eq!(cache.size(), 20);
        // SAFETY: the reference `store` returned.
        unsafe { decref(r) };
        for h in held {
            // SAFETY: our references.
            unsafe { decref(h) };
        }
        cache.dispose();
        assert_eq!(current_storage(), before);
    }

    #[test]
    fn set_max_object_size_range() {
        let _g = crate::util::test_support::init();
        assert!(set_max_object_size(-1, 1).is_err());
        assert!(set_max_object_size(8, 1).is_err());
        assert_eq!(max_object_size(9), 0);
        assert_eq!(max_object_size(-2), 0);
        assert_eq!(max_object_size(1), 4096);
    }
}

crate::c_layout_test! {
    headers: ["cache.h", "repository.h"],
    types: [
        GitCachedObj => "git_cached_obj" { oid, type_ as "type", flags, size, refcount },
        GitCache => "git_cache" { map, lock, used_memory },
    ],
    asserts: [
        "GIT_CACHE_STORE_ANY == 0",
        "GIT_CACHE_STORE_RAW == 1",
        "GIT_CACHE_STORE_PARSED == 2",
        "sizeof(git_cache__current_storage) == 8",
        "offsetof(git_repository, objects) == 32",
    ],
}
