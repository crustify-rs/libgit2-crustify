//! UB tests for `deps/reftable/basics.c`: the enum `HashId`, the buffer `Buf` and its owner,
//! the owners of reftable's allocator, `parse_names`' `Names` and `binsearch`'s callback.
//!
//! `HashId` is a fieldless enum over C's `enum reftable_hash`. The only safe way in from a
//! raw value is `from_raw` / `try_from`, which must reject every value without a variant
//! rather than produce an invalid discriminant, which Miri and `UBSan` would flag on the
//! next read; an `as` cast to it does not compile (a `compile_fail` doctest on `HashId`).
//!
//! A `Buf` value comes from `Buf::zeroed()`, the unallocated state, which its getters read
//! without following `buf`, or from `Buf::new()`, whose `BufVal` releases what the buffer
//! comes to own. Its handles reborrow without aliasing, and what they read is tied to the
//! value's borrow (`compile_fail` doctests on `Buf` and its getters). The owners are moved,
//! dropped after handing their allocation out, and dropped on other threads (`TSan`), and a
//! buffer, which is `Send` and `Sync`, is grown and released on other threads and read by
//! several at once;
//! `binsearch` is re-entered from its own predicate and stopped by a panic. Every test that
//! allocates or calls C is ignored under Miri. `Buf::INIT` is the unallocated value as a
//! constant.
//!
//! A `NamesRef` borrows a NULL-terminated array of strings for its lifetime, and cannot
//! outlive the `NamesVec` or `Names` that lends it nor the strings it points at
//! (`compile_fail` doctests on `NamesRef`). `names_length` and `names_equal` read the arrays
//! up to their terminators, which the tests reach through lists of every relative length,
//! so that an over-read past a terminator is caught by `ASan`.

use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::hint::black_box;
use std::{panic, thread};

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::deps::reftable::basics::{
    Buf, BufMut, BufRef, BufVal, HashId, NamesRef, NamesVec, ReftableArrayFree, ReftableBox,
    ReftableStr, ReftableVec, binsearch, free_and_null, parse_names,
};

/// Every value around the declared ones, then a coarse stride across the whole type and
/// its top; `BorrowSanitizer` instruments each comparison, so the probe set stays small.
fn probes() -> impl Iterator<Item = ffi::reftable_hash> {
    (0..=300)
        .chain((0..=ffi::reftable_hash::MAX).step_by(1 << 26))
        .chain([ffi::reftable_hash::MAX - 1, ffi::reftable_hash::MAX])
}

#[test]
fn hash_id_from_raw_yields_only_valid_ids() {
    let mut seen = HashSet::new();
    for raw in probes() {
        // `black_box` keeps the conversion from being folded at compile time.
        let Some(id) = HashId::from_raw(black_box(raw)) else {
            assert!(HashId::try_from(raw).is_err());
            continue;
        };
        // Reading the discriminant back is where an invalid value would be caught.
        let back = black_box(id).raw();
        assert_eq!(back, raw);
        assert!(matches!(id, HashId::Sha1 | HashId::Sha256));
        assert_eq!(HashId::try_from(raw), Ok(id));
        seen.insert(id);
    }
    assert_eq!(seen.len(), 2, "both ids are reached");
}

#[test]
fn hash_id_round_trips_through_its_c_value() {
    for id in [HashId::Sha1, HashId::Sha256] {
        let raw = ffi::reftable_hash::from(black_box(id));
        assert_eq!(HashId::from_raw(raw), Some(id));
    }
}

/// Reads every getter, so that each one runs over the handle it is given.
fn read_all(b: BufRef<'_>) -> (usize, bool, usize, usize, bool) {
    (
        b.len(),
        b.is_empty(),
        b.capacity(),
        b.as_bytes().len(),
        b.as_c_str().is_some_and(CStr::is_empty),
    )
}

const EMPTY: (usize, bool, usize, usize, bool) = (0, true, 0, 0, true);

