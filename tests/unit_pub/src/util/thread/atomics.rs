//! `thread.h`'s atomic operations, ported: `Atomic32Ref`'s `get`, `set`, `inc`, `dec` and
//! `add`, `Atomic64Ref`'s `get`, `set` and `add` (`git_atomic_ssize`'s too), the pointer-sized
//! `atomic_compare_and_swap`, `atomic_swap` and `atomic_load`, and `memory_barrier`.
//!
//! Each case of `tests/libgit2/threads/atomic.c` is carried over by the test named after it,
//! with its inputs and assertions; the others hold the edges the C cases leave out: the values
//! each operation returns at the integer bounds, where the C builtins wrap, and the integer
//! slots of both signs. None of these reaches C, so Miri runs them all.

use core::ptr;
use core::sync::atomic::Ordering::SeqCst;
use core::sync::atomic::{AtomicIsize, AtomicPtr, AtomicUsize};

use libgit2_crustify::libgit2::src::util::thread::{
    Atomic32, Atomic64, AtomicSsize, atomic_compare_and_swap, atomic_load, atomic_swap,
    memory_barrier,
};

/// Carries over `test_threads_atomic__atomic32_set`.
#[test]
fn atomic32_set() {
    let v = Atomic32::new(0);
    v.as_ref().set(1);
    assert_eq!(v.as_ref().val().load(SeqCst), 1);
}

/// Carries over `test_threads_atomic__atomic32_get`.
#[test]
fn atomic32_get() {
    let v = Atomic32::new(1);
    assert_eq!(v.as_ref().get(), 1);
}

/// Carries over `test_threads_atomic__atomic32_inc`.
#[test]
fn atomic32_inc() {
    let v = Atomic32::new(0);
    assert_eq!(v.as_ref().inc(), 1);
    assert_eq!(v.as_ref().val().load(SeqCst), 1);
}

/// Carries over `test_threads_atomic__atomic32_add`.
#[test]
fn atomic32_add() {
    let v = Atomic32::new(0);
    assert_eq!(v.as_ref().add(1), 1);
    assert_eq!(v.as_ref().val().load(SeqCst), 1);
}

/// Carries over `test_threads_atomic__atomic32_dec`.
#[test]
fn atomic32_dec() {
    let v = Atomic32::new(1);
    assert_eq!(v.as_ref().dec(), 0);
    assert_eq!(v.as_ref().val().load(SeqCst), 0);
}

/// Carries over `test_threads_atomic__atomic64_set`.
#[test]
fn atomic64_set() {
    let v = Atomic64::new(0);
    v.as_ref().set(1);
    assert_eq!(v.as_ref().val().load(SeqCst), 1);
}

/// Carries over `test_threads_atomic__atomic64_get`.
#[test]
fn atomic64_get() {
    let v = Atomic64::new(1);
    assert_eq!(v.as_ref().get(), 1);
}

/// Carries over `test_threads_atomic__atomic64_add`.
#[test]
fn atomic64_add() {
    let v = Atomic64::new(0);
    assert_eq!(v.as_ref().add(1), 1);
    assert_eq!(v.as_ref().val().load(SeqCst), 1);
}

/// Carries over `test_threads_atomic__cas_pointer`.
#[test]
fn cas_pointer() {
    let mut newvalue1 = 1;
    let mut newvalue2 = 2;
    let value: AtomicPtr<i32> = AtomicPtr::new(ptr::null_mut());

    // value is updated
    assert!(atomic_compare_and_swap(&value, ptr::null_mut(), &raw mut newvalue1).is_null());
    assert_eq!(value.load(SeqCst), &raw mut newvalue1);

    // value is not updated
    assert_eq!(
        atomic_compare_and_swap(&value, ptr::null_mut(), &raw mut newvalue2),
        &raw mut newvalue1
    );
    assert_eq!(value.load(SeqCst), &raw mut newvalue1);
}

/// Carries over `test_threads_atomic__cas_intptr`.
#[test]
fn cas_intptr() {
    let value = AtomicIsize::new(0);

    // value is updated
    assert_eq!(atomic_compare_and_swap(&value, 0, 1), 0);
    assert_eq!(value.load(SeqCst), 1);

    // value is not updated
    assert_eq!(atomic_compare_and_swap(&value, 0, 2), 1);
    assert_eq!(value.load(SeqCst), 1);
}

/// Carries over `test_threads_atomic__swap`.
#[test]
fn swap() {
    let mut newvalue = 1;
    let value: AtomicPtr<i32> = AtomicPtr::new(ptr::null_mut());

    assert!(atomic_swap(&value, &raw mut newvalue).is_null());
    assert_eq!(value.load(SeqCst), &raw mut newvalue);

    assert_eq!(atomic_swap(&value, ptr::null_mut()), &raw mut newvalue);
    assert!(value.load(SeqCst).is_null());
}

