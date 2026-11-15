//! `src/util/thread.c`'s types: `git_atomic32`, `git_atomic64` with `git_atomic_ssize`
//! (`thread.h`), and `git_thread` (`unix/pthread.h`), as values Rust holds and the handles
//! over them. The C functions over them (`git_atomic32_inc` and its siblings, the
//! `git_thread_*` macros) are their port batches', with the C tests that check them
//! (`tests/libgit2/threads/atomic.c`). None of these reaches C, so Miri runs them all.

// The thread-local storage's tests (wave-226/batch-3).
mod tlsdata;

use core::sync::atomic::Ordering::SeqCst;

use ffibox::CZeroable;
use libgit2_crustify::libgit2::src::util::thread::{
    Atomic32, Atomic32Mut, Atomic32Ref, Atomic64, Atomic64Mut, AtomicSsize, Thread, ThreadRef,
};

mod atomics;

/// A new value holds what it was built with, and the default is 0, as C's `{0}` and zeroed
/// storage are.
#[test]
fn an_atomic32_holds_the_value_it_was_built_with() {
    for val in [0, 1, -1, 42, i32::MIN, i32::MAX] {
        assert_eq!(Atomic32::new(val).as_ref().val().load(SeqCst), val);
    }
    assert_eq!(Atomic32::default().as_ref().val().load(SeqCst), 0);
}

/// As [`an_atomic32_holds_the_value_it_was_built_with`], for 64 bits.
#[test]
fn an_atomic64_holds_the_value_it_was_built_with() {
    for val in [0, 1, -1, 1 << 40, i64::MIN, i64::MAX] {
        assert_eq!(Atomic64::new(val).as_ref().val().load(SeqCst), val);
    }
    assert_eq!(Atomic64::default().as_ref().val().load(SeqCst), 0);
}

/// Every handle reaches the one value: a write through one is read through each other, the
/// shared and exclusive reborrows of an exclusive handle included.
#[test]
fn every_handle_reaches_one_value() {
    let mut count = Atomic32::new(1);
    count.as_ref().val().store(2, SeqCst);
    let mut exclusive: Atomic32Mut<'_> = count.as_mut();
    assert_eq!(exclusive.as_ref().val().load(SeqCst), 2);
    exclusive.as_mut().as_ref().val().fetch_add(3, SeqCst);
    let shared: Atomic32Ref<'_> = exclusive.as_ref();
    let copy = shared;
    assert_eq!(shared.val().load(SeqCst), 5);
    assert_eq!(copy.val().swap(7, SeqCst), 5);
    assert_eq!(count.as_ref().val().load(SeqCst), 7);

    let mut size = Atomic64::new(-1);
    let mut exclusive: Atomic64Mut<'_> = size.as_mut();
    exclusive.as_mut().as_ref().val().fetch_sub(1, SeqCst);
    assert_eq!(size.as_ref().val().load(SeqCst), -2);
}

/// The lent atomic is borrowed for as long as the value, not the handle it came from.
#[test]
fn the_lent_atomic_lives_as_long_as_the_value() {
    let count = Atomic32::new(3);
    let val = {
        let handle = count.as_ref();
        handle.val()
    };
    assert_eq!(val.fetch_add(1, SeqCst), 3);
    assert_eq!(count.as_ref().val().load(SeqCst), 4);
}

/// The value is the integer two's complement arithmetic gives, wrapping at its bounds, as
/// C's `__atomic_add_fetch` and `__atomic_sub_fetch` on a signed integer do.
#[test]
fn arithmetic_wraps_at_the_bounds() {
    let count = Atomic32::new(i32::MAX);
    count.as_ref().val().fetch_add(1, SeqCst);
    assert_eq!(count.as_ref().val().load(SeqCst), i32::MIN);
    count.as_ref().val().fetch_sub(1, SeqCst);
    assert_eq!(count.as_ref().val().load(SeqCst), i32::MAX);

    let size = Atomic64::new(i64::MIN);
    size.as_ref().val().fetch_sub(1, SeqCst);
    assert_eq!(size.as_ref().val().load(SeqCst), i64::MAX);
}

