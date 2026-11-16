//! The exports of work package u1, called as C calls them (raw pointers,
//! uninitialized out-parameters, C strings).

// These tests call the exports in sequences, as C does; the SAFETY comment
// of each block covers the calls in it.
#![allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]

use core::ffi::{CStr, c_char, c_int, c_void};
use core::mem::MaybeUninit;
use core::ptr;

use libgit2_rs::oid::{GIT_OID_SHA1, GIT_OID_SHA256, ffi as oid_ffi};
use libgit2_rs::oidarray::{GitOidArray, ffi as oa_ffi};
use libgit2_rs::parse::{GitParseCtx, ffi as parse_ffi};
use libgit2_rs::util::array::GitArray;
use libgit2_rs::util::errors;
use libgit2_rs::util::hashmap::{CStrKey, GitHashMap};
use libgit2_rs::util::pool::{GitPool, ffi as pool_ffi};
use libgit2_rs::util::pqueue::{GIT_PQUEUE_FIXED_SIZE, GitPqueue, ffi as pq_ffi};
use libgit2_rs::util::test_support;
use libgit2_rs::util::varint::ffi as varint_ffi;
use libgit2_rs::{Oid, OidType};

const HEX: &CStr = c"8b137891791fe96927ad78e64b0aad7bded08bdc";

fn msg() -> Vec<u8> {
    errors::last_message().unwrap_or_default()
}

#[test]
fn oid_parse_exports() {
    let _g = test_support::init();
    // SAFETY: as C would call them.
    unsafe {
        let mut o = MaybeUninit::<Oid>::uninit();
        assert_eq!(
            oid_ffi::git_oid_from_string(o.as_mut_ptr(), HEX.as_ptr(), GIT_OID_SHA1),
            0
        );
        let o = o.assume_init();
        assert_eq!(o.to_string().as_bytes(), HEX.to_bytes());

        // a short C string: C stops at its NUL
        let mut p = Oid::NONE;
        assert_eq!(
            oid_ffi::git_oid_from_prefix(&mut p, c"abc".as_ptr(), 40, GIT_OID_SHA1),
            -1
        );
        assert_eq!(msg(), b"unable to parse OID - contains invalid characters");
        assert_eq!(&p.raw_id()[..2], &[0xab, 0xc0]);
        assert_eq!(
            oid_ffi::git_oid_from_prefix(&mut p, ptr::null(), 4, GIT_OID_SHA1),
            -1
        );
        assert_eq!(msg(), b"invalid argument: 'str'");
        assert_eq!(
            oid_ffi::git_oid_from_prefix(ptr::null_mut(), c"ab".as_ptr(), 2, 7),
            -1
        );
        assert_eq!(msg(), b"invalid argument: 'out'");
        assert_eq!(
            oid_ffi::git_oid_from_prefix(&mut p, c"ab".as_ptr(), 2, 7),
            -1
        );
        assert_eq!(msg(), b"unable to parse OID - unknown type");
        assert_eq!(oid_ffi::git_oid_from_string(&mut p, c"ab".as_ptr(), 0), -1);
        assert_eq!(msg(), b"unable to parse OID - unknown type");
        let long = c"8b137891791fe96927ad78e64b0aad7bded08bdc0";
        assert_eq!(
            oid_ffi::git_oid_from_string(&mut p, long.as_ptr(), GIT_OID_SHA1),
            -1
        );
        assert_eq!(msg(), b"unable to parse OID - too long");

        let raw = [7u8; 32];
        assert_eq!(
            oid_ffi::git_oid_from_raw(&mut p, raw.as_ptr(), GIT_OID_SHA256),
            0
        );
        assert_eq!(p.kind(), Some(OidType::Sha256));
        assert_eq!(oid_ffi::git_oid_from_raw(&mut p, raw.as_ptr(), 3), -1);
    }
}