/// Carries over `test_threads_atomic__load_ptr`.
#[test]
fn load_ptr() {
    let mut value = 1;
    let ptr = AtomicPtr::new(&raw mut value);
    assert_eq!(atomic_load(&ptr), &raw mut value);
}

/// Carries over `test_threads_atomic__load_intptr`.
#[test]
fn load_intptr() {
    let value = AtomicIsize::new(1);
    assert_eq!(atomic_load(&value), 1);
}

/// Each 32-bit operation returns the value it leaves, as C's `__atomic_*_fetch` builtins do,
/// and wraps at the bounds as they do, so that a sequence of them reads back what C's would.
#[test]
fn atomic32_operations_return_what_they_leave_and_wrap() {
    let v = Atomic32::new(i32::MAX);
    let v = v.as_ref();
    assert_eq!(v.inc(), i32::MIN);
    assert_eq!(v.dec(), i32::MAX);
    assert_eq!(v.add(2), i32::MIN + 1);
    assert_eq!(v.add(-2), i32::MAX);
    v.set(i32::MIN);
    assert_eq!(v.dec(), i32::MAX);
    v.set(-1);
    assert_eq!(v.inc(), 0);
    assert_eq!(v.dec(), -1);
    assert_eq!(v.add(i32::MIN), i32::MAX);
    assert_eq!(v.add(0), i32::MAX);
    assert_eq!(v.get(), i32::MAX);
}

/// As [`atomic32_operations_return_what_they_leave_and_wrap`], for 64 bits.
#[test]
fn atomic64_operations_return_what_they_leave_and_wrap() {
    let v = Atomic64::new(i64::MAX);
    let v = v.as_ref();
    assert_eq!(v.add(1), i64::MIN);
    assert_eq!(v.add(-1), i64::MAX);
    v.set(i64::MIN);
    assert_eq!(v.add(i64::MIN), 0);
    assert_eq!(v.add(-(1 << 40)), -(1 << 40));
    assert_eq!(v.get(), -(1 << 40));
}

/// A reference count rises and falls through the handles of one value, and the exclusive
/// handle's shared reborrow reaches the same operations.
#[test]
fn a_count_rises_and_falls_through_every_handle() {
    let mut refcount = Atomic32::default();
    assert_eq!(refcount.as_ref().inc(), 1);
    assert_eq!(refcount.as_ref().inc(), 2);
    let exclusive = refcount.as_mut();
    assert_eq!(exclusive.as_ref().dec(), 1);
    assert_eq!(exclusive.as_ref().get(), 1);
    assert_eq!(refcount.as_ref().dec(), 0);
}

/// `AtomicSsize`'s `git_atomic_ssize_add` and `git_atomic_ssize_get` are `Atomic64Ref`'s, so its
/// total holds any `isize`, as the object cache's storage count does.
#[test]
fn atomic_ssize_adds_and_reads_back() {
    static STORAGE: AtomicSsize = AtomicSsize::new(0);
    let storage = STORAGE.as_ref();
    let max = i64::try_from(isize::MAX).unwrap();
    assert_eq!(storage.add(max), max);
    assert_eq!(storage.add(-max), 0);
    assert_eq!(storage.add(-512), -512);
    assert_eq!(storage.get(), -512);
    storage.set(0);
    assert_eq!(storage.get(), 0);
}

/// The pointer-sized operations over an unsigned slot, across the bound of its sign, and a
/// compare-and-swap that finds the value it expects only once another one has been swapped in.
#[test]
fn unsigned_slots_compare_swap_and_load() {
    let slot = AtomicUsize::new(usize::MAX);
    assert_eq!(atomic_compare_and_swap(&slot, 0, 1), usize::MAX);
    assert_eq!(atomic_load(&slot), usize::MAX);
    assert_eq!(atomic_swap(&slot, 0), usize::MAX);
    assert_eq!(atomic_compare_and_swap(&slot, 0, 1), 0);
    assert_eq!(atomic_load(&slot), 1);
}

/// A compare-and-swap that finds its expected value stores the new one even when it is the
/// value it found, and returns it: the C's `foundval == oldval` case.
#[test]
fn a_compare_and_swap_to_the_same_value_succeeds() {
    let slot = AtomicIsize::new(-7);
    assert_eq!(atomic_compare_and_swap(&slot, -7, -7), -7);
    assert_eq!(atomic_load(&slot), -7);
    assert_eq!(atomic_compare_and_swap(&slot, isize::MIN, isize::MAX), -7);
    assert_eq!(atomic_load(&slot), -7);
}

/// `memory_barrier` is a fence and changes no value: what was stored before it is read after.
#[test]
fn a_memory_barrier_keeps_every_value() {
    let v = Atomic32::new(3);
    v.as_ref().set(4);
    memory_barrier();
    assert_eq!(v.as_ref().get(), 4);
}
