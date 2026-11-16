//! Tests of `git_vector` and `tsort`: the safe API and the exports.

// These tests call the exports in sequences, as C does; the SAFETY comment
// of each block covers the calls in it.
#![allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]

use core::ffi::{c_int, c_void};
use libgit2_rs::util::errors::{self, ErrorCode};
use libgit2_rs::util::test_support;
use libgit2_rs::util::tsort::tsort;
use libgit2_rs::util::vector::{GitVector, ffi};

fn msg() -> String {
    String::from_utf8(errors::last_message().unwrap_or_default()).unwrap()
}

/// Elements are indices cast to pointers; nothing dereferences them.
fn p(i: usize) -> *mut c_void {
    core::ptr::without_provenance_mut(i)
}
fn ix(e: *mut c_void) -> usize {
    e.addr()
}

unsafe extern "C" fn cmp_addr(a: *const c_void, b: *const c_void) -> c_int {
    (a.addr() as isize - b.addr() as isize).signum() as c_int
}

#[test]
fn growth_matches_c() {
    let _g = test_support::init();
    let mut v = GitVector::with_capacity(0, None).unwrap();
    assert_eq!(v.capacity(), 8);
    let mut caps = vec![];
    for i in 0..100 {
        v.insert(p(i)).unwrap();
        caps.push(v.capacity());
    }
    caps.dedup();
    assert_eq!(caps, [8, 12, 18, 27, 40, 60, 90, 135]);
    assert_eq!(v.len(), 100);
    assert!(!v.is_sorted());
    let mut w = GitVector::new();
    w.insert(p(1)).unwrap();
    assert!(w.is_sorted(), "one element is sorted");
    assert_eq!(w.capacity(), 8);
}

#[test]
fn sort_search_insert_sorted() {
    let _g = test_support::init();
    let mut v = GitVector::with_capacity(4, Some(cmp_addr)).unwrap();
    for i in [5, 3, 9, 1, 7] {
        v.insert(p(i)).unwrap();
    }
    // SAFETY: `cmp_addr` compares addresses only.
    unsafe { v.sort() };
    assert!(v.is_sorted());
    assert_eq!(v.iter().map(ix).collect::<Vec<_>>(), [1, 3, 5, 7, 9]);
    // SAFETY: as above.
    unsafe {
        v.insert_sorted(p(4), None).unwrap();
        v.insert_sorted(p(10), None).unwrap();
        v.insert_sorted(p(0), None).unwrap();
        assert_eq!(
            v.iter().map(ix).collect::<Vec<_>>(),
            [0, 1, 3, 4, 5, 7, 9, 10]
        );
        let mut pos = 0;
        assert_eq!(v.bsearch(Some(&mut pos), p(7)), Ok(()));
        assert_eq!(pos, 5);
        assert_eq!(v.bsearch(Some(&mut pos), p(8)), Err(ErrorCode::NOT_FOUND));
        assert_eq!(pos, 6);
        assert_eq!(v.search(p(9)), Ok(6));
        assert_eq!(v.search(p(2)), Err(ErrorCode::NOT_FOUND));
        assert_eq!(v.verify_sorted(), 0);
    }
    // Duplicates: on_dup refusing stops the insertion with its code.
    let mut calls = 0;
    let r = v.insert_sorted_by(
        p(5),
        |a, b| ix(a) as c_int - ix(b) as c_int,
        Some(&mut |_old: &mut *mut c_void, _new| {
            calls += 1;
            -42
        }),
    );
    assert_eq!(r.map_err(ErrorCode::raw), Err(-42));
    assert_eq!(calls, 1);
    assert_eq!(v.len(), 8);
    // Accepting inserts it anyway, as C.
    v.insert_sorted_by(
        p(5),
        |a, b| ix(a) as c_int - ix(b) as c_int,
        Some(&mut |_: &mut *mut c_void, _| 0),
    )
    .unwrap();
    assert_eq!(v.len(), 9);
    // SAFETY: as above.
    unsafe { v.uniq(None) };
    assert_eq!(
        v.iter().map(ix).collect::<Vec<_>>(),
        [0, 1, 3, 4, 5, 7, 9, 10]
    );
}