#[test]
fn oid_format_exports() {
    let _g = test_support::init();
    let o = Oid::from_hex(HEX.to_bytes(), OidType::Sha1).unwrap();
    // SAFETY: as C would call them.
    unsafe {
        let mut buf = [b'z' as c_char; 70];
        assert_eq!(oid_ffi::git_oid_fmt(buf.as_mut_ptr(), &o), 0);
        assert_eq!(buf[40], b'z' as c_char);
        assert_eq!(oid_ffi::git_oid_pathfmt(buf.as_mut_ptr(), &o), 0);
        assert_eq!(buf[2], b'/' as c_char);
        assert_eq!(buf[41], b'z' as c_char);
        assert_eq!(oid_ffi::git_oid_nfmt(buf.as_mut_ptr(), 45, &o), 0);
        assert_eq!(buf[40..45], [0; 5]);
        assert_eq!(oid_ffi::git_oid_nfmt(buf.as_mut_ptr(), 3, ptr::null()), 0);
        assert_eq!(buf[..3], [0; 3]);

        let s = oid_ffi::git_oid_tostr(buf.as_mut_ptr(), 8, &o);
        assert_eq!(CStr::from_ptr(s), c"8b13789");
        assert_eq!(
            CStr::from_ptr(oid_ffi::git_oid_tostr(ptr::null_mut(), 8, &o)),
            c""
        );
        assert_eq!(
            CStr::from_ptr(oid_ffi::git_oid_tostr(buf.as_mut_ptr(), 8, ptr::null())),
            c""
        );

        let s1 = oid_ffi::git_oid_tostr_s(&o);
        assert_eq!(CStr::from_ptr(s1), HEX);
        let s2 = oid_ffi::git_oid_tostr_s(&Oid::EMPTY_BLOB_SHA1);
        assert_eq!(s1, s2, "one buffer per thread");
        let other = std::thread::spawn(move || oid_ffi::git_oid_tostr_s(&o) as usize)
            .join()
            .unwrap();
        assert_ne!(other, s1 as usize);

        let a = oid_ffi::git_oid_allocfmt(&o);
        assert_eq!(CStr::from_ptr(a), HEX);
        libgit2_rs::util::alloc::free(a.cast());
        let bad = Oid::from_raw_parts(9, [0; 32]);
        assert!(oid_ffi::git_oid_allocfmt(&bad).is_null());
        assert_eq!(oid_ffi::git_oid_fmt(buf.as_mut_ptr(), &bad), -1);
        assert_eq!(msg(), b"unable to parse OID - unknown type");
    }
}

#[test]
fn oid_compare_and_shorten_exports() {
    let _g = test_support::init();
    let a = Oid::from_hex(HEX.to_bytes(), OidType::Sha1).unwrap();
    let mut b = Oid::NONE;
    // SAFETY: as C would call them.
    unsafe {
        assert_eq!(oid_ffi::git_oid_cpy(&mut b, &a), 0);
        assert_eq!(oid_ffi::git_oid_equal(&a, &b), 1);
        assert_eq!(oid_ffi::git_oid_cpy(&mut b, &b), 0);
        b.raw_id_mut()[19] = 0xdf;
        assert_eq!(oid_ffi::git_oid_cmp(&a, &b), 0xdc - 0xdf);
        assert_eq!(oid_ffi::git_oid_ncmp(&a, &b, 38), 0);
        assert_eq!(oid_ffi::git_oid_ncmp(&a, &b, 40), 1);
        assert_eq!(oid_ffi::git_oid_streq(&a, c"8b1378".as_ptr()), 0);
        assert_eq!(oid_ffi::git_oid_streq(&a, c"8b1379".as_ptr()), -1);
        assert_eq!(oid_ffi::git_oid_strcmp(&a, c"8b1g".as_ptr()), -1);
        assert_eq!(oid_ffi::git_oid_is_zero(&Oid::SHA1_ZERO), 1);
        assert_eq!(oid_ffi::git_oid__empty_tree_sha1, Oid::EMPTY_TREE_SHA1);

        let os = oid_ffi::git_oid_shorten_new(0);
        assert_eq!(oid_ffi::git_oid_shorten_add(os, HEX.as_ptr()), 1);
        assert_eq!(oid_ffi::git_oid_shorten_add(os, c"8b1z".as_ptr()), -1);
        assert_eq!(msg(), b"unable to shorten OID - invalid hex value");
        assert_eq!(oid_ffi::git_oid_shorten_add(os, ptr::null()), 1);
        oid_ffi::git_oid_shorten_free(os);
        oid_ffi::git_oid_shorten_free(ptr::null_mut());
        assert!(oid_ffi::git_oid_shorten_new(1 << 40).is_null());
        assert_eq!(
            msg(),
            b"invalid argument: '(size_t)((int)min_length) == min_length'"
        );
    }
}

#[test]
fn oidarray_exports() {
    let _g = test_support::init();
    let ids: Vec<Oid> = (1..=3u8).map(|i| Oid::from_sha1([i; 20])).collect();
    // SAFETY: as C would call them.
    unsafe {
        let mut arr: MaybeUninit<GitArray<Oid>> = MaybeUninit::zeroed();
        for id in ids.iter().chain(&ids) {
            assert_eq!(oa_ffi::git_oidarray__add(arr.as_mut_ptr(), id), 0);
        }
        assert!(oa_ffi::git_oidarray__remove(arr.as_mut_ptr(), &ids[0]));
        let mut out = MaybeUninit::<GitOidArray>::uninit();
        oa_ffi::git_oidarray__from_array(out.as_mut_ptr(), arr.as_ptr());
        let mut out = out.assume_init();
        assert_eq!(out.count, 2);
        oa_ffi::git_oidarray__reverse(&mut out);
        assert_eq!(*out.ids, ids[2]);
        oa_ffi::git_oidarray__to_array(arr.as_mut_ptr(), &out);
        assert_eq!(arr.assume_init_ref().capacity(), 2);
        // the buffer is shared: dispose it once
        oa_ffi::git_oidarray_dispose(&mut out);
    }
}

