//! A Rust equivalent of `src/util/hashmap.h`: libgit2's open-addressing hash
//! map and set (a variant of khash), macro-instantiated in C.
//!
//! [`GitHashMap<K, V, X>`] has the layout of the C struct that
//! `GIT_HASHMAP_STRUCT(name, key_t, val_t)` declares (with `X = ()`), or of
//! a struct that puts a field before the members (`X = u32`: `git_index_entrymap`'s
//! `ignore_case`), and runs the C's algorithm exactly: the same hash
//! functions, buckets, quadratic probing, deletion marks, growth (at 0.77
//! load) and in-place rehash. So:
//!
//! - iteration order is the C's for the same sequence of operations (it is
//!   bucket order);
//! - a map can be embedded in a struct that C also sees, and C's inline
//!   functions and the Rust can work on the same map: its arrays come from
//!   `git__allocator`.
//!
//! A set is a map whose value is `()` ([`GitHashSet<K>`]): its `vals` stays
//! NULL, as the C set's.
//!
//! Keys implement [`HashKey`] (the C's `__hash_fn` and `__equal_fn`):
//! [`CStrKey`] (`hashmap_str.h`: `const char *` keys, X31 hash), `&Oid` and
//! `Oid` (`hashmap_oid.h`, in [`crate::hashmap_oid`]), `i64`
//! (`pack.c`'s `off64_hash`). A map whose hash depends on runtime state
//! uses the `_with` methods, which take the two functions.
//!
//! Keys and values may be any type; the map drops what it holds when it is
//! cleared, disposed or dropped. A map that C also touches holds `Copy`
//! values (pointers), as C's.
//!
//! ```
//! use libgit2_rs::__test::util::hashmap::{CStrKey, GitHashMap};
//! # let _g = libgit2_rs::__test::util::test_support::init();
//! let mut m: GitHashMap<CStrKey<'_>, u32> = GitHashMap::new();
//! m.put(CStrKey::new(c"one"), 1)?;
//! m.put(CStrKey::new(c"two"), 2)?;
//! assert_eq!(m.get(&CStrKey::new(c"two")), Some(&2));
//! assert_eq!(m.len(), 2);
//! let mut it = 0;
//! while let Some((k, v)) = m.iterate(&mut it) { /* bucket order, as C */ let _ = (k, v); }
//! # Ok::<(), libgit2_rs::__test::util::errors::ErrorCode>(())
//! ```

use core::ffi::{CStr, c_char, c_void};
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr::{self, NonNull};

use super::alloc;
use super::errors::{ErrorCode, Result};

/// `git_hashmap_iter_t`: a bucket index; start at 0
/// (`GIT_HASHMAP_ITER_INIT`).
pub type HashMapIter = u32;

/// A key type: the C's `__hash_fn` and `__equal_fn`.
pub trait HashKey {
    /// The hash.
    fn hash_code(&self) -> u32;
    /// Whether two keys are equal.
    fn key_eq(&self, other: &Self) -> bool;
}

/// `git_hashmap__upper_bound`.
const UPPER_BOUND: f64 = 0.77;

#[inline(always)]
fn flagsize(m: u32) -> usize {
    if m < 16 { 1 } else { (m >> 4) as usize }
}

#[inline(always)]
fn roundup(mut x: u32) -> u32 {
    x = x.wrapping_sub(1);
    x |= x >> 1;
    x |= x >> 2;
    x |= x >> 4;
    x |= x >> 8;
    x |= x >> 16;
    x.wrapping_add(1)
}

// The bucket flags: two bits per bucket, bit 1 "empty", bit 0 "deleted"
// (C's `GIT_HASHMAP_IS_*` and `GIT_HASHMAP_SET_*` macros).

#[inline(always)]
fn bits(f: &[u32], i: u32) -> u32 {
    (f[(i >> 4) as usize] >> ((i & 0xf) << 1)) & 3
}
#[inline(always)]
fn is_empty(f: &[u32], i: u32) -> bool {
    bits(f, i) & 2 != 0
}
#[inline(always)]
fn is_del(f: &[u32], i: u32) -> bool {
    bits(f, i) & 1 != 0
}
#[inline(always)]
fn is_either(f: &[u32], i: u32) -> bool {
    bits(f, i) != 0
}
#[inline(always)]
fn set_empty_false(f: &mut [u32], i: u32) {
    f[(i >> 4) as usize] &= !(2 << ((i & 0xf) << 1));
}
#[inline(always)]
fn set_both_false(f: &mut [u32], i: u32) {
    f[(i >> 4) as usize] &= !(3 << ((i & 0xf) << 1));
}
#[inline(always)]
fn set_del_true(f: &mut [u32], i: u32) {
    f[(i >> 4) as usize] |= 1 << ((i & 0xf) << 1);
}

/// A map's arrays as slices: flags, keys and values (a value slot of a
/// zero-sized `V` is nothing).
struct Parts<'a, K, V> {
    flags: &'a mut [u32],
    keys: &'a mut [MaybeUninit<K>],
    vals: &'a mut [MaybeUninit<V>],
}

/// [`Parts`], shared.
struct View<'a, K, V> {
    flags: &'a [u32],
    keys: &'a [MaybeUninit<K>],
    vals: &'a [MaybeUninit<V>],
}

/// `n` slots at `p`, or none if `n` is 0.
///
/// # Safety
///
/// When `n > 0` and `T` is not zero-sized, `p` is valid for `n` `T`s for
/// `'a`, and nothing else accesses them meanwhile.
#[inline(always)]
unsafe fn slots_mut<'a, T>(p: *mut T, n: usize) -> &'a mut [MaybeUninit<T>] {
    if n == 0 {
        &mut []
    } else if size_of::<T>() == 0 {
        // SAFETY: zero-sized elements need no memory.
        unsafe { core::slice::from_raw_parts_mut(NonNull::dangling().as_ptr(), n) }
    } else {
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts_mut(p.cast(), n) }
    }
}

/// As [`slots_mut`], shared.
///
/// # Safety
///
/// As [`slots_mut`], with only reads meanwhile.
#[inline(always)]
unsafe fn slots<'a, T>(p: *const T, n: usize) -> &'a [MaybeUninit<T>] {
    if n == 0 {
        &[]
    } else if size_of::<T>() == 0 {
        // SAFETY: zero-sized elements need no memory.
        unsafe { core::slice::from_raw_parts(NonNull::dangling().as_ptr(), n) }
    } else {
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(p.cast(), n) }
    }
}

