//! The C ABI of odb.c, odb_mempack.c and cache.c, called as C calls it:
//! raw pointers, a backend table written "in C" (here, `extern "C"` Rust
//! functions over a `#[repr(C)]` struct), several threads at once. Runs
//! under Miri (nothing here reaches the C).

use core::ffi::{c_char, c_int, c_void};
use core::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

use libgit2_rs::cache::{self, GitCache, GitCachedObj};
use libgit2_rs::odb::ffi::*;
use libgit2_rs::odb::{GitOdb, GitOdbBackend, GitOdbObject, GitOdbStream};
use libgit2_rs::odb_mempack::ffi::*;
use libgit2_rs::util::alloc;
use libgit2_rs::util::errors;
use libgit2_rs::util::test_support;
use libgit2_rs::{Oid, OidType};

const HELLO: &str = "ce013625030ba8dba906f756967f9e9ca394464a";

fn oid(s: &str) -> Oid {
    Oid::from_str_hex(s, OidType::Sha1).unwrap()
}

fn last_message() -> String {
    String::from_utf8(errors::last().1).unwrap()
}

/// A backend as a C user writes one: the table first, then its state.
#[repr(C)]
struct FakeBackend {
    parent: GitOdbBackend,
    name: u8,
    reads: AtomicUsize,
    frees: *const AtomicUsize,
}

unsafe extern "C" fn fake_read(
    buf: *mut *mut c_void,
    len: *mut usize,
    t: *mut c_int,
    b: *mut GitOdbBackend,
    id: *const Oid,
) -> c_int {
    let fake = b.cast::<FakeBackend>();
    unsafe { (*fake).reads.fetch_add(1, Ordering::SeqCst) };
    if unsafe { *id } != oid(HELLO) {
        return -3;
    }
    let p = unsafe { git_odb_backend_data_alloc(b, 6) }.cast::<u8>();
    unsafe { ptr::copy_nonoverlapping(b"hello\n".as_ptr(), p, 6) };
    unsafe {
        *buf = p.cast();
        *len = 6;
        *t = 3;
    }
    0
}

unsafe extern "C" fn fake_free(b: *mut GitOdbBackend) {
    let fake = b.cast::<FakeBackend>();
    unsafe { (*(*fake).frees).fetch_add(1, Ordering::SeqCst) };
    drop(unsafe { Box::from_raw(fake) });
}

fn fake(name: u8, frees: &'static AtomicUsize) -> *mut GitOdbBackend {
    let mut parent = GitOdbBackend::default();
    assert_eq!(unsafe { git_odb_init_backend(&raw mut parent, 1) }, 0);
    parent.read = Some(fake_read);
    parent.free = Some(fake_free);
    Box::into_raw(Box::new(FakeBackend {
        parent,
        name,
        reads: AtomicUsize::new(0),
        frees,
    }))
    .cast()
}

#[test]
fn backends_sorted_by_priority() {
    let _g = test_support::init();
    static FREES: AtomicUsize = AtomicUsize::new(0);
    let mut db: *mut GitOdb = ptr::null_mut();
    assert_eq!(unsafe { git_odb_new(&raw mut db) }, 0);
    let prios = [(b'a', 1, false), (b'b', 3, false), (b'c', 2, true), (b'd', 2, false), (b'e', 3, true)];
    for (n, p, alt) in prios {
        let b = fake(n, &FREES);
        let r = if alt {
            unsafe { git_odb_add_alternate(db, b, p) }
        } else {
            unsafe { git_odb_add_backend(db, b, p) }
        };
        assert_eq!(r, 0);
        // Its owner is set.
        assert_eq!(unsafe { (*b).odb }, db);
    }
    assert_eq!(unsafe { git_odb_num_backends(db) }, 5);
    let mut order = Vec::new();
    for i in 0..5 {
        let mut b = ptr::null_mut();
        assert_eq!(unsafe { git_odb_get_backend(&raw mut b, db, i) }, 0);
        order.push(unsafe { (*b.cast::<FakeBackend>()).name });
    }
    // C's timsort with `backend_sort_cmp`: priority down, alternates first.
    assert_eq!(order, b"ebcda");
    let mut b = ptr::null_mut();
    assert_eq!(unsafe { git_odb_get_backend(&raw mut b, db, 5) }, -3);
    assert_eq!(last_message(), "no ODB backend loaded at index 5");

    // The first backend that has it answers.
    let mut obj: *mut GitOdbObject = ptr::null_mut();
    let id = oid(HELLO);
    assert_eq!(unsafe { git_odb_read(&raw mut obj, db, &id) }, 0);
    assert_eq!(unsafe { git_odb_object_size(obj) }, 6);
    assert_eq!(unsafe { git_odb_object_type(obj) }, 3);
    let data = unsafe { core::slice::from_raw_parts(git_odb_object_data(obj).cast::<u8>(), 6) };
    assert_eq!(data, b"hello\n");
    let mut dup = ptr::null_mut();
    assert_eq!(unsafe { git_odb_object_dup(&raw mut dup, obj) }, 0);
    assert_eq!(dup, obj);
    unsafe { git_odb_object_free(obj) };
    unsafe { git_odb_object_free(dup) };

    // A backend of another odb is refused.
    let mut db2: *mut GitOdb = ptr::null_mut();
    assert_eq!(unsafe { git_odb_new(&raw mut db2) }, 0);
    let mut b0 = ptr::null_mut();
    assert_eq!(unsafe { git_odb_get_backend(&raw mut b0, db, 0) }, 0);
    assert_eq!(unsafe { git_odb_add_backend(db2, b0, 1) }, -1);
    assert_eq!(
        last_message(),
        "unrecoverable internal error: '!backend->odb || backend->odb == odb'"
    );
    unsafe { git_odb_free(db2) };

    unsafe { git_odb_free(db) };
    assert_eq!(FREES.load(Ordering::SeqCst), 5);
}