#[test]
fn pool_exports() {
    let _g = test_support::init();
    // SAFETY: as C would call them; the pool is initialized over garbage.
    unsafe {
        let mut p = MaybeUninit::<GitPool>::uninit();
        ptr::write_bytes(p.as_mut_ptr(), 0xa5, 1);
        assert_eq!(pool_ffi::git_pool_init(p.as_mut_ptr(), 1), 0);
        let p = p.as_mut_ptr();
        let s = pool_ffi::git_pool_strndup(p, c"abcdef".as_ptr(), 3);
        assert_eq!(CStr::from_ptr(s), c"abc");
        let s = pool_ffi::git_pool_strcat(p, ptr::null(), c"x".as_ptr());
        assert_eq!(CStr::from_ptr(s), c"x");
        assert!(pool_ffi::git_pool_strdup_safe(p, ptr::null()).is_null());
        assert!(pool_ffi::git_pool_strndup(p, c"a".as_ptr(), usize::MAX).is_null());
        assert!(pool_ffi::git_pool__ptr_in_pool(p, s.cast()));
        let z = pool_ffi::git_pool_mallocz(p, 16).cast::<u8>();
        assert_eq!(*z.add(15), 0);
        let mut q = GitPool::new();
        pool_ffi::git_pool_swap(p, &mut q);
        assert_eq!(pool_ffi::git_pool__open_pages(p), 0);
        assert_eq!(q.open_pages(), 1);
        pool_ffi::git_pool_swap(p, p);
        pool_ffi::git_pool_clear(&mut q);
        assert_eq!(pool_ffi::git_pool_init(p, 0), -1);
        assert_eq!(msg(), b"invalid argument: 'item_size >= 1'");
        assert_eq!(pool_ffi::git_pool_init(ptr::null_mut(), 1), -1);
        assert_eq!(msg(), b"invalid argument: 'pool'");
        let mut items = GitPool::with_item_size(4).unwrap();
        assert!(pool_ffi::git_pool_strdup(&mut items, c"x".as_ptr()).is_null());
        assert_eq!(
            msg(),
            b"invalid argument: 'pool->item_size == sizeof(char)'"
        );
    }
}

unsafe extern "C" fn cmp_int(a: *const c_void, b: *const c_void) -> c_int {
    // SAFETY: the elements point to `c_int`s.
    unsafe { *a.cast::<c_int>() - *b.cast::<c_int>() }
}

#[test]
fn pqueue_exports() {
    let _g = test_support::init();
    let mut vals: Vec<c_int> = (0..100).map(|i| (i * 37) % 101).collect();
    // SAFETY: as C would call them; the elements outlive the queue.
    unsafe {
        let mut pq = MaybeUninit::<GitPqueue>::uninit();
        assert_eq!(
            pq_ffi::git_pqueue_init(pq.as_mut_ptr(), GIT_PQUEUE_FIXED_SIZE, 10, Some(cmp_int)),
            0
        );
        let pq = pq.assume_init_mut();
        for v in vals.iter_mut() {
            assert_eq!(pq_ffi::git_pqueue_insert(pq, (v as *mut c_int).cast()), 0);
        }
        assert_eq!(pq.len(), 10);
        let mut got = vec![];
        loop {
            let p = pq_ffi::git_pqueue_pop(pq);
            if p.is_null() {
                break;
            }
            got.push(*p.cast::<c_int>());
        }
        let mut want: Vec<c_int> = (0..100).map(|i| (i * 37) % 101).collect();
        want.sort();
        assert_eq!(got, want[90..]);
        pq.as_vector_mut().dispose();
    }
}