/// A `static` is a value Rust holds, and is written through shared borrows, as C's globals
/// (`init_count`, `git_cache__current_storage`) are.
#[test]
fn a_static_value_is_writable() {
    static COUNT: Atomic32 = Atomic32::new(0);
    static SIZE: AtomicSsize = AtomicSsize::new(0);
    assert_eq!(COUNT.as_ref().val().fetch_add(1, SeqCst), 0);
    assert_eq!(COUNT.as_ref().val().load(SeqCst), 1);
    SIZE.as_ref().val().fetch_add(1 << 33, SeqCst);
    assert_eq!(SIZE.as_ref().val().load(SeqCst), 1 << 33);
}

/// `AtomicSsize` is `Atomic64` where pointers are 64 bits wide, so it holds any `isize`.
#[test]
#[cfg(target_pointer_width = "64")]
fn an_atomic_ssize_is_an_atomic64() {
    let size: Atomic64 = AtomicSsize::new(i64::try_from(isize::MAX).unwrap());
    assert_eq!(
        size.as_ref().val().load(SeqCst),
        i64::try_from(isize::MAX).unwrap()
    );
}

/// A value moves as its integer does, keeping it.
#[test]
fn a_value_moves_with_its_integer() {
    let counts: Vec<Atomic32> = (0..4).map(Atomic32::new).collect();
    let moved: Vec<Atomic32> = counts.into_iter().rev().collect();
    let read: Vec<i32> = moved
        .iter()
        .map(|c| c.as_ref().val().load(SeqCst))
        .collect();
    assert_eq!(read, [3, 2, 1, 0]);
}

/// A zeroed thread, as `Thread::default` and `Thread::zeroed` give it, stores the id 0, which
/// names no thread, as C's zeroed storage does before `git_thread_create`.
#[test]
fn a_zeroed_thread_stores_the_id_zero() {
    assert_eq!(Thread::default().as_ref().thread(), 0);
    assert_eq!(Thread::zeroed().as_ref().thread(), 0);
    let mut thread = Thread::default();
    let exclusive = thread.as_mut();
    let shared: ThreadRef<'_> = exclusive.as_ref();
    assert_eq!(shared.thread(), 0);
}

/// A thread's value moves as its id does, keeping it.
#[test]
fn a_thread_value_moves_with_its_id() {
    let threads: Vec<Thread> = (0..3).map(|_| Thread::default()).collect();
    let moved: Vec<Thread> = threads.into_iter().collect();
    assert!(moved.iter().all(|t| t.as_ref().thread() == 0));
}

/// A value, and each handle over it, prints the integer as the standard atomic does, and a
/// value converts from its integer as one does.
#[test]
fn atomics_print_and_convert_as_the_standard_atomics_do() {
    let mut count = Atomic32::from(-7);
    assert_eq!(
        format!("{count:?}"),
        format!("{:?}", core::sync::atomic::AtomicI32::new(-7))
    );
    assert_eq!(format!("{:?}", count.as_ref()), "-7");
    assert_eq!(format!("{:?}", count.as_mut()), "-7");
    count.as_ref().val().store(3, SeqCst);
    assert_eq!(format!("{count:?}"), "3");

    let mut size: Atomic64 = (1_i64 << 40).into();
    assert_eq!(format!("{size:?}"), "1099511627776");
    assert_eq!(format!("{:?}", size.as_mut()), "1099511627776");
    assert_eq!(
        format!("{:?}", AtomicSsize::from(i64::MIN)),
        i64::MIN.to_string()
    );
}

/// A thread, and each handle over it, prints the id it stores.
#[test]
fn a_thread_prints_its_id() {
    let mut thread = Thread::default();
    assert_eq!(format!("{thread:?}"), "Thread { thread: 0 }");
    assert_eq!(format!("{:?}", thread.as_ref()), "Thread { thread: 0 }");
    assert_eq!(format!("{:?}", thread.as_mut()), "Thread { thread: 0 }");
}

mod sync;
