//! `src/util/alloc.c`: `git2/sys/alloc.h`'s `Allocator` values, their handles and the
//! trampolines `Allocator::new` builds, driven by libgit2 itself; and the lifecycle
//! strategies over libgit2's allocator family, `AllocBox`, `AllocVec` and `AllocStr`.
//!
//! Each libgit2 test installs a Rust allocator in a child process of its own (libgit2's
//! allocator is process-global), then runs safe workloads that allocate, resize and
//! free through it; ASan/LSan flag a trampoline that mishandles a block, `TSan` one that
//! races, and the counters show the calls reached Rust.
//!
//! The strategies' owners are driven through the safe API alone: every size around a
//! page, copies that outlive their sources, partial fills dropped early, owners that
//! cross threads, and first allocations that race to initialise libgit2. `ASan` flags an
//! overrun, a double free or a free through the wrong allocator, `TSan` a race.

use std::ffi::{CStr, CString};
use std::mem::{MaybeUninit, align_of};
use std::thread;

use libgit2::libgit2::src::libgit2::oid::Oid;
use libgit2::libgit2::src::util::alloc::{AllocBox, AllocStr, AllocVec, ArrayFree, StrFree};

use crate::fixture::{Pair, isolated};

/// Up to just past a page; whole-slice comparisons keep `BSan`'s per-access cost low.
const SIZES: [usize; 7] = [1, 2, 7, 16, 63, 4096, 4097];

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 + 7).to_le_bytes()[0]).collect()
}