#[test]
fn argument_checks_and_versions() {
    let _g = test_support::init();
    assert_eq!(unsafe { git_odb_new(ptr::null_mut()) }, -1);
    assert_eq!(last_message(), "invalid argument: 'out'");
    let mut db: *mut GitOdb = ptr::null_mut();
    let bad = libgit2_rs::odb::GitOdbOptions { version: 7, oid_type: 0 };
    assert_eq!(unsafe { git_odb_new_ext(&raw mut db, &bad) }, -1);
    assert_eq!(last_message(), "invalid version 7 on git_odb_options");
    let mut be = GitOdbBackend::default();
    assert_eq!(unsafe { git_odb_init_backend(&raw mut be, 2) }, -1);
    assert_eq!(last_message(), "invalid version 2 on git_odb_backend");

    assert_eq!(unsafe { git_odb_new(&raw mut db) }, 0);
    assert_eq!(unsafe { git_odb_add_backend(db, &raw mut be, 1) }, -1);
    assert_eq!(last_message(), "invalid version 0 on git_odb_backend");
    let mut out = Oid::NONE;
    assert_eq!(unsafe { git_odb_write(&raw mut out, db, b"x".as_ptr().cast(), 1, 3) }, -1);
    assert_eq!(
        last_message(),
        "cannot write object - unsupported in the loaded odb backends"
    );
    let mut s: *mut GitOdbStream = ptr::null_mut();
    assert_eq!(unsafe { git_odb_open_wstream(&raw mut s, db, 1, 3) }, -1);
    unsafe { git_odb_free(db) };
}

#[test]
fn object_headers() {
    let _g = test_support::init();
    let mut hdr = [0x55u8; 64];
    let mut n = 0usize;
    let r = unsafe { git_odb__format_object_header(&raw mut n, hdr.as_mut_ptr().cast::<c_char>(), 64, 12345, 1) };
    assert_eq!((r, n), (0, 13));
    assert_eq!(&hdr[..13], b"commit 12345\0");
    // An unknown type has an empty name, as `git_object_type2string`.
    let r = unsafe { git_odb__format_object_header(&raw mut n, hdr.as_mut_ptr().cast::<c_char>(), 64, 1, 5) };
    assert_eq!((r, n), (0, 3));
    assert_eq!(&hdr[..3], b" 1\0");
    // Too small: snprintf's truncation, and the error.
    let mut small = [0x55u8; 6];
    let r = unsafe { git_odb__format_object_header(&raw mut n, small.as_mut_ptr().cast::<c_char>(), 6, 1234, 3) };
    assert_eq!(r, -1);
    assert_eq!(&small, b"blob \0");
    assert!(last_message().starts_with("object header creation failed"));
    // A size above i64::MAX prints as C's `(int64_t)` cast.
    let r = unsafe { git_odb__format_object_header(&raw mut n, hdr.as_mut_ptr().cast::<c_char>(), 64, u64::MAX, 3) };
    assert_eq!(r, 0);
    assert_eq!(&hdr[..n], b"blob -1\0");
}

