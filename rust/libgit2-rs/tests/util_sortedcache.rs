//! Tests of `sortedcache.c`'s port: the exported C ABI (with C-style items
//! ending in their key, and `extern "C"` callbacks) and the typed safe API.

#![allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]

use core::ffi::{CStr, c_char, c_int, c_void};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use libgit2_rs::__test::util::errors::{self, ErrorCode};
use libgit2_rs::__test::util::sortedcache::SortedCache;
use libgit2_rs::__test::util::sortedcache::{Shared, ffi};
use libgit2_rs::__test::util::test_support;

fn msg() -> String {
    String::from_utf8(errors::last_message().unwrap_or_default()).unwrap()
}

/// `sortedcache_test_struct`.
#[repr(C)]
struct Item {
    value: c_int,
    smaller_value: c_char,
    path: [c_char; 0],
}
const PATH_OFFSET: usize = core::mem::offset_of!(Item, path);

/// `strcmp` (in Rust, so that Miri runs it).
unsafe fn strcmp(a: *const c_char, b: *const c_char) -> c_int {
    // SAFETY: the caller passes NUL-terminated strings.
    let (a, b) = unsafe { (CStr::from_ptr(a), CStr::from_ptr(b)) };
    a.cmp(b) as c_int
}

unsafe extern "C" fn item_cmp(a: *const c_void, b: *const c_void) -> c_int {
    // SAFETY: items of the cache, with their keys at `PATH_OFFSET`.
    unsafe {
        strcmp(
            a.cast::<c_char>().add(PATH_OFFSET),
            b.cast::<c_char>().add(PATH_OFFSET),
        )
    }
}

unsafe extern "C" fn item_free(payload: *mut c_void, item: *mut c_void) {
    // SAFETY: the payload is an `AtomicUsize`; the item one of ours.
    unsafe {
        (*payload.cast::<AtomicUsize>()).fetch_add(1, Ordering::SeqCst);
        (*item.cast::<Item>()).smaller_value = 0;
    }
}

unsafe extern "C" fn name_cmp(a: *const c_void, b: *const c_void) -> c_int {
    // SAFETY: NUL-terminated keys at offset 0.
    unsafe { strcmp(a.cast(), b.cast()) }
}

unsafe fn key<'a>(item: *mut c_void, offset: usize) -> &'a str {
    // SAFETY: the caller passes an item with its key at `offset`.
    unsafe { CStr::from_ptr(item.cast::<c_char>().add(offset)) }
        .to_str()
        .unwrap()
}

#[test]
fn name_only() {
    let _g = test_support::init();
    let mut sc: *const Shared = core::ptr::null();
    let mut item: *mut c_void = core::ptr::null_mut();
    let mut pos = 99usize;
    // SAFETY: the cache is used as `sortedcache.h` says.
    unsafe {
        assert_eq!(
            ffi::git_sortedcache_new(
                &mut sc,
                0,
                None,
                core::ptr::null_mut(),
                Some(name_cmp),
                core::ptr::null()
            ),
            0
        );
        assert_eq!(CStr::from_ptr(ffi::git_sortedcache_path(sc)), c"");
        assert_eq!(ffi::git_sortedcache_wlock(sc), 0);
        for k in [c"aaa", c"bbb", c"zzz", c"mmm", c"iii"] {
            assert_eq!(ffi::git_sortedcache_upsert(&mut item, sc, k.as_ptr()), 0);
        }
        // the same key again: the same item
        let again = item;
        assert_eq!(
            ffi::git_sortedcache_upsert(&mut item, sc, c"iii".as_ptr()),
            0
        );
        assert_eq!(item, again);
        ffi::git_sortedcache_wunlock(sc);

        assert_eq!(ffi::git_sortedcache_entrycount(sc), 5);
        assert_eq!(
            key(ffi::git_sortedcache_lookup(sc, c"mmm".as_ptr()), 0),
            "mmm"
        );
        assert!(ffi::git_sortedcache_lookup(sc, c"qqq".as_ptr()).is_null());
        let order: Vec<_> = (0..5)
            .map(|i| key(ffi::git_sortedcache_entry(sc, i), 0))
            .collect();
        assert_eq!(order, ["aaa", "bbb", "iii", "mmm", "zzz"]);
        assert!(ffi::git_sortedcache_entry(sc, 5).is_null());
        assert_eq!(
            ffi::git_sortedcache_lookup_index(&mut pos, sc, c"iii".as_ptr()),
            0
        );
        assert_eq!(pos, 2);
        assert_eq!(
            ffi::git_sortedcache_lookup_index(&mut pos, sc, c"abc".as_ptr()),
            ErrorCode::NOT_FOUND.raw()
        );
        assert_eq!(pos, 1);

        assert_eq!(ffi::git_sortedcache_clear(sc, true), 0);
        assert_eq!(ffi::git_sortedcache_entrycount(sc), 0);
        assert!(ffi::git_sortedcache_entry(sc, 0).is_null());
        ffi::git_sortedcache_free(sc);
        ffi::git_sortedcache_free(core::ptr::null());
    }
}