/// `GIT_HASHMAP_STRUCT(name, K, V)`, with an optional field `X` before the
/// members.
///
/// Invariant: `n_buckets` is 0 (`flags` NULL; `keys`/`vals` NULL, or arrays
/// left by a grow that failed halfway, which the next grow reallocates and
/// `dispose` frees) or a power of two
/// ≥ 4 with `flags` an array of `flagsize(n_buckets)` words, and `keys` (and
/// `vals`, unless `V` is zero-sized, when it stays NULL) arrays of at least
/// `n_buckets` slots, all from `git__allocator` and owned by the map; a
/// bucket whose flags are both clear ("live") holds an initialized key and
/// value; `size` counts the live buckets, `n_occupied` the live and deleted
/// ones.
#[repr(C)]
pub struct GitHashMap<K, V, X = ()> {
    /// The field C puts before the members (`()` for none).
    pub extra: X,
    n_buckets: u32,
    size: u32,
    n_occupied: u32,
    upper_bound: u32,
    flags: *mut u32,
    keys: *mut K,
    vals: *mut V,
    _own: PhantomData<(K, V)>,
}

/// `GIT_HASHSET_STRUCT(name, K)`.
pub type GitHashSet<K, X = ()> = GitHashMap<K, (), X>;

// SAFETY: the map owns its keys and values and its arrays, which
// `git__allocator` frees from any thread.
unsafe impl<K: Send, V: Send, X: Send> Send for GitHashMap<K, V, X> {}
// SAFETY: shared access only reads.
unsafe impl<K: Sync, V: Sync, X: Sync> Sync for GitHashMap<K, V, X> {}

impl<K, V, X: Default> Default for GitHashMap<K, V, X> {
    fn default() -> Self {
        Self::with_extra(X::default())
    }
}

impl<K, V, X> Drop for GitHashMap<K, V, X> {
    fn drop(&mut self) {
        self.dispose();
    }
}

impl<K: core::fmt::Debug, V: core::fmt::Debug, X> core::fmt::Debug for GitHashMap<K, V, X> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K, V> GitHashMap<K, V, ()> {
    /// An empty map (`GIT_HASHMAP_INIT`): no arrays.
    pub const fn new() -> Self {
        Self::with_extra(())
    }
}

impl<K, V, X> GitHashMap<K, V, X> {
    const CHECK: () = assert!(
        align_of::<K>() <= 8 && align_of::<V>() <= 8 && size_of::<K>() > 0,
        "hash map of a zero-sized or over-aligned key, or over-aligned value"
    );

    const IS_MAP: bool = size_of::<V>() > 0;

    /// An empty map with the leading field `extra`.
    pub const fn with_extra(extra: X) -> Self {
        GitHashMap {
            extra,
            n_buckets: 0,
            size: 0,
            n_occupied: 0,
            upper_bound: 0,
            flags: ptr::null_mut(),
            keys: ptr::null_mut(),
            vals: ptr::null_mut(),
            _own: PhantomData,
        }
    }

    /// `name##_size`: the number of entries.
    #[inline]
    pub fn len(&self) -> usize {
        self.size as usize
    }

    /// Whether the map has no entries.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// The number of buckets.
    #[inline]
    pub fn buckets(&self) -> u32 {
        self.n_buckets
    }

    /// The number of buckets in use or marked deleted (`n_occupied`).
    #[inline]
    pub fn occupied(&self) -> u32 {
        self.n_occupied
    }