// --- one object -----------------------------------------------------------------------

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn box_uninit_may_drop_unfilled() {
    for _ in 0..64 {
        let mut slot = AllocBox::<Pair>::new_uninit().unwrap();
        assert!(!slot.as_c_ptr().is_null());
        assert_eq!(slot.as_c_ptr() as usize % align_of::<Pair>(), 0);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn box_zeroed_is_zero_and_writable() {
    let mut id = AllocBox::<Oid>::new_zeroed().unwrap();
    assert!(id.as_ref().id().iter().all(|&b| b == 0));
    assert!(id.as_mut().id_mut().copy_from_slice(&[0xAB; 32]));
    assert!(id.as_ref().id().iter().all(|&b| b == 0xAB));
    let other = AllocBox::<Oid>::new_zeroed().unwrap();
    assert_ne!(other.as_ptr(), id.as_ptr());
    assert!(other.as_ref().id().iter().all(|&b| b == 0));
}

// --- runs -------------------------------------------------------------------------------

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn vec_copies_slices_of_every_size() {
    for len in SIZES {
        let src = pattern(len);
        let mut v = AllocVec::<u8>::from_slice(&src).unwrap();
        assert_eq!((v.len(), v.byte_len()), (len, len));
        assert_eq!(v.as_slice(), &src[..]);
        // First and last bytes are in bounds; ASan flags any overrun.
        let s = v.as_mut_slice();
        s[0] ^= 0xFF;
        s[len - 1] ^= 0xFF;
        assert_eq!(
            v.as_slice()[len - 1],
            src[len - 1] ^ if len == 1 { 0 } else { 0xFF }
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn vec_clone_is_independent_and_outlives_its_source() {
    let src = pattern(100);
    let v = AllocVec::<u8>::from_slice(&src).unwrap();
    let mut w = v.clone();
    assert_ne!(v.as_ptr(), w.as_ptr());
    w.as_mut_slice().fill(0);
    assert_eq!(v.as_slice(), &src[..]);
    let t = w.try_clone().unwrap();
    drop(v);
    drop(w);
    assert_eq!(t.as_slice(), &[0; 100][..]);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn vec_zeroed_is_zero_and_writable_to_the_end() {
    for len in SIZES {
        let mut v = ArrayFree::new_zeroed::<u64>(len).unwrap();
        assert_eq!(v.as_slice(), &vec![0; len][..]);
        v.as_mut_slice()[len - 1] = u64::MAX;
        assert_eq!(v.as_slice()[len - 1], u64::MAX);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn vec_uninit_may_drop_partially_filled() {
    for len in SIZES {
        let mut v = ArrayFree::new_uninit::<u32>(len).unwrap();
        assert_eq!(v.len(), len);
        v.as_mut_slice()[len - 1].write(1);
        // Dropped without `assume_init`: the free reads no element.
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn vec_of_owners_releases_each() {
    let strings =
        AllocVec::<AllocStr>::from_fn(5, |i| StrFree::dup_bytes(&pattern(i * 9 + 1)[..]).unwrap())
            .unwrap();
    assert_eq!(strings.len(), 5);
    let lens: Vec<usize> = strings.iter().map(AllocStr::len).collect();
    assert!(lens.iter().zip(0..).all(|(&n, i)| n <= i * 9 + 1));
    let nullable = AllocVec::<Option<AllocStr>>::from_fn(4, |i| {
        (i % 2 == 0).then(|| StrFree::dup(c"x").unwrap())
    })
    .unwrap();
    assert_eq!(nullable.iter().flatten().count(), 2);
    // Dropping either releases each string, then the run.
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn vec_storage_aligns_wide_elements() {
    let words = AllocVec::<u64>::from_slice(&[1, 2, 3]).unwrap();
    assert_eq!(words.as_ptr() as usize % align_of::<u64>(), 0);
    let ptrs = ArrayFree::new_zeroed::<*const u8>(3).unwrap();
    assert!(ptrs.as_slice().iter().all(|q| q.is_null()));
    let floats = AllocVec::<f64>::from_slice(&[0.5, -1.0]).unwrap();
    assert_eq!(floats.clone().as_slice(), &[0.5, -1.0]);
    let wide = AllocVec::<u128>::from_slice(&[u128::MAX]).unwrap();
    assert_eq!(wide.as_ptr() as usize % align_of::<u128>(), 0);
    let byte = AllocVec::<u8>::from_slice(&[9]).unwrap();
    assert_eq!(byte.clone().as_slice(), &[9]);
}

#[test]
fn vec_empty_ffibox_forms_own_nothing() {
    let a = AllocVec::<u8>::from_slice(&[]).unwrap();
    let b = AllocVec::<u64>::from_fn(0, |_| unreachable!()).unwrap();
    assert!(a.as_ptr().is_null() && b.as_ptr().is_null());
    assert!(a.clone().is_empty() && a.as_slice().is_empty() && b.as_slice().is_empty());
}

/// `git__mallocarray(0, ..)` and `git__calloc(0, ..)` hold a zero-byte block, which reads
/// as an empty slice, copies, and is released once; `ASan` flags a lost or doubled free.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn vec_empty_c_forms_hold_a_zero_byte_block() {
    for _ in 0..16 {
        let b = ArrayFree::new_uninit::<u16>(0).unwrap();
        let c = ArrayFree::new_zeroed::<u64>(0).unwrap();
        assert!(b.as_slice().is_empty() && c.as_slice().is_empty());
        assert_eq!(c.as_ptr() as usize % align_of::<u64>(), 0);
        let d = c.clone();
        assert_eq!(d.len(), 0);
        drop(c);
        assert_eq!(d.try_clone().unwrap().len(), 0);
    }
}

#[test]
#[cfg_attr(miri, ignore = "reports through libgit2's error state")]
fn vec_rejects_oversized_requests_without_panicking() {
    assert!(ArrayFree::new_uninit::<u8>(isize::MAX as usize + 1).is_none());
    assert!(ArrayFree::new_uninit::<MaybeUninit<u64>>(usize::MAX / 4).is_none());
    assert!(ArrayFree::new_zeroed::<u32>(usize::MAX).is_none());
    assert!(AllocVec::<u64>::new_uninit(usize::MAX).is_none());
}

// --- strings ----------------------------------------------------------------------------

/// String lengths up to just past a page, the empty string included.
const STR_LENS: [usize; 8] = [0, 1, 2, 7, 16, 63, 4095, 4096];

fn text(len: usize, seed: u8) -> CString {
    CString::new(
        (0..len)
            .map(|i| b'!' + (i.to_le_bytes()[0].wrapping_mul(7) ^ seed) % 90)
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn str_copies_strings_of_every_length() {
    for len in STR_LENS {
        let src = text(len, 3);
        for s in [
            StrFree::dup(&src).unwrap(),
            StrFree::dup_prefix(src.as_bytes()).unwrap(),
            StrFree::dup_prefix(src.as_bytes_with_nul()).unwrap(),
            StrFree::dup_bytes(src.as_bytes()).unwrap(),
        ] {
            assert_ne!(s.as_ptr(), src.as_ptr());
            assert_eq!((s.len(), s.is_empty()), (len, len == 0));
            assert_eq!(s.as_c_str(), src.as_c_str());
            assert_eq!(s.to_str().unwrap().len(), len);
            assert_eq!(format!("{s:?}"), format!("CStrBox({src:?})"));
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn str_runs_end_at_their_first_nul() {
    for (run, expect) in [
        (&b"ab\0cd"[..], &b"ab"[..]),
        (b"\0", b""),
        (b"", b""),
        (b"\0\0x", b""),
        (b"no nul", b"no nul"),
    ] {
        let prefix = StrFree::dup_prefix(run).unwrap();
        let bytes = StrFree::dup_bytes(run).unwrap();
        assert_eq!((prefix.as_bytes(), bytes.as_bytes()), (expect, expect));
        assert_eq!(bytes.clone().as_bytes(), expect);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn str_keeps_bytes_that_are_not_utf8() {
    let s = StrFree::dup(c"caf\xe9").unwrap();
    assert!(s.to_str().is_err());
    assert_eq!(s.as_bytes(), b"caf\xe9");
    assert_eq!(s.clone().as_bytes(), b"caf\xe9");
    assert_eq!(
        StrFree::dup_bytes(b"\xff\xfe").unwrap().as_bytes(),
        b"\xff\xfe"
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn str_clone_is_independent_and_outlives_its_source() {
    let s = StrFree::dup(c"refs/remotes/origin/HEAD").unwrap();
    let t = s.clone();
    let u = t.try_clone().unwrap();
    assert!(s.as_ptr() != t.as_ptr() && t.as_ptr() != u.as_ptr());
    drop(s);
    drop(t);
    assert_eq!(u.as_c_str(), c"refs/remotes/origin/HEAD");
    let owned: CString = u.into();
    assert_eq!(owned.as_c_str(), c"refs/remotes/origin/HEAD");
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn owners_cross_threads() {
    let s = StrFree::dup(c"shared").unwrap();
    let v = AllocVec::<u8>::from_slice(&pattern(64)).unwrap();
    thread::scope(|scope| {
        for _ in 0..4 {
            // Shared reads and copies of one string and one run at once.
            scope.spawn(|| {
                assert_eq!(s.clone().as_bytes(), b"shared");
                assert_eq!(v.clone().as_slice(), v.as_slice());
            });
        }
    });
    // Made here, freed on other threads.
    let t = thread::spawn(move || drop(s));
    let u = thread::spawn(move || drop(v));
    t.join().unwrap();
    u.join().unwrap();

    let made: Vec<AllocStr> = (0..4)
        .map(|i| thread::spawn(move || StrFree::dup(&text(64, i)).unwrap()))
        .map(|h| h.join().unwrap())
        .collect();
    assert!(made.iter().all(|s| s.len() == 64));
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn owners_work_before_any_init() {
    isolated(
        concat!(module_path!(), "::owners_work_before_any_init"),
        || {
            // Nothing has initialised libgit2: an unpinned allocation would fail through
            // libgit2's `failalloc` and write the error through a nonexistent TLS key.
            let s = StrFree::dup_bytes(b"first").unwrap();
            let v = ArrayFree::new_zeroed::<u32>(2).unwrap();
            assert_eq!((s.clone().as_bytes(), v.clone().len()), (&b"first"[..], 2));
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn first_allocations_race_to_pin_the_runtime() {
    isolated(
        concat!(
            module_path!(),
            "::first_allocations_race_to_pin_the_runtime"
        ),
        || {
            let barrier = std::sync::Barrier::new(8);
            thread::scope(|s| {
                for i in 0..8u8 {
                    let barrier = &barrier;
                    s.spawn(move || {
                        barrier.wait();
                        let id = AllocBox::<Oid>::new_zeroed().unwrap();
                        let v = AllocVec::<u8>::from_slice(&[i; 32]).unwrap();
                        let t = StrFree::dup_prefix(&[b'a' + i; 3]).unwrap();
                        assert_eq!(v.clone().as_slice(), &[i; 32]);
                        assert_eq!(t.as_bytes(), &[b'a' + i; 3]);
                        drop(id);
                    });
                }
            });
        },
    );
}

/// A borrow of a string ends with its owner.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn str_borrows_are_tied_to_their_owner() {
    let s = StrFree::dup(c"tied").unwrap();
    let view: &CStr = s.as_c_str();
    assert_eq!(view, c"tied");
    // `drop(s)` here, with `view` used after, fails to compile (`StrFree`'s
    // `compile_fail` doctest).
}