#[test]
fn in_memory_items_and_callbacks() {
    let _g = test_support::init();
    let count = AtomicUsize::new(0);
    let payload = (&raw const count).cast_mut().cast::<c_void>();
    let mut sc: *const Shared = core::ptr::null();
    let mut item: *mut c_void = core::ptr::null_mut();
    let mut pos = 0usize;
    // SAFETY: as in `name_only`; the callbacks fit the items.
    unsafe {
        assert_eq!(
            ffi::git_sortedcache_new(
                &mut sc,
                PATH_OFFSET,
                Some(item_free),
                payload,
                Some(item_cmp),
                c"some/path".as_ptr()
            ),
            0
        );
        assert_eq!(CStr::from_ptr(ffi::git_sortedcache_path(sc)), c"some/path");
        assert_eq!(ffi::git_sortedcache_wlock(sc), 0);
        for (k, v) in [(c"testing", 10), (c"again", 20), (c"final", 30)] {
            assert_eq!(ffi::git_sortedcache_upsert(&mut item, sc, k.as_ptr()), 0);
            // fresh items are zeroed, and 8-aligned
            assert_eq!((*item.cast::<Item>()).value, 0);
            assert_eq!(item.addr() % 8, 0);
            (*item.cast::<Item>()).value = v;
        }
        ffi::git_sortedcache_wunlock(sc);

        assert_eq!(ffi::git_sortedcache_rlock(sc), 0);
        let vals: Vec<_> = (0..3)
            .map(|i| {
                let it = ffi::git_sortedcache_entry(sc, i);
                (key(it, PATH_OFFSET), (*it.cast::<Item>()).value)
            })
            .collect();
        assert_eq!(vals, [("again", 20), ("final", 30), ("testing", 10)]);

        // a copy, with the default memcpy of the bytes before the key
        let mut copy: *const Shared = core::ptr::null();
        assert_eq!(
            ffi::git_sortedcache_copy(&mut copy, sc, false, None, core::ptr::null_mut()),
            0
        );
        ffi::git_sortedcache_runlock(sc);
        let it = ffi::git_sortedcache_lookup(copy, c"final".as_ptr());
        assert_eq!((*it.cast::<Item>()).value, 30);
        assert_eq!(
            key(ffi::git_sortedcache_entry(copy, 0), PATH_OFFSET),
            "again"
        );
        ffi::git_sortedcache_incref(copy);
        ffi::git_sortedcache_free(copy);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        ffi::git_sortedcache_free(copy);
        // the copy shares the free callback: three items freed
        assert_eq!(count.load(Ordering::SeqCst), 3);

        assert_eq!(ffi::git_sortedcache_wlock(sc), 0);
        assert_eq!(
            ffi::git_sortedcache_lookup_index(&mut pos, sc, c"again".as_ptr()),
            0
        );
        assert_eq!(ffi::git_sortedcache_remove(sc, pos), 0);
        assert_eq!(count.load(Ordering::SeqCst), 4);
        assert_eq!(
            ffi::git_sortedcache_remove(sc, 7),
            ErrorCode::NOT_FOUND.raw()
        );
        assert_eq!(msg(), "removing item out of range");
        assert_eq!(ffi::git_sortedcache_entrycount(sc), 2);
        ffi::git_sortedcache_wunlock(sc);

        ffi::git_sortedcache_free(sc);
        assert_eq!(count.load(Ordering::SeqCst), 6);
    }
}

unsafe extern "C" fn failing_copy(payload: *mut c_void, _t: *mut c_void, _s: *mut c_void) -> c_int {
    // SAFETY: the payload is an `AtomicUsize`.
    let n = unsafe { &*payload.cast::<AtomicUsize>() }.fetch_add(1, Ordering::SeqCst);
    if n == 1 { -7 } else { 0 }
}