    /// The arrays, for `n` buckets (at most what they hold).
    ///
    /// # Safety
    ///
    /// The arrays hold at least `n` slots (`n_buckets`, or more during a
    /// resize).
    #[inline(always)]
    unsafe fn parts_n(&mut self, n: u32) -> Parts<'_, K, V> {
        let n = n as usize;
        let flags: &mut [u32] = if self.n_buckets == 0 {
            &mut []
        } else {
            // SAFETY: the invariant: with buckets, the map owns its flags
            // array of `flagsize(n_buckets)` words; `&mut self` makes this
            // the only access.
            unsafe { core::slice::from_raw_parts_mut(self.flags, flagsize(self.n_buckets)) }
        };
        // SAFETY: the map owns its key array of at least `n` slots (the
        // caller's contract), distinct from the others; `&mut self` makes
        // this the only access.
        let keys = unsafe { slots_mut(self.keys, n) };
        // SAFETY: as above.
        let vals = unsafe { slots_mut(self.vals, n) };
        Parts { flags, keys, vals }
    }

    /// The arrays.
    #[inline(always)]
    fn parts(&mut self) -> Parts<'_, K, V> {
        // SAFETY: `n_buckets` slots: the invariant.
        unsafe { self.parts_n(self.n_buckets) }
    }

    /// The arrays, shared.
    #[inline(always)]
    fn view(&self) -> View<'_, K, V> {
        let n = self.n_buckets as usize;
        if n == 0 {
            return View {
                flags: &[],
                keys: &[],
                vals: &[],
            };
        }
        // SAFETY: as in `parts_n`, with `&self` excluding writers.
        let flags = unsafe { core::slice::from_raw_parts(self.flags, flagsize(self.n_buckets)) };
        // SAFETY: as above.
        let keys = unsafe { slots(self.keys, n) };
        // SAFETY: as above.
        let vals = unsafe { slots(self.vals, n) };
        View { flags, keys, vals }
    }

    /// Whether bucket `i` (< `n_buckets`) is live.
    #[inline(always)]
    fn is_live(&self, i: u32) -> bool {
        !is_either(self.view().flags, i)
    }

    /// The live bucket `i`'s key and value.
    #[inline(always)]
    fn entry(&self, i: u32) -> (&K, &V) {
        let v = self.view();
        debug_assert!(!is_either(v.flags, i));
        let i = i as usize;
        // SAFETY: a live bucket holds an initialized key (the invariant).
        let k = unsafe { v.keys[i].assume_init_ref() };
        // SAFETY: and an initialized value.
        let val = unsafe { v.vals[i].assume_init_ref() };
        (k, val)
    }

    /// The live bucket `i`'s key and value, the value mutable.
    #[inline(always)]
    fn entry_mut(&mut self, i: u32) -> (&K, &mut V) {
        let p = self.parts();
        debug_assert!(!is_either(p.flags, i));
        let i = i as usize;
        // SAFETY: a live bucket holds an initialized key (the invariant).
        let k = unsafe { p.keys[i].assume_init_ref() };
        // SAFETY: and an initialized value.
        let val = unsafe { p.vals[i].assume_init_mut() };
        (k, val)
    }

    /// `name##__idx`: the bucket of `key`, by `hash` and `eq`.
    #[inline]
    pub fn find_with(
        &self,
        key: &K,
        hash: impl Fn(&K) -> u32,
        eq: impl Fn(&K, &K) -> bool,
    ) -> Option<u32> {
        if self.n_buckets == 0 {
            return None;
        }
        let v = self.view();
        let mask = self.n_buckets - 1;
        let mut i = hash(key) & mask;
        let last = i;
        let mut step = 0u32;
        // a live bucket's key: only read where neither flag is set
        let live_eq = |i: u32| {
            // SAFETY: called only for a bucket that is neither empty nor
            // deleted, which holds an initialized key.
            eq(unsafe { v.keys[i as usize].assume_init_ref() }, key)
        };
        while !is_empty(v.flags, i) && (is_del(v.flags, i) || !live_eq(i)) {
            step += 1;
            i = (i + step) & mask;
            if i == last {
                return None;
            }
        }
        if is_either(v.flags, i) { None } else { Some(i) }
    }

    /// `name##__resize`, by `hash`: make room for `new_n_buckets` (rounded
    /// up to a power of two, at least 4), unless the entries would not fit
    /// at 0.77 load (then nothing changes). Out of memory: the error set,
    /// and the map as it was.
    pub fn resize_with(&mut self, new_n_buckets: u32, hash: impl Fn(&K) -> u32) -> Result<()> {
        let () = Self::CHECK;
        let mut new_n_buckets = roundup(new_n_buckets);
        if new_n_buckets < 4 {
            new_n_buckets = 4;
        }
        if self.size >= (new_n_buckets as f64 * UPPER_BOUND + 0.5) as u32 {
            // requested size is too small
            return Ok(());
        }
        let fwords = flagsize(new_n_buckets);
        let new_flags_ptr = alloc::mallocarray(fwords, size_of::<u32>()).cast::<u32>();
        if new_flags_ptr.is_null() {
            return Err(ErrorCode::GENERIC);
        }
        // SAFETY: a fresh array of `fwords` words, which only this function
        // uses until it is installed below.
        unsafe { ptr::write_bytes(new_flags_ptr, 0xaa, fwords) };
        // SAFETY: as above, initialized now.
        let new_flags = unsafe { core::slice::from_raw_parts_mut(new_flags_ptr, fwords) };
        if self.n_buckets < new_n_buckets {
            // expand
            if let Err(e) = self.grow_arrays(new_n_buckets) {
                // SAFETY: the fresh flags array, not used again.
                unsafe { alloc::free(new_flags_ptr.cast()) };
                return Err(e);
            }
        }

        // Rehash in place ("kick-out", as C): entries move between slots as
        // `MaybeUninit`s; a slot is read as a key only while live in the
        // old flags, which are then marked deleted.
        let old_n = self.n_buckets;
        let new_mask = new_n_buckets - 1;
        // SAFETY: the arrays hold `max(old_n, new_n_buckets)` slots now.
        let p = unsafe { self.parts_n(old_n.max(new_n_buckets)) };
        let (old, keys, vals) = (p.flags, p.keys, p.vals);
        for j in 0..old_n {
            if is_either(old, j) {
                continue;
            }
            let mut key = core::mem::replace(&mut keys[j as usize], MaybeUninit::uninit());
            let mut val = core::mem::replace(&mut vals[j as usize], MaybeUninit::uninit());
            set_del_true(old, j);
            loop {
                // SAFETY: `key` came from a live slot: initialized.
                let k = hash(unsafe { key.assume_init_ref() });
                let mut i = k & new_mask;
                let mut step = 0;
                while !is_empty(new_flags, i) {
                    step += 1;
                    i = (i + step) & new_mask;
                }
                set_empty_false(new_flags, i);
                if i < old_n && !is_either(old, i) {
                    // kick out the existing element
                    core::mem::swap(&mut key, &mut keys[i as usize]);
                    core::mem::swap(&mut val, &mut vals[i as usize]);
                    set_del_true(old, i);
                } else {
                    keys[i as usize] = key;
                    vals[i as usize] = val;
                    break;
                }
            }
        }
        if old_n > new_n_buckets {
            self.shrink_arrays(new_n_buckets);
        }
        // SAFETY: the old flags array (or NULL), replaced.
        unsafe { alloc::free(self.flags.cast()) };
        self.flags = new_flags_ptr;
        self.n_buckets = new_n_buckets;
        self.n_occupied = self.size;
        self.upper_bound = (self.n_buckets as f64 * UPPER_BOUND + 0.5) as u32;
        Ok(())
    }

    /// Grow the key and value arrays to `n` slots (C's expand step).
    fn grow_arrays(&mut self, n: u32) -> Result<()> {
        // SAFETY: `keys` is NULL or the map's own array; on failure it is
        // left as it was.
        let nk = unsafe { alloc::reallocarray(self.keys.cast(), n as usize, size_of::<K>()) };
        if nk.is_null() {
            return Err(ErrorCode::GENERIC);
        }
        self.keys = nk.cast();
        if Self::IS_MAP {
            // SAFETY: as above, for `vals`.
            let nv = unsafe { alloc::reallocarray(self.vals.cast(), n as usize, size_of::<V>()) };
            if nv.is_null() {
                return Err(ErrorCode::GENERIC);
            }
            self.vals = nv.cast();
        }
        Ok(())
    }

    /// Shrink the key and value arrays to `n` slots. C assigns the result
    /// even when it is NULL; the larger arrays are kept then (still valid
    /// and big enough).
    fn shrink_arrays(&mut self, n: u32) {
        // SAFETY: the map's own array; on failure it is left as it was.
        let nk = unsafe { alloc::reallocarray(self.keys.cast(), n as usize, size_of::<K>()) };
        if !nk.is_null() {
            self.keys = nk.cast();
        }
        if Self::IS_MAP {
            // SAFETY: as above, for `vals`.
            let nv = unsafe { alloc::reallocarray(self.vals.cast(), n as usize, size_of::<V>()) };
            if !nv.is_null() {
                self.vals = nv.cast();
            }
        }
    }

    /// `name##__put_idx`: the bucket for `key`, and whether it was newly
    /// taken (`true`: the key is written, the value slot is empty);
    /// otherwise the key is given back, and the bucket's pair untouched.
    fn put_idx_with(
        &mut self,
        key: K,
        hash: impl Fn(&K) -> u32,
        eq: impl Fn(&K, &K) -> bool,
    ) -> Result<(u32, Option<K>)> {
        if self.n_occupied >= self.upper_bound {
            if self.n_buckets > (self.size << 1) {
                // clear "deleted" elements
                self.resize_with(self.n_buckets - 1, &hash)?;
            } else {
                self.resize_with(self.n_buckets + 1, &hash)?;
            }
        }
        let n = self.n_buckets;
        let mask = n - 1;
        let h = hash(&key);
        let p = self.parts();
        let (fl, keys) = (p.flags, p.keys);
        let mut x;
        let mut site = n;
        let mut i = h & mask;
        if is_empty(fl, i) {
            x = i;
        } else {
            x = n;
            let last = i;
            let mut step = 0;
            loop {
                if is_empty(fl, i) {
                    break;
                }
                if !is_del(fl, i) {
                    // SAFETY: a live bucket: an initialized key.
                    if eq(unsafe { keys[i as usize].assume_init_ref() }, &key) {
                        break;
                    }
                } else {
                    site = i;
                }
                step += 1;
                i = (i + step) & mask;
                if i == last {
                    x = site;
                    break;
                }
            }
            if x == n {
                x = if is_empty(fl, i) && site != n {
                    site
                } else {
                    i
                };
            }
        }
        if is_empty(fl, x) {
            keys[x as usize] = MaybeUninit::new(key);
            set_both_false(fl, x);
            self.size += 1;
            self.n_occupied += 1;
            Ok((x, None))
        } else if is_del(fl, x) {
            keys[x as usize] = MaybeUninit::new(key);
            set_both_false(fl, x);
            self.size += 1;
            Ok((x, None))
        } else {
            Ok((x, Some(key)))
        }
    }

    /// `name##_put`, by `hash` and `eq`: insert, or replace the key and the
    /// value of an equal key (C overwrites both); the old pair.
    pub fn put_with(
        &mut self,
        key: K,
        val: V,
        hash: impl Fn(&K) -> u32,
        eq: impl Fn(&K, &K) -> bool,
    ) -> Result<Option<(K, V)>> {
        let (x, present) = self.put_idx_with(key, hash, eq)?;
        let p = self.parts();
        let x = x as usize;
        match present {
            None => {
                p.vals[x] = MaybeUninit::new(val);
                Ok(None)
            }
            Some(key) => {
                let old_k = core::mem::replace(&mut p.keys[x], MaybeUninit::new(key));
                let old_v = core::mem::replace(&mut p.vals[x], MaybeUninit::new(val));
                // SAFETY: the bucket was live: both were initialized.
                let old_k = unsafe { old_k.assume_init() };
                // SAFETY: as above.
                let old_v = unsafe { old_v.assume_init() };
                Ok(Some((old_k, old_v)))
            }
        }
    }

    /// `name##__remove_at_idx`: remove the live bucket `idx`; its pair.
    pub fn remove_at(&mut self, idx: u32) -> Option<(K, V)> {
        if idx >= self.n_buckets || !self.is_live(idx) {
            return None;
        }
        self.size -= 1;
        let p = self.parts();
        set_del_true(p.flags, idx);
        let i = idx as usize;
        let k = core::mem::replace(&mut p.keys[i], MaybeUninit::uninit());
        let v = core::mem::replace(&mut p.vals[i], MaybeUninit::uninit());
        // SAFETY: the bucket was live: its pair was initialized, and is moved
        // out once (the bucket is marked deleted).
        let k = unsafe { k.assume_init() };
        // SAFETY: as above.
        let v = unsafe { v.assume_init() };
        Some((k, v))
    }

    /// `name##_remove` by `hash` and `eq`.
    pub fn remove_with(
        &mut self,
        key: &K,
        hash: impl Fn(&K) -> u32,
        eq: impl Fn(&K, &K) -> bool,
    ) -> Option<(K, V)> {
        let idx = self.find_with(key, hash, eq)?;
        self.remove_at(idx)
    }

    /// The key and value at bucket `idx`, if live.
    pub fn at(&self, idx: u32) -> Option<(&K, &V)> {
        if idx < self.n_buckets && self.is_live(idx) {
            Some(self.entry(idx))
        } else {
            None
        }
    }

    /// The key and value at bucket `idx`, if live, the value mutable.
    pub fn at_mut(&mut self, idx: u32) -> Option<(&K, &mut V)> {
        if idx < self.n_buckets && self.is_live(idx) {
            Some(self.entry_mut(idx))
        } else {
            None
        }
    }

    /// `name##_iterate`: the next live entry from bucket `*iter`, advancing
    /// `*iter` past it; `None` at the end (`GIT_ITEROVER`). Entries may be
    /// removed between calls, as in C.
    pub fn iterate(&self, iter: &mut HashMapIter) -> Option<(&K, &V)> {
        while *iter < self.n_buckets {
            let i = *iter;
            *iter += 1;
            if self.is_live(i) {
                return Some(self.entry(i));
            }
        }
        None
    }

    /// The entries in bucket order (C's iteration order).
    pub fn iter(&self) -> Iter<'_, K, V, X> {
        Iter { map: self, i: 0 }
    }

    /// The keys in bucket order.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.iter().map(|(k, _)| k)
    }

    /// The values in bucket order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }

    /// Visit every entry in bucket order, with the value mutable.
    pub fn for_each_mut(&mut self, mut f: impl FnMut(&K, &mut V)) {
        for i in 0..self.n_buckets {
            if self.is_live(i) {
                let (k, v) = self.entry_mut(i);
                f(k, v);
            }
        }
    }

    /// Keep the entries for which `keep` is true, removing the others in
    /// bucket order (the C's iterate-and-remove loops).
    pub fn retain(&mut self, mut keep: impl FnMut(&K, &mut V) -> bool) {
        for i in 0..self.n_buckets {
            if self.is_live(i) {
                let (k, v) = self.entry_mut(i);
                if !keep(k, v) {
                    drop(self.remove_at(i));
                }
            }
        }
    }

    fn drop_entries(&mut self) {
        if !(core::mem::needs_drop::<K>() || core::mem::needs_drop::<V>()) {
            return;
        }
        for i in 0..self.n_buckets {
            drop(self.remove_at(i));
        }
    }

    /// `name##_clear`: remove every entry, keeping the buckets.
    pub fn clear(&mut self) {
        self.drop_entries();
        if !self.flags.is_null() {
            self.parts().flags.fill(0xaaaa_aaaa);
            self.size = 0;
            self.n_occupied = 0;
        }
    }

    /// `name##_dispose`: remove every entry and free the arrays; the map is
    /// empty and has no buckets. (C also zeroes the leading field; it is
    /// left as is.)
    pub fn dispose(&mut self) {
        self.drop_entries();
        for p in [
            self.flags.cast::<c_void>(),
            self.keys.cast(),
            self.vals.cast(),
        ] {
            // SAFETY: the map's own arrays (or NULL), freed once and
            // forgotten below.
            unsafe { alloc::free(p) };
        }
        self.n_buckets = 0;
        self.size = 0;
        self.n_occupied = 0;
        self.upper_bound = 0;
        self.flags = ptr::null_mut();
        self.keys = ptr::null_mut();
        self.vals = ptr::null_mut();
    }

    /// Swap the contents of two maps (leading fields included), as the C's
    /// `memcpy` swaps.
    pub fn swap(&mut self, other: &mut Self) {
        core::mem::swap(self, other);
    }
}