#[test]
fn zeroed_buf_reads_as_empty_through_every_handle() {
    let mut value = Buf::zeroed();
    assert_eq!(read_all(value.as_ref()), EMPTY);
    let mut m = value.as_mut();
    assert_eq!(read_all(m.as_ref()), EMPTY);
    assert_eq!(read_all(m.as_mut().as_ref()), EMPTY);
    assert_eq!(read_all(value.as_ref()), EMPTY);
    assert_eq!(format!("{value:?}"), format!("{:?}", value.as_ref()));
}

/// Takes an exclusive handle by value and hands a reborrow on, as a routine that fills a
/// buffer would.
fn pass_on(mut m: BufMut<'_>) -> usize {
    let inner = read_all(m.as_mut().as_ref()).0;
    inner + m.as_ref().len()
}

#[test]
fn buf_handles_reborrow_without_aliasing() {
    let mut value = Buf::zeroed();
    let mut m = value.as_mut();
    assert_eq!(pass_on(m.as_mut()), 0);
    assert_eq!(pass_on(m), 0);
    let shared = value.as_ref();
    let copy = shared;
    assert_eq!(read_all(shared), read_all(copy));
}

#[test]
fn buf_values_move_and_stay_empty() {
    let boxed = Box::new(Buf::zeroed());
    let mut values: Vec<Buf> = (0..64).map(|_| Buf::zeroed()).collect();
    values.push(*boxed);
    for value in &mut values {
        assert_eq!(read_all(value.as_mut().as_ref()), EMPTY);
    }
    let moved: Vec<Buf> = values.into_iter().rev().collect();
    assert!(moved.iter().all(|v| read_all(v.as_ref()) == EMPTY));
}

#[test]
fn bytes_outlive_the_handle_they_were_read_through() {
    let value = Buf::zeroed();
    // Tied to the value's borrow, not to the handle's own scope.
    let (bytes, c_str) = {
        let shared = value.as_ref();
        (shared.as_bytes(), shared.as_c_str())
    };
    assert_eq!(bytes, b"");
    assert_eq!(c_str, Some(c""));
    assert_eq!(read_all(value.as_ref()), EMPTY);
}