#[test]
fn copy_failure_and_no_comparison() {
    let _g = test_support::init();
    let calls = AtomicUsize::new(0);
    let mut sc: *const Shared = core::ptr::null();
    let mut copy: *const Shared = core::ptr::without_provenance::<Shared>(1);
    let mut item: *mut c_void = core::ptr::null_mut();
    let mut pos = 42usize;
    // SAFETY: as in `name_only`.
    unsafe {
        assert_eq!(
            ffi::git_sortedcache_new(
                &mut sc,
                0,
                None,
                core::ptr::null_mut(),
                None,
                core::ptr::null()
            ),
            0
        );
        assert_eq!(ffi::git_sortedcache_wlock(sc), 0);
        for k in [c"b", c"a", c"c"] {
            assert_eq!(ffi::git_sortedcache_upsert(&mut item, sc, k.as_ptr()), 0);
        }
        ffi::git_sortedcache_wunlock(sc);
        // without a comparison: insertion order, and lookup_index is -1
        assert_eq!(key(ffi::git_sortedcache_entry(sc, 0), 0), "b");
        errors::clear();
        assert_eq!(
            ffi::git_sortedcache_lookup_index(&mut pos, sc, c"a".as_ptr()),
            -1
        );
        assert_eq!(pos, 42);
        assert!(!errors::exists());

        let p = (&raw const calls).cast_mut().cast::<c_void>();
        assert_eq!(
            ffi::git_sortedcache_copy(&mut copy, sc, true, Some(failing_copy), p),
            -7
        );
        assert!(copy.is_null());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        ffi::git_sortedcache_free(sc);
    }
}

#[test]
fn upsert_under_alloc_limit() {
    let g = test_support::exclusive();
    let mut sc: *const Shared = core::ptr::null();
    let mut item: *mut c_void = core::ptr::without_provenance_mut::<c_void>(1);
    // SAFETY: as in `name_only`.
    unsafe {
        assert_eq!(
            ffi::git_sortedcache_new(
                &mut sc,
                0,
                None,
                core::ptr::null_mut(),
                Some(name_cmp),
                core::ptr::null()
            ),
            0
        );
        assert_eq!(ffi::git_sortedcache_wlock(sc), 0);
        test_support::with_alloc_limit(&g, 0, || {
            assert_eq!(
                ffi::git_sortedcache_upsert(&mut item, sc, c"x".as_ptr()),
                -1
            );
        });
        assert!(item.is_null());
        assert_eq!(ffi::git_sortedcache_entrycount(sc), 0);
        ffi::git_sortedcache_wunlock(sc);
        ffi::git_sortedcache_free(sc);
    }
}

// ----- the typed API -----