impl<K: HashKey, V, X> GitHashMap<K, V, X> {
    /// `name##_contains`.
    #[inline]
    pub fn contains(&self, key: &K) -> bool {
        self.find(key).is_some()
    }

    /// `name##__idx`: the bucket of `key`.
    #[inline]
    pub fn find(&self, key: &K) -> Option<u32> {
        self.find_with(key, K::hash_code, K::key_eq)
    }

    /// `name##_get`.
    #[inline]
    pub fn get(&self, key: &K) -> Option<&V> {
        self.find(key).map(|i| self.entry(i).1)
    }

    /// `name##_get`, with the key that is stored.
    #[inline]
    pub fn get_key_value(&self, key: &K) -> Option<(&K, &V)> {
        self.find(key).map(|i| self.entry(i))
    }

    /// `name##_get`, the value mutable.
    #[inline]
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        let i = self.find(key)?;
        Some(self.entry_mut(i).1)
    }

    /// `name##_put`: insert, or replace the key and value of an equal key;
    /// the old pair. Out of memory: the error set, nothing inserted.
    #[inline]
    pub fn put(&mut self, key: K, val: V) -> Result<Option<(K, V)>> {
        self.put_with(key, val, K::hash_code, K::key_eq)
    }

    /// `name##_remove`: the removed pair, `None` if absent
    /// (`GIT_ENOTFOUND`).
    #[inline]
    pub fn remove(&mut self, key: &K) -> Option<(K, V)> {
        self.remove_with(key, K::hash_code, K::key_eq)
    }

    /// `name##__resize`.
    #[inline]
    pub fn resize(&mut self, new_n_buckets: u32) -> Result<()> {
        self.resize_with(new_n_buckets, K::hash_code)
    }
}