#[test]
fn edits() {
    let _g = test_support::init();
    let mut v = GitVector::with_capacity(0, None).unwrap();
    for i in 1..=5 {
        v.insert(p(i)).unwrap();
    }
    v.remove(1).unwrap();
    assert_eq!(v.remove(10), Err(ErrorCode::NOT_FOUND));
    assert_eq!(v.iter().map(ix).collect::<Vec<_>>(), [1, 3, 4, 5]);
    v.insert_null(1, 2).unwrap();
    assert_eq!(v.iter().map(ix).collect::<Vec<_>>(), [1, 0, 0, 3, 4, 5]);
    assert_eq!(v.insert_null(1, 0), Err(ErrorCode::GENERIC));
    assert_eq!(msg(), "invalid argument: 'insert_len > 0'");
    assert_eq!(v.insert_null(100, 1), Err(ErrorCode::GENERIC));
    assert_eq!(msg(), "invalid argument: 'idx <= v->length'");
    v.remove_range(1, 2).unwrap();
    assert_eq!(v.iter().map(ix).collect::<Vec<_>>(), [1, 3, 4, 5]);
    assert!(v.remove_range(3, 2).is_err());
    assert_eq!(
        msg(),
        "unrecoverable internal error: 'end_idx <= v->length'"
    );
    assert!(v.remove_range(1, usize::MAX).is_err());
    assert_eq!(msg(), "unrecoverable internal error: '0'");
    assert!(v.remove_range(1, 0).is_err());
    assert_eq!(msg(), "invalid argument: 'remove_len > 0'");
    assert_eq!(v.set(6, p(9)).unwrap(), p(0));
    assert_eq!(v.iter().map(ix).collect::<Vec<_>>(), [1, 3, 4, 5, 0, 0, 9]);
    assert_eq!(v.set(0, p(7)).unwrap(), p(1));
    v.reverse();
    assert_eq!(v.iter().map(ix).collect::<Vec<_>>(), [9, 0, 0, 5, 4, 3, 7]);
    v.remove_matching(|v, i| ix(v.get(i).unwrap()) == 0);
    assert_eq!(v.iter().map(ix).collect::<Vec<_>>(), [9, 5, 4, 3, 7]);
    v.resize_to(2).unwrap();
    assert_eq!(v.last(), Some(p(5)));
    v.pop();
    v.pop();
    v.pop();
    assert!(v.is_empty());
    let (ptr, n, a) = v.detach();
    assert_eq!((n, a), (0, 8));
    // SAFETY: the detached array is a `git__allocator` allocation.
    unsafe { libgit2_rs::util::alloc::free(ptr.cast()) };
    assert_eq!(v.capacity(), 0);
}

#[test]
fn dup_and_cmp() {
    let _g = test_support::init();
    // One function pointer value: comparisons of separately reified Rust
    // functions are not reliable (Miri gives them distinct addresses).
    let cmp: libgit2_rs::util::vector::VectorCmp = cmp_addr;
    let mut v = GitVector::with_capacity(0, Some(cmp)).unwrap();
    v.insert(p(2)).unwrap();
    v.insert(p(1)).unwrap();
    // SAFETY: `cmp_addr` compares addresses only.
    unsafe { v.sort() };
    // C compares the argument with the source's function: NULL differs,
    // so the copy is not marked sorted (though it keeps the function).
    let d = v.dup(None).unwrap();
    assert!(!d.is_sorted());
    assert!(d.cmp_fn().is_some());
    let d = v.dup(Some(cmp)).unwrap();
    assert!(d.is_sorted());
    assert_eq!(d.capacity(), 2);
    assert_eq!(d.as_slice(), v.as_slice());
    unsafe extern "C" fn other(_: *const c_void, _: *const c_void) -> c_int {
        0
    }
    let d = v.dup(Some(other)).unwrap();
    assert!(!d.is_sorted());
    let mut d = d;
    d.set_cmp(Some(cmp));
    assert!(!d.is_sorted());
    d.set_sorted(true);
    d.set_cmp(Some(cmp));
    assert!(d.is_sorted(), "same function keeps the flag");
}

#[test]
fn tsort_is_stable_and_sorts() {
    let _g = test_support::init();
    let mut seed = 1u64;
    let mut rnd = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    for n in [0usize, 1, 2, 3, 63, 64, 65, 100, 1000, 5000] {
        for pattern in 0..4 {
            let mut v: Vec<(u32, usize)> = (0..n)
                .map(|i| {
                    let key = match pattern {
                        0 => rnd() % 50,
                        1 => i as u32,
                        2 => (n - i) as u32,
                        _ => (i as u32 / 7) ^ (rnd() % 2),
                    };
                    (key, i)
                })
                .collect();
            let mut expect = v.clone();
            expect.sort_by_key(|e| e.0);
            tsort(&mut v, |a, b| a.0 as c_int - b.0 as c_int);
            assert_eq!(v, expect, "n={n} pattern={pattern}");
        }
    }
}

