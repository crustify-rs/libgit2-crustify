//! The C ABI of the index (WP l-index), called as C would: argument checks
//! and their messages, NULL indexes (which the C dereferenced), the
//! iterators and snapshots, the writer, and the comparison functions C
//! sorts with.

#![allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]

use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr;

use libgit2_rs::index::ffi::*;
use libgit2_rs::index::{GitIndex, GitIndexEntry, GitIndexWriter};
use libgit2_rs::util::errors;
use libgit2_rs::util::test_support;
use libgit2_rs::util::vector::GitVector;
use libgit2_rs::{Oid, OidType};

fn last() -> String {
    String::from_utf8_lossy(&errors::last_message().unwrap_or_default()).into_owned()
}

fn new_index() -> *mut GitIndex {
    let mut ix = ptr::null_mut();
    // SAFETY: a writable out-pointer.
    assert_eq!(unsafe { git_index_new(&raw mut ix) }, 0);
    ix
}

fn add(ix: *mut GitIndex, path: &CStr, stage: c_int) -> c_int {
    let mut e = GitIndexEntry::zeroed();
    e.mode = 0o100644;
    e.id = Oid::from_hex(b"1111111111111111111111111111111111111111", OidType::Sha1).unwrap();
    e.path = path.as_ptr();
    e.set_stage(stage);
    // SAFETY: a live index and a valid entry.
    unsafe { git_index_add(ix, &raw const e) }
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn argument_checks() {
    let _g = test_support::init();
    // SAFETY: (throughout) the exports' contracts, with NULL where they
    // check it.
    unsafe {
        assert_eq!(git_index_entrycount(ptr::null()), usize::MAX);
        assert_eq!(last(), "invalid argument: 'index'");
        assert_eq!(git_index_version(ptr::null_mut()), u32::MAX);
        assert!(git_index_get_byindex(ptr::null_mut(), 0).is_null());
        assert_eq!(git_index_remove(ptr::null_mut(), c"x".as_ptr(), 0), -1);
        assert_eq!(git_index_open(ptr::null_mut(), ptr::null()), -1);
        assert_eq!(last(), "invalid argument: 'index_out'");
        let opts = libgit2_rs::index::GitIndexOptions {
            version: 7,
            oid_type: 0,
        };
        let mut ix = ptr::null_mut();
        assert_eq!(git_index_new_ext(&raw mut ix, &raw const opts), -1);
        assert_eq!(last(), "invalid version 7 on git_index_options");
        let opts = libgit2_rs::index::GitIndexOptions {
            version: 1,
            oid_type: 9,
        };
        assert_eq!(git_index_new_ext(&raw mut ix, &raw const opts), -1);
        assert_eq!(
            last(),
            "invalid argument: 'git_oid_type_is_valid(opts->oid_type)'"
        );

        let ix = new_index();
        assert_eq!(git_index_add(ix, ptr::null()), -1);
        assert_eq!(
            last(),
            "invalid argument: 'source_entry && source_entry->path'"
        );
        assert_eq!(
            git_index_name_add(ix, c"a".as_ptr(), ptr::null(), ptr::null()),
            -1
        );
        assert_eq!(
            last(),
            "invalid argument: '(ancestor && ours) || (ancestor && theirs) || (ours && theirs)'"
        );
        assert_eq!(git_index_set_version(ix, 1), -1);
        assert_eq!(last(), "invalid version number");
        assert_eq!(git_index_add_bypath(ix, c"f".as_ptr()), -1);
        assert_eq!(
            last(),
            "could not initialize index entry. Index is not backed up by an existing repository."
        );
        assert!(git_index_get_bypath(ix, c"nope".as_ptr(), 0).is_null());
        assert_eq!(last(), "index does not contain 'nope'");
        let mut pos = 99usize;
        assert_eq!(git_index_find(&raw mut pos, ix, c"nope".as_ptr()), -3);
        assert_eq!(pos, 99);
        assert_eq!(last(), "index does not contain nope");
        assert_eq!(
            git_index__find_pos(&raw mut pos, ix, c"nope".as_ptr(), 0, 0),
            -3
        );
        assert_eq!(pos, 0);
        git_index_free(ix);
        git_index_free(ptr::null_mut());
    }
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn iterators_and_snapshots() {
    let _g = test_support::init();
    let ix = new_index();
    for p in [c"c", c"a", c"b"] {
        assert_eq!(add(ix, p, 0), 0);
    }
    assert_eq!(add(ix, c"x", 2), 0);
    // SAFETY: (throughout) live objects of these exports.
    unsafe {
        let mut it = ptr::null_mut();
        assert_eq!(git_index_iterator_new(&raw mut it, ix), 0);
        // the snapshot outlives a clear: removals are deferred
        assert_eq!(git_index_clear(ix), 0);
        let mut names = Vec::new();
        let mut e = ptr::null();
        while git_index_iterator_next(&raw mut e, it) == 0 {
            names.push(CStr::from_ptr((*e).path).to_bytes().to_vec());
        }
        assert_eq!(
            names,
            vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec(), b"x".to_vec()]
        );
        assert_eq!(git_index_iterator_next(&raw mut e, it), -31);
        git_index_iterator_free(it);

        // a C-style snapshot, searched with the C's search function
        assert_eq!(add(ix, c"q/r", 0), 0);
        assert_eq!(add(ix, c"q/s", 0), 0);
        let mut snap = core::mem::MaybeUninit::<GitVector>::uninit();
        assert_eq!(git_index_snapshot_new(snap.as_mut_ptr(), ix), 0);
        let mut pos = 0usize;
        let srch: unsafe extern "C" fn(*const c_void, *const c_void) -> c_int =
            git_index_entry_srch;
        assert_eq!(
            git_index_snapshot_find(
                &raw mut pos,
                snap.as_mut_ptr(),
                Some(srch),
                c"q/s".as_ptr(),
                0,
                0
            ),
            0
        );
        assert_eq!(pos, 1);
        assert_eq!(
            git_index_snapshot_find(
                &raw mut pos,
                snap.as_mut_ptr(),
                Some(srch),
                c"q/".as_ptr(),
                2,
                -1
            ),
            -3
        );
        assert_eq!(pos, 0);
        git_index_snapshot_release(snap.as_mut_ptr(), ix);

        // conflicts
        let (mut a, mut o, mut t) = (ptr::null(), ptr::null(), ptr::null());
        let mut ci = ptr::null_mut();
        assert_eq!(add(ix, c"k", 1), 0);
        assert_eq!(add(ix, c"k", 3), 0);
        assert_eq!(git_index_conflict_iterator_new(&raw mut ci, ix), 0);
        assert_eq!(
            git_index_conflict_next(&raw mut a, &raw mut o, &raw mut t, ci),
            0
        );
        assert!(!a.is_null() && o.is_null() && !t.is_null());
        assert_eq!(
            git_index_conflict_next(&raw mut a, &raw mut o, &raw mut t, ci),
            -31
        );
        git_index_conflict_iterator_free(ci);
        git_index_free(ix);
    }
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn comparison_functions() {
    let _g = test_support::init();
    let mut a = GitIndexEntry::zeroed();
    let mut b = GitIndexEntry::zeroed();
    a.path = c"A".as_ptr();
    b.path = c"a".as_ptr();
    let (pa, pb) = (
        (&raw const a).cast::<c_void>(),
        (&raw const b).cast::<c_void>(),
    );
    // SAFETY: two entries with string paths.
    unsafe {
        assert!(git_index_entry_cmp(pa, pb) < 0);
        assert_eq!(git_index_entry_icmp(pa, pb), 0);
        b.set_stage(2);
        assert!(git_index_entry_icmp(pa, (&raw const b).cast()) < 0);
        assert_eq!(git_index_entry_stage(&raw const b), 2);
        assert_eq!(git_index_entry_is_conflict(&raw const b), 1);
        assert_eq!(git_index__create_mode(0o100700), 0o100755);
        assert_eq!(git_index__create_mode(0o120777), 0o120000);
        assert_eq!(git_index__create_mode(0o040755), 0o160000);
    }
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn extensions_and_reuc() {
    let _g = test_support::init();
    let ix = new_index();
    // SAFETY: (throughout) a live index; valid arguments.
    unsafe {
        assert_eq!(
            git_index_extension_add(ix, c"AB".as_ptr(), c"xy".as_ptr(), 2),
            0
        );
        let mut buf = RawBuf::default();
        assert_eq!(
            git_index_extension_lookup(&raw mut buf, ix, c"AB".as_ptr()),
            0
        );
        assert_eq!(
            core::slice::from_raw_parts(buf.ptr().cast::<u8>(), buf.size()),
            b"xy"
        );
        libgit2_rs::util::alloc::free(buf.ptr().cast());
        assert_eq!(git_index_extension_remove(ix, c"AB".as_ptr()), 0);
        assert_eq!(git_index_extension_remove(ix, c"AB".as_ptr()), -3);

        let id = Oid::from_hex(b"2222222222222222222222222222222222222222", OidType::Sha1).unwrap();
        assert_eq!(
            git_index_reuc_add(
                ix,
                c"p".as_ptr(),
                0o100644,
                &raw const id,
                0,
                ptr::null(),
                0,
                ptr::null()
            ),
            0
        );
        assert_eq!(
            git_index_reuc_add(
                ix,
                c"q".as_ptr(),
                0,
                ptr::null(),
                0o100644,
                ptr::null(),
                0,
                ptr::null()
            ),
            -1
        );
        assert_eq!(last(), "unrecoverable internal error: 'our_oid'");
        let r = git_index_reuc_get_bypath(ix, c"p".as_ptr());
        assert!(!r.is_null());
        assert_eq!(CStr::from_ptr((*r).path).to_bytes(), b"p");
        let mut pos = 9;
        assert_eq!(git_index_reuc_find(&raw mut pos, ix, c"zz".as_ptr()), -3);
        assert_eq!(pos, 1);
        assert_eq!(git_index_reuc_remove(ix, 5), -3);
        assert_eq!(git_index_reuc_clear(ix), 0);
        git_index_free(ix);
    }
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn writer_locks_the_index() {
    let _g = test_support::init();
    let d = std::env::temp_dir().join(format!("l_index_ffi.{}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    let path = std::ffi::CString::new(d.join("index").to_str().unwrap()).unwrap();
    let mut ix = ptr::null_mut();
    // SAFETY: (throughout) valid arguments; the writers are zeroed
    // (`GIT_INDEXWRITER_INIT`) and do not move.
    unsafe {
        assert_eq!(git_index_open(&raw mut ix, path.as_ptr()), 0);
        assert_eq!(add(ix, c"f", 0), 0);
        let mut one: GitIndexWriter = core::mem::zeroed();
        let mut two: GitIndexWriter = core::mem::zeroed();
        assert_eq!(git_indexwriter_init(&raw mut one, ix), 0);
        assert_eq!(git_indexwriter_init(&raw mut two, ix), -14);
        assert_eq!(
            last(),
            "the index is locked; this might be due to a concurrent or crashed process"
        );
        assert_eq!(git_indexwriter_commit(&raw mut one), 0);
        git_indexwriter_cleanup(&raw mut one);
        git_indexwriter_cleanup(&raw mut two);
        let mut back = ptr::null_mut();
        assert_eq!(git_index_open(&raw mut back, path.as_ptr()), 0);
        assert_eq!(git_index_entrycount(back), 1);
        git_index_free(back);
        git_index_free(ix);
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
#[cfg_attr(miri, ignore = "reaches C")]
fn callbacks_may_reenter() {
    let _g = test_support::init();
    let ix = new_index();
    for p in [c"a/1", c"a/2", c"a/3", c"z"] {
        assert_eq!(add(ix, p, 0), 0);
    }
    unsafe extern "C" fn cb(path: *const c_char, _m: *const c_char, payload: *mut c_void) -> c_int {
        let ix = payload.cast::<GitIndex>();
        // SAFETY: the test's index and a string path.
        unsafe {
            if CStr::from_ptr(path).to_bytes() == b"a/1" {
                // free another entry the operation will reach
                assert_eq!(git_index_remove(ix, c"a/2".as_ptr(), 0), 0);
            }
        }
        0
    }
    let s = [c"a/*".as_ptr().cast_mut()];
    let arr = libgit2_rs::strarray::GitStrArray {
        strings: s.as_ptr().cast_mut(),
        count: 1,
    };
    // SAFETY: a live index, a valid array and a sound callback.
    unsafe {
        assert_eq!(
            git_index_remove_all(ix, &raw const arr, Some(cb), ix.cast()),
            0
        );
        assert_eq!(git_index_entrycount(ix), 1);
        git_index_free(ix);
    }
}