impl<K: HashKey, X> GitHashMap<K, (), X> {
    /// `name##_add` of a set: insert, or replace an equal key (C
    /// overwrites it); the old key.
    #[inline]
    pub fn add(&mut self, key: K) -> Result<Option<K>> {
        Ok(self.put(key, ())?.map(|(k, ())| k))
    }

    /// `name##_iterate` of a set: the next key from `*iter`, advancing
    /// past it.
    pub fn iterate_keys(&self, iter: &mut HashMapIter) -> Option<&K> {
        self.iterate(iter).map(|(k, ())| k)
    }
}

/// The entries of a [`GitHashMap`], in bucket order.
pub struct Iter<'a, K, V, X> {
    map: &'a GitHashMap<K, V, X>,
    i: u32,
}

impl<'a, K, V, X> Iterator for Iter<'a, K, V, X> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        self.map.iterate(&mut self.i)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.map.len()))
    }
}

impl<'a, K, V, X> IntoIterator for &'a GitHashMap<K, V, X> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V, X>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

// ---------------------------------------------------------------------------
// String keys (hashmap_str.h)
// ---------------------------------------------------------------------------

/// `git_hashmap_str_hash`: X31 over the bytes up to the NUL, each read as
/// C's (signed) `char`.
#[inline]
pub fn str_hash(s: &[u8]) -> u32 {
    let mut it = s.iter().take_while(|&&c| c != 0);
    let Some(&first) = it.next() else { return 0 };
    let mut h = first as i8 as u32;
    for &c in it {
        h = (h << 5).wrapping_sub(h).wrapping_add(c as i8 as u32);
    }
    h
}

/// A `const char *` key of `hashmap_str.h`: a thin pointer to a
/// NUL-terminated string that lives for `'a`. Hash `git_hashmap_str_hash`,
/// equality `strcmp`.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct CStrKey<'a>(NonNull<c_char>, PhantomData<&'a CStr>);

// SAFETY: a shared reference to an immutable string.
unsafe impl Send for CStrKey<'_> {}
// SAFETY: as above.
unsafe impl Sync for CStrKey<'_> {}

impl<'a> CStrKey<'a> {
    /// The key for `s`.
    #[inline]
    pub const fn new(s: &'a CStr) -> CStrKey<'a> {
        // SAFETY: a reference's pointer is not null.
        CStrKey(
            unsafe { NonNull::new_unchecked(s.as_ptr().cast_mut()) },
            PhantomData,
        )
    }

    /// The key for a C string pointer.
    ///
    /// # Safety
    ///
    /// `p` is a NUL-terminated string that stays valid and unwritten for
    /// `'a`.
    #[inline]
    pub const unsafe fn from_ptr(p: NonNull<c_char>) -> CStrKey<'a> {
        CStrKey(p, PhantomData)
    }

    /// The string.
    #[inline]
    pub fn as_c_str(&self) -> &'a CStr {
        // SAFETY: the constructors' contract: a live NUL-terminated string
        // for `'a`.
        unsafe { CStr::from_ptr(self.0.as_ptr()) }
    }

    /// The pointer.
    #[inline]
    pub const fn as_ptr(&self) -> *const c_char {
        self.0.as_ptr()
    }
}

impl core::fmt::Debug for CStrKey<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(self.as_c_str(), f)
    }
}

/// By content (`strcmp`), as the maps compare keys.
impl PartialEq for CStrKey<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.key_eq(other)
    }
}

impl Eq for CStrKey<'_> {}