#[derive(Clone, Debug)]
struct Counted(u64, Arc<AtomicUsize>);
impl Drop for Counted {
    fn drop(&mut self) {
        self.1.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn typed_api() {
    let _g = test_support::init();
    let drops = Arc::new(AtomicUsize::new(0));
    let cache = SortedCache::<Counted>::new(None).unwrap();
    {
        let mut w = cache.write();
        for (i, k) in ["refs/heads/main", "refs/tags/v1", "refs/heads/dev", "HEAD"]
            .iter()
            .enumerate()
        {
            let v = w
                .upsert_with(k.as_bytes(), || Counted(i as u64, drops.clone()))
                .unwrap();
            v.0 += 100;
        }
        // an existing key keeps its value; `init` is not called
        assert_eq!(w.upsert_with(b"HEAD", || unreachable!()).unwrap().0, 103);
        w.get_mut(b"refs/tags/v1").unwrap().0 = 7;
    }
    assert_eq!(cache.len(), 4);
    {
        let r = cache.read();
        let keys: Vec<_> = r
            .iter()
            .map(|(k, v)| (String::from_utf8(k.to_vec()).unwrap(), v.0))
            .collect();
        assert_eq!(
            keys,
            [
                ("HEAD".to_string(), 103),
                ("refs/heads/dev".to_string(), 102),
                ("refs/heads/main".to_string(), 100),
                ("refs/tags/v1".to_string(), 7)
            ]
        );
        assert_eq!(r.index_of(b"refs/heads/main"), Some(2));
        assert_eq!(r.index_of(b"refs/heads/zzz"), None);
        assert!(r.get(b"nope").is_none());
    }
    let copy = cache.try_clone().unwrap();
    {
        let mut w = cache.write();
        w.remove(0).unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(w.remove(10).is_err());
        w.clear();
        assert_eq!(drops.load(Ordering::SeqCst), 4);
    }
    assert_eq!(copy.read().get(b"HEAD").unwrap().0, 103);
    let shared = copy.clone();
    drop(copy);
    assert_eq!(drops.load(Ordering::SeqCst), 4);
    drop(shared);
    assert_eq!(drops.load(Ordering::SeqCst), 8);
}

/// Fewer under Miri, which is slow.
const PER_THREAD: u64 = if cfg!(miri) { 8 } else { 200 };

#[test]
fn typed_api_threads() {
    let _g = test_support::init();
    let cache = SortedCache::<u64>::new(None).unwrap();
    std::thread::scope(|s| {
        for t in 0..4u64 {
            let cache = cache.clone();
            s.spawn(move || {
                for i in 0..PER_THREAD {
                    {
                        let mut w = cache.write();
                        let k = format!("t{t}/{i:04}");
                        *w.upsert_with(k.as_bytes(), || 0).unwrap() += i;
                    }
                    let r = cache.read();
                    let n = r.len();
                    let mut prev: Option<Vec<u8>> = None;
                    for (k, _) in r.iter().take(16) {
                        if let Some(p) = &prev {
                            assert!(p.as_slice() < k);
                        }
                        prev = Some(k.to_vec());
                    }
                    assert!(n >= 1);
                }
            });
        }
    });
    assert_eq!(cache.len(), 4 * PER_THREAD as usize);
    let last = format!("t3/{:04}", PER_THREAD - 1);
    assert_eq!(*cache.read().get(last.as_bytes()).unwrap(), PER_THREAD - 1);
}

#[test]
#[cfg_attr(miri, ignore = "futils is C until its WP lands")]
fn lock_and_load_on_disk() {
    if !cfg!(feature = "link-c") {
        return;
    }
    let _g = test_support::init();
    let dir = std::env::temp_dir().join(format!("u4-sortedcache-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cacheitems.txt");
    let cache = SortedCache::<i64>::new(Some(path.to_str().unwrap().as_bytes())).unwrap();

    // missing file: GIT_ENOTFOUND, unlocked
    let e = cache.lock_and_load().err().unwrap();
    assert_eq!(e.code(), ErrorCode::NOT_FOUND);

    std::fs::write(&path, "10 abc\n20 bcd\n30 cde\n").unwrap();
    let reload = |cache: &SortedCache<i64>| {
        let Some((mut w, data)) = cache.lock_and_load().unwrap() else {
            return false;
        };
        w.clear();
        for line in core::str::from_utf8(&data).unwrap().lines() {
            let (v, k) = line.split_once(' ').unwrap();
            *w.upsert_with(k.as_bytes(), || 0).unwrap() = v.parse().unwrap();
        }
        true
    };
    assert!(reload(&cache));
    assert_eq!(*cache.read().get(b"bcd").unwrap(), 20);
    assert!(!reload(&cache), "unchanged: not reloaded");

    std::fs::write(&path, "100 abc\n5 zzz\n").unwrap();
    assert!(reload(&cache));
    {
        let r = cache.read();
        assert_eq!(r.len(), 2);
        assert_eq!(
            r.entry(1).map(|(k, v)| (k.to_vec(), *v)),
            Some((b"zzz".to_vec(), 5))
        );
    }
    let mut w = cache.write();
    w.updated();
    drop(w);
    std::fs::remove_dir_all(&dir).unwrap();
}

// ----- the C ABI from several threads, as refdb_fs uses it (review U4-1,
// U4-2); Miri's race detector checks these -----

struct Sc(*const Shared);
// SAFETY: the cache is shared across threads as the C does, under its lock.
unsafe impl Send for Sc {}
// SAFETY: as above.
unsafe impl Sync for Sc {}

fn new_cache(cmp: bool) -> Sc {
    let mut sc: *const Shared = core::ptr::null();
    let f: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> c_int> =
        if cmp { Some(item_cmp) } else { None };
    // SAFETY: valid arguments.
    let r = unsafe {
        ffi::git_sortedcache_new(
            &mut sc,
            PATH_OFFSET,
            None,
            core::ptr::null_mut(),
            f,
            core::ptr::null(),
        )
    };
    assert_eq!(r, 0);
    Sc(sc)
}

const ROUNDS: usize = if cfg!(miri) { 12 } else { 400 };

/// A writer (wlock, upsert, wunlock) against `copy(lock = 1)`, as a refdb
/// iterator copying the refcache while another thread reloads it.
#[test]
fn c_abi_copy_races_with_writer() {
    let _g = test_support::init();
    let sc = new_cache(true);
    let sc = &sc;
    std::thread::scope(|s| {
        s.spawn(move || {
            for i in 0..ROUNDS {
                let key = std::ffi::CString::new(format!("refs/heads/k{i:04}")).unwrap();
                let mut item: *mut c_void = core::ptr::null_mut();
                // SAFETY: the write lock is held around the upsert.
                unsafe {
                    assert_eq!(ffi::git_sortedcache_wlock(sc.0), 0);
                    assert_eq!(
                        ffi::git_sortedcache_upsert(&mut item, sc.0, key.as_ptr()),
                        0
                    );
                    (*item.cast::<Item>()).value = i as c_int;
                    ffi::git_sortedcache_wunlock(sc.0);
                }
            }
        });
        for _ in 0..2 {
            s.spawn(move || {
                for _ in 0..ROUNDS / 2 {
                    let mut out: *const Shared = core::ptr::null();
                    // SAFETY: copy takes the read lock; the copy is ours.
                    unsafe {
                        assert_eq!(
                            ffi::git_sortedcache_copy(
                                &mut out,
                                sc.0,
                                true,
                                None,
                                core::ptr::null_mut()
                            ),
                            0
                        );
                        let n = ffi::git_sortedcache_entrycount(out);
                        let mut prev: Option<String> = None;
                        for i in 0..n {
                            let k =
                                key(ffi::git_sortedcache_entry(out, i), PATH_OFFSET).to_string();
                            if let Some(p) = &prev {
                                assert!(*p < k);
                            }
                            prev = Some(k);
                        }
                        ffi::git_sortedcache_free(out);
                    }
                }
            });
        }
    });
    // SAFETY: the last reference.
    unsafe {
        assert_eq!(ffi::git_sortedcache_entrycount(sc.0), ROUNDS);
        ffi::git_sortedcache_free(sc.0);
    }
}

/// Readers calling `entry`, `lookup` and `lookup_index` under read locks
/// concurrently, with and without a comparison (a cache without one is
/// never flagged sorted: readers must not sort it).
#[test]
fn c_abi_concurrent_readers() {
    let _g = test_support::init();
    for with_cmp in [false, true] {
        let sc = new_cache(with_cmp);
        // SAFETY: the write lock is held around the upserts.
        unsafe {
            assert_eq!(ffi::git_sortedcache_wlock(sc.0), 0);
            for k in [c"b", c"a", c"d", c"c"] {
                let mut item: *mut c_void = core::ptr::null_mut();
                assert_eq!(ffi::git_sortedcache_upsert(&mut item, sc.0, k.as_ptr()), 0);
            }
            ffi::git_sortedcache_wunlock(sc.0);
        }
        let sc = &sc;
        std::thread::scope(|s| {
            for _ in 0..3 {
                s.spawn(move || {
                    // SAFETY: each thread holds a read lock while reading.
                    unsafe {
                        assert_eq!(ffi::git_sortedcache_rlock(sc.0), 0);
                        for _ in 0..ROUNDS / 20 + 1 {
                            let order: Vec<_> = (0..4)
                                .map(|i| key(ffi::git_sortedcache_entry(sc.0, i), PATH_OFFSET))
                                .collect();
                            if with_cmp {
                                assert_eq!(order, ["a", "b", "c", "d"]);
                            } else {
                                assert_eq!(order, ["b", "a", "d", "c"]);
                            }
                            assert!(!ffi::git_sortedcache_lookup(sc.0, c"c".as_ptr()).is_null());
                            let mut pos = 0usize;
                            let r =
                                ffi::git_sortedcache_lookup_index(&mut pos, sc.0, c"c".as_ptr());
                            if with_cmp {
                                assert_eq!((r, pos), (0, 2));
                            } else {
                                assert_eq!(r, -1);
                            }
                        }
                        ffi::git_sortedcache_runlock(sc.0);
                    }
                });
            }
        });
        // SAFETY: the last reference.
        unsafe { ffi::git_sortedcache_free(sc.0) };
    }
}