/// Fills `m` through a reborrow, then reads it through the handle itself.
fn fill(mut m: BufMut<'_>, bytes: &[u8]) -> usize {
    m.as_mut().add(bytes).unwrap();
    m.as_ref().as_bytes().len()
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn owned_buffers_grow_move_and_release_once() {
    let mut values: Vec<BufVal> = (0..16).map(|_| Buf::new()).collect();
    for (i, value) in values.iter_mut().enumerate() {
        let bytes = vec![b'r'; i * 37];
        assert_eq!(fill(value.as_mut(), &bytes), i * 37);
        value.as_mut().add_str(c"!").unwrap();
    }
    // Moving an owner moves the struct, not the allocation it owns.
    let moved: Vec<BufVal> = values.into_iter().rev().collect();
    for (i, value) in moved.iter().rev().enumerate() {
        let b = value.as_ref();
        assert_eq!(b.len(), i * 37 + 1);
        assert_eq!(b.as_bytes().last(), Some(&b'!'));
        assert_eq!(b.as_c_str().map(CStr::count_bytes), Some(b.len()));
    }
    let boxed = Box::new(Buf::new());
    let mut value = *boxed;
    value.as_mut().add(b"boxed").unwrap();
    assert_eq!(value.as_ref().as_bytes(), b"boxed");
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn detached_and_reset_buffers_release_nothing_twice() {
    let mut value = Buf::new();
    value.as_mut().add(b"first").unwrap();
    let first = value.as_mut().detach().unwrap();
    value.as_mut().add(b"second").unwrap();
    value.as_mut().truncate(3).unwrap();
    value.as_mut().reset();
    value.as_mut().add_str(c"third").unwrap();
    let third = value.as_mut().detach().unwrap();
    assert_eq!(value.as_mut().detach(), None);
    drop(value);
    assert_eq!((first.as_c_str(), third.as_c_str()), (c"first", c"third"));
    // A buffer on reftable's heap, filled, then released before its box.
    let mut boxed = ReftableBox::<Buf>::new_zeroed().unwrap();
    boxed.as_mut().add(b"on the heap").unwrap();
    let s = boxed.as_mut().detach().unwrap();
    drop(boxed);
    assert_eq!(s.as_c_str(), c"on the heap");
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn comparisons_borrow_without_aliasing_writers() {
    let (mut a, mut b) = (Buf::new(), Buf::new());
    a.as_mut().add(b"refs/heads/a").unwrap();
    b.as_mut().add(b"refs/heads/b").unwrap();
    let (ra, rb) = (a.as_ref(), b.as_ref());
    assert!(ra < rb);
    assert_eq!(ra.common_prefix_len(rb), 11);
    assert_eq!(ra.common_prefix_len(ra), 12);
    // A handle compared with itself, and with its own copy.
    let copy = ra;
    assert_eq!(ra, copy);
    a.as_mut().add(b"x").unwrap();
    assert!(a.as_ref() < b.as_ref());
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn reftable_owners_resize_clone_and_drop() {
    let mut run = ReftableVec::<u64>::new_uninit(0).unwrap();
    for request in 1..=64 {
        ReftableArrayFree::grow(&mut run, request).unwrap();
        assert!(run.len() >= request);
    }
    ReftableArrayFree::resize(&mut run, 3).unwrap();
    ReftableArrayFree::resize(&mut run, 0).unwrap();
    let copies: Vec<_> = (0..8)
        .map(|i| ReftableVec::from_slice(&[i; 5]).unwrap())
        .collect();
    let cloned = copies.clone();
    assert_eq!(copies, cloned);
    let strings: Vec<ReftableStr> = (0..8)
        .map(|_| ReftableStr::from_c_str(c"name").unwrap())
        .collect();
    assert!(strings.iter().all(|s| s.clone() == *s));
    let set: HashSet<&CStr> = strings.iter().map(ReftableStr::as_c_str).collect();
    assert_eq!(set.len(), 1);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn names_are_read_while_the_array_lives() {
    let names = parse_names(b"a\nbb\n\nccc\n").unwrap();
    let read: Vec<usize> = names.iter().map(CStr::count_bytes).collect();
    assert_eq!(read, [1, 2, 3]);
    let first = names.get(0).unwrap();
    assert_eq!(first, c"a");
    drop(names);
    let failures = (0..32)
        .filter(|i| parse_names(&[b'n'; 3][..i % 4]).is_err())
        .count();
    assert_eq!(
        failures, 24,
        "every input without a newline but the empty one"
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn binsearch_is_reentered_from_its_own_predicate() {
    let outer = binsearch(16, |k| {
        // Each level is a callback of its own: no `&mut` closure is aliased.
        let inner = binsearch(k + 1, |j| Ok::<_, ()>(j >= k))?;
        Ok::<_, ()>(inner == k && k >= 9)
    });
    assert_eq!(outer, Ok(9));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn binsearch_panic_leaves_nothing_behind() {
    let mut state = [0u8; 16];
    let caught = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        binsearch(state.len(), |k| -> Result<bool, ()> {
            state[k] += 1;
            assert!(k != 4, "stop");
            Ok(k > 4)
        })
    }));
    assert!(caught.is_err());
    // The closure ran no further probes once it panicked.
    assert!(state.iter().all(|&n| n <= 1));
    assert_eq!(binsearch(state.len(), |k| Ok::<_, ()>(state[k] > 0)), Ok(4));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn strings_and_runs_are_released_on_other_threads() {
    let strings: Vec<ReftableStr> = (0..8)
        .map(|_| ReftableStr::from_c_str(c"0x01-0x02-abcd.ref").unwrap())
        .collect();
    let runs: Vec<ReftableVec<u8>> = (0..8)
        .map(|i| ReftableVec::from_slice(&[i; 9]).unwrap())
        .collect();
    let workers: Vec<_> = strings
        .into_iter()
        .zip(runs)
        .map(|(s, run)| {
            thread::spawn(move || {
                let copy = s.clone();
                (copy == s, run.len())
            })
        })
        .collect();
    for worker in workers {
        assert_eq!(worker.join().unwrap(), (true, 9));
    }
    // Allocated on one thread, read and released on another, while this one allocates.
    let s = ReftableStr::from_c_str(c"shared").unwrap();
    thread::scope(|scope| {
        scope.spawn(|| assert_eq!(s.as_c_str(), c"shared"));
        scope.spawn(|| assert_eq!(s.to_str(), Ok("shared")));
        let _local = ReftableVec::from_slice(b"local").unwrap();
    });
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn buffers_grow_read_and_release_on_other_threads() {
    // Filled here, grown and released on other threads while this one allocates.
    let workers: Vec<_> = (0..8u8)
        .map(|i| {
            let mut value = Buf::new();
            value.as_mut().add(&[b'a' + i; 5]).unwrap();
            thread::spawn(move || {
                value.as_mut().add(b"-grown").unwrap();
                let read = (value.as_ref().len(), value.as_ref().as_c_str().is_some());
                drop(value);
                read
            })
        })
        .collect();
    let mut local = Buf::new();
    local.as_mut().add(b"local").unwrap();
    for worker in workers {
        assert_eq!(worker.join().unwrap(), (11, true));
    }
    // One buffer read and compared by several threads at once, through the owner and
    // through shared handles sent to them.
    let mut a = Buf::new();
    a.as_mut().add(b"refs/heads/main").unwrap();
    let mut b = Buf::new();
    b.as_mut().add(b"refs/heads/maint").unwrap();
    let rb = b.as_ref();
    thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                let ra = a.as_ref();
                assert!(ra < rb);
                assert_eq!(ra.common_prefix_len(rb), 15);
                assert_eq!(ra.as_c_str(), Some(c"refs/heads/main"));
            });
        }
        local.as_mut().add(b"-more").unwrap();
    });
    // An exclusive handle fills a buffer on another thread; the owner reads it afterwards.
    let mut c = Buf::new();
    let m = c.as_mut();
    thread::scope(|scope| {
        scope.spawn(move || fill(m, b"filled elsewhere"));
    });
    assert_eq!(c.as_ref().as_bytes(), b"filled elsewhere");
    // Released on another thread after this one detached part of it.
    let detached = c.as_mut().detach().unwrap();
    c.as_mut().add(b"again").unwrap();
    thread::spawn(move || drop((c, detached))).join().unwrap();
}

#[test]
fn buf_init_reads_as_empty_and_moves() {
    let mut values: Vec<Buf> = (0..8).map(|_| Buf::INIT).collect();
    values.push(Buf::INIT);
    for value in &mut values {
        assert_eq!(read_all(value.as_ref()), EMPTY);
        assert_eq!(pass_on(value.as_mut()), 0);
    }
    let boxed: Vec<Box<Buf>> = values.into_iter().map(Box::new).collect();
    assert!(boxed.iter().all(|b| read_all(b.as_ref().as_ref()) == EMPTY));
}

/// Reads every part of a list: through C (`names_length`, `names_equal` with itself) and
/// through Rust (its iterator).
fn read_names(names: NamesRef<'_>) -> (usize, usize) {
    let iterated: usize = names.iter().map(|n| n.to_bytes().len()).sum();
    let count = names.iter().count();
    assert_eq!(names.len(), count);
    assert!(names.equals(names));
    (count, iterated)
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn name_lists_of_every_relative_length_are_read_up_to_their_terminators() {
    // Heap strings of growing length, so that ASan bounds each one and each array.
    let strings: Vec<CString> = (0..6)
        .map(|i| CString::new("n".repeat(i)).unwrap())
        .collect();
    let lists: Vec<NamesVec<'_>> = (0..=strings.len())
        .map(|n| NamesVec::new(strings[..n].iter().map(CString::as_c_str)))
        .collect();
    for (i, a) in lists.iter().enumerate() {
        let (count, bytes) = read_names(a.as_ref());
        assert_eq!(count, i);
        assert_eq!(bytes, (0..i).sum::<usize>());
        for (j, b) in lists.iter().enumerate() {
            // A prefix and a longer list differ at the shorter one's terminator.
            assert_eq!(a.as_ref().equals(b.as_ref()), i == j);
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn name_handles_copy_and_outlive_the_scope_that_made_them() {
    let strings = [c"a.ref".to_owned(), c"b.ref".to_owned()];
    let list = NamesVec::new(strings.iter().map(CString::as_c_str));
    let names = {
        let first = list.as_ref();
        let copy = first;
        assert_eq!(read_names(first), read_names(copy));
        copy
    };
    // Tied to the list's borrow, not to the handle's scope.
    let read: Vec<&CStr> = names.iter().collect();
    assert_eq!(read, [c"a.ref", c"b.ref"]);
    assert_eq!(read_names(names), (2, 10));
    // A list moves with its heap array; a fresh handle reads the moved list.
    let moved = Box::new(list);
    assert_eq!(read_names(moved.as_ref().as_ref()), (2, 10));
}

#[test]
fn free_and_null_releases_each_owner_once() {
    let mut slots: Vec<Option<Box<Buf>>> = (0..16).map(|_| Some(Box::new(Buf::INIT))).collect();
    for slot in &mut slots {
        free_and_null(slot);
        free_and_null(slot);
        assert!(slot.is_none());
    }
    let mut nested = Some(vec![Some(Box::new(1u8)), None]);
    free_and_null(&mut nested);
    assert!(nested.is_none());
}

#[test]
fn name_handles_iterate_without_calling_c() {
    let strings: Vec<CString> = (0..4)
        .map(|i| CString::new("x".repeat(i)).unwrap())
        .collect();
    let list = NamesVec::new(strings.iter().map(CString::as_c_str));
    let names = list.as_ref();
    let mut a = names.iter();
    let b = a;
    assert_eq!(a.next(), Some(c""));
    assert_eq!(b.count(), 4, "a copy taken before a step starts over");
    assert_eq!(a.count_remaining(), 3);
    assert!(!names.is_empty());
    assert!(NamesVec::new([]).as_ref().is_empty());
    assert_eq!(format!("{names:?}"), r#"["", "x", "xx", "xxx"]"#);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn allocation_macro_forms_release_what_they_allocate_once() {
    for len in [0, 1, 7, 64] {
        let mut run = ReftableArrayFree::alloc_array::<u16>(len).unwrap();
        for slot in &mut run {
            slot.write(0xBEEF);
        }
        ReftableArrayFree::grow_or_clear(&mut run, len + 1).unwrap();
        assert!(run.len() > len);
        // A failed growth releases the run once and leaves nothing for its drop.
        assert!(ReftableArrayFree::grow_or_clear(&mut run, usize::MAX).is_err());
        assert!(run.is_empty());
        let mut slot = Some(ReftableArrayFree::alloc_array::<u8>(len + 1).unwrap());
        free_and_null(&mut slot);
        free_and_null(&mut slot);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn parsed_names_lend_their_array_while_borrowed() {
    let parsed = parse_names(b"a.ref\nbb.ref\n\nccc.ref\n").unwrap();
    let names = parsed.as_ref();
    assert_eq!(read_names(names), (3, 18));
    let built = NamesVec::new(parsed.iter());
    assert!(names.equals(built.as_ref()));
    assert!(built.as_ref().equals(names));
}