/// By content, consistent with `PartialEq`.
impl core::hash::Hash for CStrKey<'_> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.as_c_str().hash(state);
    }
}

impl HashKey for CStrKey<'_> {
    #[inline]
    fn hash_code(&self) -> u32 {
        str_hash(self.as_c_str().to_bytes())
    }
    #[inline]
    fn key_eq(&self, other: &Self) -> bool {
        self.0 == other.0 || self.as_c_str() == other.as_c_str()
    }
}

impl HashKey for &CStr {
    #[inline]
    fn hash_code(&self) -> u32 {
        str_hash(self.to_bytes())
    }
    #[inline]
    fn key_eq(&self, other: &Self) -> bool {
        *self == *other
    }
}

/// `pack.c`'s `off64_hash` and `==`.
impl HashKey for i64 {
    #[inline]
    fn hash_code(&self) -> u32 {
        let k = *self;
        ((k >> 33) ^ k ^ (k << 11)) as u32
    }
    #[inline]
    fn key_eq(&self, other: &Self) -> bool {
        self == other
    }
}

/// `git_hashmap_str`: `const char *` to `void *`.
pub type GitHashMapStr<'a> = GitHashMap<CStrKey<'a>, *mut c_void>;
/// `git_hashset_str`.
pub type GitHashSetStr<'a> = GitHashSet<CStrKey<'a>>;

// ---------------------------------------------------------------------------
// Exports of a C-named map
// ---------------------------------------------------------------------------

/// Define the `extern "C"` functions of a hash map that C declares with
/// `GIT_HASHMAP_PROTOTYPES` and whose functions a ported module defined
/// (`GIT_HASHMAP_FUNCTIONS(name, , …)`: `pack.c`'s `git_pack_oidmap`,
/// `mwindow.c`'s `git_mwindow_packmap`).
///
/// `$map` is the Rust map type (whose key and value have the layout of the
/// C's `key_t` and `val_t`), `$k` and `$v` the C's types. The functions get
/// the names given, and keep C's results (0, `GIT_ENOTFOUND`,
/// `GIT_ITEROVER`, -1).
///
/// ```ignore
/// crate::git_hashmap_exports! {
///     map: GitHashMap<OidPtr, *mut GitPackEntry>, key: *const Oid, val: *mut GitPackEntry,
///     size: git_pack_oidmap_size, contains: git_pack_oidmap_contains,
///     remove: git_pack_oidmap_remove, clear: git_pack_oidmap_clear,
///     dispose: git_pack_oidmap_dispose, get: git_pack_oidmap_get,
///     put: git_pack_oidmap_put, iterate: git_pack_oidmap_iterate,
///     foreach: git_pack_oidmap_foreach,
/// }
/// ```
///
/// The key conversion is `transmute_copy` from `$k` to the map's key type,
/// which must be layout-identical (`#[repr(transparent)]` over a pointer
/// for the provided key types) and whose contract the C caller then
/// upholds.
#[macro_export]
#[doc(hidden)] // crate-internal: expands to crate-private paths, unusable outside
macro_rules! git_hashmap_exports {
    (
        map: $map:ty, key: $k:ty, val: $v:ty,
        size: $size:ident, contains: $contains:ident, remove: $remove:ident,
        clear: $clear:ident, dispose: $dispose:ident, get: $get:ident,
        put: $put:ident, iterate: $iterate:ident, foreach: $foreach:ident $(,)?
    ) => {
        const _: () = {
            type M = $map;
            #[inline(always)]
            unsafe fn key_of(k: $k) -> <M as $crate::util::hashmap::MapTypes>::Key {
                const {
                    assert!(
                        size_of::<$k>() == size_of::<<M as $crate::util::hashmap::MapTypes>::Key>()
                    )
                };
                // SAFETY: the key types are layout-identical (the macro's
                // contract), and the caller's key meets the Rust key's.
                unsafe { ::core::mem::transmute_copy(&k) }
            }
            #[inline(always)]
            fn key_to(k: &<M as $crate::util::hashmap::MapTypes>::Key) -> $k {
                // SAFETY: as above, the other way.
                unsafe { ::core::mem::transmute_copy(k) }
            }

            /// `name##_size`.
            ///
            /// # Safety
            ///
            /// `h` is a valid map, not written concurrently.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $size(h: *mut M) -> u32 {
                // SAFETY: a valid map.
                unsafe { (*h).len() as u32 }
            }

            /// `name##_contains`.
            ///
            /// # Safety
            ///
            /// `h` is a valid map, not written concurrently; `key` is valid
            /// for the key type.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $contains(h: *mut M, key: $k) -> bool {
                $crate::util::export::barrier(|| {
                    // SAFETY: the caller's contract.
                    unsafe { (*h).contains(&key_of(key)) }
                })
            }

            /// `name##_remove`.
            ///
            /// # Safety
            ///
            /// `h` is a valid map, exclusively ours; `key` as for `contains`.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $remove(h: *mut M, key: $k) -> ::core::ffi::c_int {
                $crate::util::export::barrier(|| {
                    // SAFETY: the caller's contract.
                    match unsafe { (*h).remove(&key_of(key)) } {
                        Some(_) => 0,
                        None => $crate::util::errors::ErrorCode::NOT_FOUND.raw(),
                    }
                })
            }

            /// `name##_clear`.
            ///
            /// # Safety
            ///
            /// `h` is NULL or a valid map, exclusively ours.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $clear(h: *mut M) {
                // SAFETY: NULL or a valid map.
                if let Some(h) = unsafe { h.as_mut() } {
                    h.clear();
                }
            }

            /// `name##_dispose`.
            ///
            /// # Safety
            ///
            /// `h` is a valid map, exclusively ours.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $dispose(h: *mut M) {
                // SAFETY: a valid map.
                unsafe { (*h).dispose() }
            }

            /// `name##_get`.
            ///
            /// # Safety
            ///
            /// As `contains`; `out` is writable.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $get(out: *mut $v, h: *mut M, key: $k) -> ::core::ffi::c_int {
                $crate::util::export::barrier(|| {
                    // SAFETY: the caller's contract.
                    match unsafe { (*h).get(&key_of(key)) } {
                        Some(v) => {
                            // SAFETY: `out` is writable.
                            unsafe { out.write(*v) };
                            0
                        }
                        None => $crate::util::errors::ErrorCode::NOT_FOUND.raw(),
                    }
                })
            }

            /// `name##_put`.
            ///
            /// # Safety
            ///
            /// `h` is a valid map, exclusively ours; `key` is valid for the
            /// key type for as long as it is in the map.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $put(h: *mut M, key: $k, val: $v) -> ::core::ffi::c_int {
                $crate::util::export::barrier(|| {
                    // SAFETY: the caller's contract.
                    match unsafe { (*h).put(key_of(key), val) } {
                        Ok(_) => 0,
                        Err(e) => e.raw(),
                    }
                })
            }

            /// `name##_iterate`.
            ///
            /// # Safety
            ///
            /// `iter` is writable; `h` a valid map; `key` and `val` NULL or
            /// writable.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $iterate(
                iter: *mut u32,
                key: *mut $k,
                val: *mut $v,
                h: *mut M,
            ) -> ::core::ffi::c_int {
                $crate::util::export::barrier(|| {
                    // SAFETY: the caller's contract.
                    let (iter, h) = unsafe { (&mut *iter, &*h) };
                    match h.iterate(iter) {
                        Some((k, v)) => {
                            // SAFETY: NULL or writable.
                            unsafe {
                                $crate::util::export::write_opt(key, key_to(k));
                                $crate::util::export::write_opt(val, *v);
                            }
                            0
                        }
                        None => $crate::util::errors::ErrorCode::ITER_OVER.raw(),
                    }
                })
            }

            /// `name##_foreach`.
            ///
            /// # Safety
            ///
            /// `h` is a valid map; `cb` is sound to call with its entries
            /// and does not write the map.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn $foreach(
                h: *mut M,
                cb: unsafe extern "C" fn($k, $v) -> ::core::ffi::c_int,
            ) -> ::core::ffi::c_int {
                $crate::util::export::barrier(|| {
                    let mut it = 0u32;
                    loop {
                        // SAFETY: a valid map; the entry is copied out
                        // before the callback runs.
                        let e = unsafe { (*h).iterate(&mut it).map(|(k, v)| (key_to(k), *v)) };
                        let Some((k, v)) = e else { return 0 };
                        // SAFETY: the caller's contract.
                        let r = unsafe { cb(k, v) };
                        if r != 0 {
                            return r;
                        }
                    }
                })
            }
        };
    };
}