#[test]
fn tsort_merge_oom_leaves_it_unsorted() {
    let g = test_support::exclusive();
    let mut v: Vec<u32> = (0..1000).rev().collect();
    test_support::with_alloc_limit(&g, 0, || {
        errors::clear();
        tsort(&mut v, |a, b| a as c_int - b as c_int);
        assert_eq!(errors::last_class(), Some(errors::ErrorClass::NOMEMORY));
    });
    assert!(!v.is_sorted(), "merges were skipped, as in C");
}

// ---------------------------------------------------------------------------
// The exports
// ---------------------------------------------------------------------------

unsafe extern "C" fn match_odd(v: *const GitVector, idx: usize, payload: *mut c_void) -> c_int {
    // SAFETY: called by `git_vector_remove_matching` with the vector and an
    // index below its length; `payload` is a `usize` counter.
    unsafe {
        *payload.cast::<usize>() += 1;
        c_int::from((*v).get(idx).unwrap().addr() % 2 == 1)
    }
}

#[test]
fn ffi_calls() {
    let _g = test_support::init();
    let mut v = core::mem::MaybeUninit::<GitVector>::uninit();
    // SAFETY: as C: init, use, dispose a git_vector on the stack.
    unsafe {
        assert_eq!(ffi::git_vector_init(v.as_mut_ptr(), 2, Some(cmp_addr)), 0);
        let vp = v.as_mut_ptr();
        for i in [4, 1, 3, 2, 5] {
            assert_eq!(ffi::git_vector_insert(vp, p(i)), 0);
        }
        ffi::git_vector_sort(vp);
        assert_eq!((*vp).iter().map(ix).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
        let mut pos = 99;
        assert_eq!(
            ffi::git_vector_bsearch2(&mut pos, vp, Some(cmp_addr), p(3)),
            0
        );
        assert_eq!(pos, 2);
        assert_eq!(
            ffi::git_vector_bsearch2(&mut pos, vp, Some(cmp_addr), core::ptr::null()),
            -1
        );
        assert_eq!(msg(), "invalid argument: 'key'");
        assert_eq!(ffi::git_vector_search2(&mut pos, vp, None, p(3)), -1);
        assert_eq!(msg(), "unrecoverable internal error: 'key_lookup'");
        assert_eq!(ffi::git_vector_search(&mut pos, vp, p(5)), 0);
        assert_eq!(pos, 4);
        let mut count = 0usize;
        ffi::git_vector_remove_matching(vp, match_odd, (&raw mut count).cast());
        assert_eq!(count, 5);
        assert_eq!((*vp).iter().map(ix).collect::<Vec<_>>(), [2, 4]);
        let mut old = p(0);
        assert_eq!(ffi::git_vector_set(&mut old, vp, 3, p(8)), 0);
        assert_eq!((*vp).iter().map(ix).collect::<Vec<_>>(), [2, 4, 0, 8]);
        let mut d = core::mem::MaybeUninit::<GitVector>::uninit();
        assert_eq!(ffi::git_vector_dup(d.as_mut_ptr(), vp, None), 0);
        assert_eq!((*d.as_ptr()).len(), 4);
        ffi::git_vector_swap(vp, d.as_mut_ptr());
        ffi::git_vector_clear(vp);
        assert!((*vp).is_empty() && (*vp).is_sorted());
        ffi::git_vector_dispose(d.as_mut_ptr());
        ffi::git_vector_dispose(vp);
        ffi::git_vector_dispose(core::ptr::null_mut());
        assert_eq!(ffi::git_vector_insert(core::ptr::null_mut(), p(1)), -1);
        assert_eq!(msg(), "invalid argument: 'v'");
        d.assume_init_drop();
        v.assume_init_drop();
    }
    // A zeroed vector (GIT_VECTOR_INIT) works too.
    let mut z = GitVector::new();
    // SAFETY: as C.
    unsafe {
        assert_eq!(ffi::git_vector_insert(&mut z, p(1)), 0);
        let mut size = 0;
        let mut asize = 0;
        let arr = ffi::git_vector_detach(&mut size, &mut asize, &mut z);
        assert_eq!((size, asize), (1, 8));
        libgit2_rs::util::alloc::free(arr.cast());
    }
}

#[test]
fn ffi_dispose_deep() {
    let _g = test_support::init();
    let mut v = GitVector::with_capacity(0, None).unwrap();
    for _ in 0..3 {
        v.insert(libgit2_rs::util::alloc::malloc(16)).unwrap();
    }
    v.insert(core::ptr::null_mut()).unwrap();
    // SAFETY: the elements are distinct `git__malloc` allocations or NULL.
    unsafe { ffi::git_vector_dispose_deep(&mut v) };
    assert_eq!(v.len(), 0);
}