#[test]
fn error_helpers() {
    let _g = test_support::init();
    let id = oid(HELLO);
    assert_eq!(unsafe { git_odb__error_notfound(c"no match".as_ptr(), &id, 7) }, -3);
    assert_eq!(last_message(), "object not found - no match (ce01362)");
    assert_eq!(unsafe { git_odb__error_notfound(c"plain".as_ptr(), ptr::null(), 0) }, -3);
    assert_eq!(last_message(), "object not found - plain");
    assert_eq!(unsafe { git_odb__error_ambiguous(c"x".as_ptr()) }, -5);
    assert_eq!(last_message(), "ambiguous OID prefix - x");
    let other = oid("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
    assert_eq!(unsafe { git_odb__error_mismatch(&id, &other) }, -33);
    assert_eq!(
        last_message(),
        format!("object hash mismatch - expected {HELLO} but got e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")
    );
}

#[test]
fn mempack_c_abi_from_threads() {
    let _g = test_support::init();
    let mut db: *mut GitOdb = ptr::null_mut();
    assert_eq!(unsafe { git_odb_new(&raw mut db) }, 0);
    let mut mp: *mut GitOdbBackend = ptr::null_mut();
    assert_eq!(unsafe { git_mempack_new(&raw mut mp) }, 0);
    assert_eq!(unsafe { git_odb_add_backend(db, mp, 1) }, 0);
    struct P(*mut GitOdb, *mut GitOdbBackend);
    unsafe impl Send for P {}
    unsafe impl Sync for P {}
    let p = P(db, mp);
    std::thread::scope(|s| {
        for t in 0..3u8 {
            let p = &p;
            s.spawn(move || {
                for i in 0..10u8 {
                    let data = [t, i, b'\n'];
                    let mut id = Oid::NONE;
                    assert_eq!(unsafe { git_odb_write(&raw mut id, p.0, data.as_ptr().cast(), 3, 1) }, 0);
                    let mut obj = ptr::null_mut();
                    assert_eq!(unsafe { git_odb_read(&raw mut obj, p.0, &id) }, 0);
                    assert_eq!(unsafe { git_odb_object_size(obj) }, 3);
                    unsafe { git_odb_object_free(obj) };
                    assert_eq!(unsafe { git_odb_exists(p.0, &id) }, 1);
                    let mut n = 0;
                    assert_eq!(unsafe { git_mempack_object_count(&raw mut n, p.1) }, 0);
                    assert!(n >= 1);
                }
            });
        }
    });
    let mut n = 0;
    assert_eq!(unsafe { git_mempack_object_count(&raw mut n, mp) }, 0);
    assert_eq!(n, 30);
    assert_eq!(unsafe { git_mempack_reset(mp) }, 0);
    assert_eq!(unsafe { git_mempack_object_count(&raw mut n, mp) }, 0);
    assert_eq!(n, 0);
    assert_eq!(unsafe { git_mempack_object_count(&raw mut n, ptr::null_mut()) }, -1);
    assert_eq!(last_message(), "invalid argument: '_backend'");
    unsafe { git_odb_free(db) };
}

/// A cached object of C's kind 0 (plain memory), as C would make one.
fn plain(i: u8, size: usize) -> *mut GitCachedObj {
    let p = alloc::calloc(1, size_of::<GitCachedObj>()).cast::<GitCachedObj>();
    let mut id = [0u8; 32];
    id[0] = i;
    id[1] = i.wrapping_mul(7);
    unsafe {
        p.write(GitCachedObj {
            oid: Oid::from_raw_parts(1, id),
            type_: 1,
            flags: 0,
            size,
            refcount: Default::default(),
        })
    };
    p
}

#[test]
fn cache_c_abi_concurrently() {
    use libgit2_rs::cache::ffi::*;
    let _g = test_support::init();
    let c = Box::into_raw(Box::new(core::mem::MaybeUninit::<GitCache>::uninit())).cast::<GitCache>();
    assert_eq!(unsafe { git_cache_init(c) }, 0);
    struct P(*mut GitCache);
    unsafe impl Send for P {}
    unsafe impl Sync for P {}
    let p = P(c);
    std::thread::scope(|s| {
        for t in 0..3u8 {
            let p = &p;
            s.spawn(move || {
                for i in 0..8u8 {
                    let e = plain(t * 16 + i, 10);
                    // `flags` 0: the cache frees it as plain memory.
                    let r = unsafe { cache::GitCache::store(&*p.0, core::ptr::NonNull::new(e).unwrap()) };
                    let id = unsafe { (*e).oid };
                    let g = unsafe { git_cache_get_any(p.0, &id) };
                    assert!(!g.is_null());
                    unsafe { git_cached_obj_decref(g) };
                    unsafe { git_cached_obj_decref(r.as_ptr().cast()) };
                    let _ = unsafe { git_cache_size(p.0) };
                }
            });
        }
    });
    assert_eq!(unsafe { git_cache_size(c) }, 24);
    unsafe { git_cache_clear(c) };
    assert_eq!(unsafe { git_cache_size(c) }, 0);
    unsafe { git_cache_dispose(c) };
    drop(unsafe { Box::from_raw(c.cast::<core::mem::MaybeUninit<GitCache>>()) });
    assert_eq!(unsafe { git_cache_set_max_object_size(9, 1) }, -1);
    assert_eq!(last_message(), "type out of range");
}