/// The key and value types of a map type, for [`git_hashmap_exports!`].
pub trait MapTypes {
    /// The key.
    type Key;
    /// The value.
    type Val;
}

impl<K, V, X> MapTypes for GitHashMap<K, V, X> {
    type Key = K;
    type Val = V;
}

crate::c_layout_test! {
    headers: ["hashmap_str.h", "index_map.h", "pack.h", "cache.h", "revwalk.h"],
    types: [
        crate::hashmap_oid::OidPtrMap => "git_cache_oidmap" {
            n_buckets, size, n_occupied, upper_bound, flags, keys, vals
        },
        crate::hashmap_oid::OidMap<'static, *mut c_void> => "git_revwalk_oidmap" {
            n_buckets, size, n_occupied, upper_bound, flags, keys, vals
        },
        GitHashMapStr<'static> => "git_hashmap_str" {
            n_buckets, size, n_occupied, upper_bound, flags, keys, vals
        },
        GitHashSetStr<'static> => "git_hashset_str" {
            n_buckets, size, n_occupied, upper_bound, flags, keys, vals
        },
        GitHashMap<i64, *mut c_void> => "git_pack_offsetmap" {
            n_buckets, size, n_occupied, upper_bound, flags, keys, vals
        },
        GitHashMap<*mut c_void, *mut c_void, u32> => "git_index_entrymap" {
            extra as "ignore_case", n_buckets, size, n_occupied, upper_bound, flags, keys, vals
        },
    ],
    asserts: ["sizeof(git_hashmap_iter_t) == 4"],
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn hash_is_c_s() {
        assert_eq!(str_hash(b""), 0);
        assert_eq!(str_hash(b"a"), 97);
        assert_eq!(str_hash(b"ab"), 97 * 31 + 98);
        // a byte over 0x7f is a negative `char`
        assert_eq!(str_hash(b"\xe9"), 0xffff_ffe9);
        assert_eq!(str_hash(b"a\0b"), 97);
        assert_eq!(7i64.hash_code(), 7 ^ (7 << 11));
        assert_eq!((-1i64).hash_code(), 0xffff_f800);
    }

    #[test]
    fn put_get_remove() {
        let _g = crate::util::test_support::init();
        let strs: Vec<CString> = (0..2000)
            .map(|i| CString::new(format!("k{i}")).unwrap())
            .collect();
        let other = CString::new("k5").unwrap();
        let mut m: GitHashMap<CStrKey<'_>, usize> = GitHashMap::new();
        for (i, s) in strs.iter().enumerate() {
            assert!(m.put(CStrKey::new(s), i).unwrap().is_none());
        }
        assert_eq!(m.len(), 2000);
        assert!(m.buckets().is_power_of_two());
        for (i, s) in strs.iter().enumerate() {
            assert_eq!(m.get(&CStrKey::new(s)), Some(&i));
        }
        assert!(!m.contains(&CStrKey::new(c"nope")));
        let old = m.put(CStrKey::new(&other), 55).unwrap().unwrap();
        assert_eq!(old.1, 5);
        assert_eq!(
            m.get_key_value(&CStrKey::new(c"k5")).unwrap().0.as_ptr(),
            other.as_ptr()
        );
        for s in strs.iter().step_by(2) {
            assert!(m.remove(&CStrKey::new(s)).is_some());
        }
        assert!(m.remove(&CStrKey::new(c"k0")).is_none());
        assert_eq!(m.len(), 1000);
        assert_eq!(m.iter().count(), 1000);
        let mut it = 0;
        let mut n = 0;
        while m.iterate(&mut it).is_some() {
            n += 1;
        }
        assert_eq!(n, 1000);
        m.retain(|_, v| *v % 4 == 1);
        // k5 now holds 55
        assert_eq!(m.len(), 499);
        m.clear();
        assert_eq!(m.len(), 0);
        assert!(m.buckets() > 0);
        m.put(CStrKey::new(c"x"), 1).unwrap();
        m.dispose();
        assert_eq!(m.buckets(), 0);
    }

    #[test]
    fn owned_values_are_dropped() {
        let _g = crate::util::test_support::init();
        let rc = std::rc::Rc::new(());
        let mut m: GitHashMap<i64, std::rc::Rc<()>> = GitHashMap::new();
        for i in 0..100 {
            m.put(i, rc.clone()).unwrap();
        }
        assert_eq!(std::rc::Rc::strong_count(&rc), 101);
        drop(m.remove(&3));
        assert_eq!(std::rc::Rc::strong_count(&rc), 100);
        m.clear();
        assert_eq!(std::rc::Rc::strong_count(&rc), 1);
        for i in 0..10 {
            m.put(i, rc.clone()).unwrap();
        }
        drop(m);
        assert_eq!(std::rc::Rc::strong_count(&rc), 1);
    }

    #[test]
    fn set() {
        let _g = crate::util::test_support::init();
        let mut s: GitHashSet<i64> = GitHashSet::new();
        for i in 0..50 {
            assert!(s.add(i * 7).unwrap().is_none());
        }
        assert_eq!(s.add(14).unwrap(), Some(14));
        assert_eq!(s.len(), 50);
        assert!(s.vals.is_null());
        let mut it = 0;
        let mut n = 0;
        while s.iterate_keys(&mut it).is_some() {
            n += 1;
        }
        assert_eq!(n, 50);
    }

    #[test]
    fn resize_rules() {
        let _g = crate::util::test_support::init();
        let mut m: GitHashMap<i64, u8> = GitHashMap::new();
        m.resize(0).unwrap();
        assert_eq!(m.buckets(), 4);
        m.resize(17).unwrap();
        assert_eq!(m.buckets(), 32);
        for i in 0..20 {
            m.put(i, 0).unwrap();
        }
        // too small for 20 entries: unchanged
        m.resize(8).unwrap();
        assert_eq!(m.buckets(), 32);
        // shrink
        for i in 0..18 {
            m.remove(&i);
        }
        m.resize(4).unwrap();
        assert_eq!(m.buckets(), 4);
        assert_eq!(m.keys().copied().collect::<Vec<_>>().len(), 2);
        assert!(m.contains(&18) && m.contains(&19));
    }

    #[test]
    fn deleted_slots_are_reused() {
        let _g = crate::util::test_support::init();
        let mut m: GitHashMap<i64, i64> = GitHashMap::new();
        for round in 0..50 {
            for i in 0..10 {
                m.put(round * 10 + i, i).unwrap();
            }
            for i in 0..10 {
                assert!(m.remove(&(round * 10 + i)).is_some());
            }
        }
        assert!(m.is_empty());
        assert!(m.buckets() <= 32);
    }
}