#[test]
fn parse_exports() {
    let _g = test_support::init();
    let text = b"12 x\n8b137891791fe96927ad78e64b0aad7bded08bdc\n";
    // SAFETY: as C would call them; `text` outlives the context.
    unsafe {
        let mut c = MaybeUninit::<GitParseCtx<'static>>::uninit();
        assert_eq!(
            parse_ffi::git_parse_ctx_init(c.as_mut_ptr(), text.as_ptr().cast(), text.len()),
            0
        );
        let c = c.as_mut_ptr();
        let mut n = 0i64;
        assert_eq!(parse_ffi::git_parse_advance_digit(&mut n, c, 10), 0);
        assert_eq!(n, 12);
        assert_eq!(parse_ffi::git_parse_advance_ws(c), 0);
        let mut ch: c_char = 0;
        assert_eq!(parse_ffi::git_parse_peek(&mut ch, c, 0), 0);
        assert_eq!(ch, b'x' as c_char);
        assert_eq!(
            parse_ffi::git_parse_advance_expected(c, c"x".as_ptr(), 1),
            0
        );
        assert_eq!(parse_ffi::git_parse_advance_nl(c), 0);
        let mut o = Oid::NONE;
        assert_eq!(parse_ffi::git_parse_advance_oid(&mut o, c, GIT_OID_SHA1), 0);
        assert_eq!(o.to_string().as_bytes(), HEX.to_bytes());
        assert_eq!(parse_ffi::git_parse_advance_oid(&mut o, c, 0), -1);
        assert_eq!(msg(), b"unrecoverable internal error: 'oid_hexsize'");
        parse_ffi::git_parse_ctx_clear(c);
        assert_eq!((*c).content(), b"");
    }
}

#[test]
fn varint_exports() {
    let mut buf = [0u8; 4];
    // SAFETY: as C would call them.
    unsafe {
        assert_eq!(varint_ffi::git_encode_varint(buf.as_mut_ptr(), 4, 300), 2);
        assert_eq!(
            varint_ffi::git_encode_varint(ptr::null_mut(), 0, 1 << 40),
            6
        );
        assert_eq!(varint_ffi::git_encode_varint(buf.as_mut_ptr(), 1, 300), -1);
        let mut len = 0;
        assert_eq!(varint_ffi::git_decode_varint(buf.as_ptr(), &mut len), 300);
        assert_eq!(len, 2);
    }
}

// A C-named map family, as `pack.c`'s `git_pack_oidmap` will be exported.
type TestMap = GitHashMap<CStrKey<'static>, *mut c_void>;
libgit2_rs::git_hashmap_exports! {
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
    let _g = test_support::init();
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

#[test]
fn out_of_memory() {
    let g = test_support::exclusive();
    let (put, pool, class) = test_support::with_alloc_limit(&g, 0, || {
        let mut m: GitHashMap<CStrKey<'static>, u32> = GitHashMap::new();
        let put = m.put(CStrKey::new(c"k"), 1).map(|_| ());
        let empty = m.is_empty() && m.buckets() == 0;
        let mut p = GitPool::with_item_size(1).unwrap();
        let pool = p.malloc(8).is_err() && p.open_pages() == 0;
        (put.is_err() && empty, pool, errors::last_class())
    });
    assert!(put);
    assert!(pool);
    assert_eq!(class, Some(errors::ErrorClass::NOMEMORY));
}

/// A grow that fails after the keys were reallocated (the values' array
/// fails): the map keeps its entries and works afterwards.
#[test]
fn map_grow_fails_halfway() {
    let g = test_support::exclusive();
    let keys: Vec<std::ffi::CString> = (0..40)
        .map(|i| std::ffi::CString::new(format!("k{i}")).unwrap())
        .collect();
    let mut failures = 0;
    // 3 entries fit 4 buckets (flags 4 bytes, keys 32, values 32); the
    // 4th grows to 8: flags 4, keys 64, then values 64.
    for limit in 68..150usize {
        let failed = test_support::with_alloc_limit(&g, limit, || {
            let mut m: GitHashMap<CStrKey<'_>, u64> = GitHashMap::new();
            for (i, k) in keys.iter().take(3).enumerate() {
                m.put(CStrKey::new(k), i as u64).unwrap();
            }
            let before = m.buckets();
            let failed = m.put(CStrKey::new(&keys[3]), 3).is_err();
            if failed {
                assert_eq!((m.len(), m.buckets()), (3, before));
                for (i, k) in keys.iter().take(3).enumerate() {
                    assert_eq!(m.get(&CStrKey::new(k)), Some(&(i as u64)));
                }
                assert!(!m.contains(&CStrKey::new(&keys[3])));
            }
            failed
        });
        failures += failed as usize;
    }
    assert!(failures >= 2, "{failures}");
    // and a map that failed halfway grows later
    let mut m: GitHashMap<CStrKey<'_>, u64> = GitHashMap::new();
    for (i, k) in keys.iter().take(3).enumerate() {
        m.put(CStrKey::new(k), i as u64).unwrap();
    }
    let r = test_support::with_alloc_limit(&g, 0, || m.put(CStrKey::new(&keys[3]), 3).map(|_| ()));
    assert!(r.is_err());
    for (i, k) in keys.iter().enumerate() {
        m.put(CStrKey::new(k), i as u64).unwrap();
    }
    assert_eq!(m.len(), 40);
    assert_eq!(m.get(&CStrKey::new(&keys[39])), Some(&39));
}