#[cfg(test)]
#[allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]
mod exports_macro_tests {
    use super::*;
    use core::ffi::{c_char, c_int, c_void};
    use core::mem::MaybeUninit;
    use core::ptr;

    // A C-named map family, as `pack.c`'s `git_pack_oidmap` will be exported.
    type TestMap = GitHashMap<CStrKey<'static>, *mut c_void>;
    crate::git_hashmap_exports! {
        map: TestMap, key: *const c_char, val: *mut c_void,
        size: u1test_strmap_size, contains: u1test_strmap_contains,
        remove: u1test_strmap_remove, clear: u1test_strmap_clear,
        dispose: u1test_strmap_dispose, get: u1test_strmap_get,
        put: u1test_strmap_put, iterate: u1test_strmap_iterate,
        foreach: u1test_strmap_foreach,
    }

    unsafe extern "C" {
        fn u1test_strmap_size(h: *mut TestMap) -> u32;
        fn u1test_strmap_contains(h: *mut TestMap, k: *const c_char) -> bool;
        fn u1test_strmap_remove(h: *mut TestMap, k: *const c_char) -> c_int;
        fn u1test_strmap_clear(h: *mut TestMap);
        fn u1test_strmap_dispose(h: *mut TestMap);
        fn u1test_strmap_get(out: *mut *mut c_void, h: *mut TestMap, k: *const c_char) -> c_int;
        fn u1test_strmap_put(h: *mut TestMap, k: *const c_char, v: *mut c_void) -> c_int;
        fn u1test_strmap_iterate(
            it: *mut u32,
            k: *mut *const c_char,
            v: *mut *mut c_void,
            h: *mut TestMap,
        ) -> c_int;
        fn u1test_strmap_foreach(
            h: *mut TestMap,
            cb: unsafe extern "C" fn(*const c_char, *mut c_void) -> c_int,
        ) -> c_int;
    }

    unsafe extern "C" fn count_cb(_k: *const c_char, v: *mut c_void) -> c_int {
        if v.addr() == 2 { 42 } else { 0 }
    }

    #[test]
    fn hashmap_exports_macro() {
        let _g = crate::util::test_support::init();
        let mut m: MaybeUninit<TestMap> = MaybeUninit::zeroed();
        let m = m.as_mut_ptr();
        // SAFETY: as C would call them; the keys are static strings.
        unsafe {
            assert_eq!(
                u1test_strmap_put(m, c"a".as_ptr(), ptr::without_provenance_mut(1)),
                0
            );
            assert_eq!(
                u1test_strmap_put(m, c"b".as_ptr(), ptr::without_provenance_mut(2)),
                0
            );
            assert_eq!(
                u1test_strmap_put(m, c"a".as_ptr(), ptr::without_provenance_mut(3)),
                0
            );
            assert_eq!(u1test_strmap_size(m), 2);
            assert!(u1test_strmap_contains(m, c"b".as_ptr()));
            let mut v = ptr::null_mut();
            assert_eq!(u1test_strmap_get(&mut v, m, c"a".as_ptr()), 0);
            assert_eq!(v.addr(), 3);
            assert_eq!(u1test_strmap_get(&mut v, m, c"zz".as_ptr()), -3);
            let (mut it, mut k, mut n) = (0u32, ptr::null(), 0);
            while u1test_strmap_iterate(&mut it, &mut k, ptr::null_mut(), m) == 0 {
                assert!(!k.is_null());
                n += 1;
            }
            assert_eq!(n, 2);
            assert_eq!(u1test_strmap_iterate(&mut it, &mut k, &mut v, m), -31);
            assert_eq!(u1test_strmap_foreach(m, count_cb), 42);
            assert_eq!(u1test_strmap_remove(m, c"a".as_ptr()), 0);
            assert_eq!(u1test_strmap_remove(m, c"a".as_ptr()), -3);
            u1test_strmap_clear(m);
            assert_eq!(u1test_strmap_size(m), 0);
            u1test_strmap_dispose(m);
            assert_eq!((*m).buckets(), 0);
        }
    }
}
