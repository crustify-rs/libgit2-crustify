//! Equivalence of `thread.h`'s atomic operations with their ports, which C keeps `static inline`
//! in the header and the raw bindings reach through their compiled wrappers (`static_fns.c`,
//! `git_atomic32_inc__extern` and its siblings), each the header's body.
//!
//! - Sequences: one description of steps runs on a Rust value and on an independently owned C one,
//!   `git_atomic32`, `git_atomic64` or a `void *` or `intptr_t` slot, from each of the edge values;
//!   each step's return and the value it leaves are compared.
//! - Shared values: C and Rust operate on one object, the C through the handle's pointer, from many
//!   threads at once, as libgit2's C and its ported Rust will share a reference count; no update is
//!   lost, and each side reads what the other wrote.
//!
//! `GIT_MEMORY_BARRIER` and the three pointer macros (`git_atomic_compare_and_swap`,
//! `git_atomic_swap`, `git_atomic_load`) expand to no value the raw bindings reach: the macros
//! are compared through the functions they call, `git_atomic__compare_and_swap`,
//! `git_atomic__swap` and `git_atomic__load`, and the fence, which computes nothing, through no
//! function (`unit_pub` and `ub_safe` check it). None of these calls allocates or sets an error,
//! so no test initialises libgit2.

use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::Ordering::SeqCst;
use core::sync::atomic::{AtomicIsize, AtomicPtr};
use std::thread;

use ffibox::CRefSeam;
use libgit2_crustify::libgit2::src::util::thread::{
    Atomic32, Atomic32Ref, Atomic64, atomic_compare_and_swap, atomic_load, atomic_swap,
};
use libgit2_crustify::sys::libgit2 as sys;

/// One step of a sequence over a 32-bit value.
#[derive(Clone, Copy, Debug)]
enum Step32 {
    Get,
    Set(i32),
    Inc,
    Dec,
    Add(i32),
}

/// The edge values every sequence starts from.
const STARTS32: [i32; 7] = [0, 1, -1, i32::MAX, i32::MIN, i32::MAX - 1, i32::MIN + 1];

/// A sequence that crosses both bounds, adds 0, and sets and adds the edge values.
const STEPS32: [Step32; 16] = [
    Step32::Get,
    Step32::Inc,
    Step32::Dec,
    Step32::Dec,
    Step32::Add(0),
    Step32::Add(i32::MAX),
    Step32::Add(i32::MIN),
    Step32::Set(i32::MAX),
    Step32::Inc,
    Step32::Dec,
    Step32::Set(i32::MIN),
    Step32::Dec,
    Step32::Add(-2),
    Step32::Add(3),
    Step32::Set(0),
    Step32::Get,
];

/// What a step returns on the Rust side: `set` returns nothing, as C's does.
fn rust_step32(v: Atomic32Ref<'_>, step: Step32) -> Option<i32> {
    match step {
        Step32::Get => Some(v.get()),
        Step32::Set(val) => {
            v.set(val);
            None
        }
        Step32::Inc => Some(v.inc()),
        Step32::Dec => Some(v.dec()),
        Step32::Add(addend) => Some(v.add(addend)),
    }
}

/// What a step returns on the C side.
fn c_step32(v: &mut sys::git_atomic32, step: Step32) -> Option<i32> {
    let v = ptr::from_mut(v);
    match step {
        // SAFETY: each call is the header's body over `v`, a live `git_atomic32` this function
        // borrows exclusively, which the call reads and writes for its duration alone.
        Step32::Get => Some(unsafe { sys::git_atomic32_get(v) }),
        Step32::Set(val) => {
            // SAFETY: as above.
            unsafe { sys::git_atomic32_set(v, val) };
            None
        }
        // SAFETY: as above.
        Step32::Inc => Some(unsafe { sys::git_atomic32_inc(v) }),
        // SAFETY: as above.
        Step32::Dec => Some(unsafe { sys::git_atomic32_dec(v) }),
        // SAFETY: as above.
        Step32::Add(addend) => Some(unsafe { sys::git_atomic32_add(v, addend) }),
    }
}

/// `Atomic32Ref`'s operations return and leave what `git_atomic32_*` do, step by step, from
/// every edge value.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn atomic32_sequences_match_the_c() {
    for start in STARTS32 {
        let rust = Atomic32::new(start);
        let mut c = sys::git_atomic32 { val: start };
        for (i, step) in STEPS32.into_iter().enumerate() {
            let expected = c_step32(&mut c, step);
            assert_eq!(
                rust_step32(rust.as_ref(), step),
                expected,
                "{start}, step {i}: {step:?}"
            );
            assert_eq!(
                rust.as_ref().val().load(SeqCst),
                c.val,
                "{start}, step {i}: {step:?}"
            );
        }
    }
}

/// One step of a sequence over a 64-bit value.
#[derive(Clone, Copy, Debug)]
enum Step64 {
    Get,
    Set(i64),
    Add(i64),
}

/// `Atomic64Ref`'s operations, `git_atomic_ssize`'s too, return and leave what
/// `git_atomic64_*` do, step by step, from every edge value.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn atomic64_sequences_match_the_c() {
    let starts = [0, 1, -1, i64::MAX, i64::MIN, 1 << 40, -(1 << 40)];
    let steps = [
        Step64::Get,
        Step64::Add(1),
        Step64::Add(-1),
        Step64::Add(0),
        Step64::Add(i64::MAX),
        Step64::Add(i64::MIN),
        Step64::Set(i64::MAX),
        Step64::Add(1),
        Step64::Set(i64::MIN),
        Step64::Add(-1),
        Step64::Add(1 << 33),
        Step64::Set(0),
        Step64::Get,
    ];
    for start in starts {
        let rust = Atomic64::new(start);
        let mut c = sys::git_atomic64 { val: start };
        for (i, step) in steps.into_iter().enumerate() {
            let c_ptr = ptr::from_mut(&mut c);
            let (got, expected) = match step {
                // SAFETY: `c_ptr` addresses `c`, live and borrowed by no one else for the call.
                Step64::Get => (
                    Some(rust.as_ref().get()),
                    Some(unsafe { sys::git_atomic64_get(c_ptr) }),
                ),
                Step64::Set(val) => {
                    rust.as_ref().set(val);
                    // SAFETY: as above.
                    unsafe { sys::git_atomic64_set(c_ptr, val) };
                    (None, None)
                }
                Step64::Add(addend) => (
                    Some(rust.as_ref().add(addend)),
                    // SAFETY: as above.
                    Some(unsafe { sys::git_atomic64_add(c_ptr, addend) }),
                ),
            };
            assert_eq!(got, expected, "{start}, step {i}: {step:?}");
            assert_eq!(
                rust.as_ref().val().load(SeqCst),
                c.val,
                "{start}, step {i}: {step:?}"
            );
        }
    }
}

/// One step of a sequence over a pointer-sized slot, by the index of the value it uses.
#[derive(Clone, Copy, Debug)]
enum SlotStep {
    Load,
    Swap(usize),
    CompareAndSwap(usize, usize),
}

/// A sequence that swaps every value in and out, and compares against what the slot holds
/// and what it does not, the value itself included.
const SLOT_STEPS: [SlotStep; 12] = [
    SlotStep::Load,
    SlotStep::CompareAndSwap(0, 1),
    SlotStep::CompareAndSwap(0, 2),
    SlotStep::Load,
    SlotStep::Swap(2),
    SlotStep::CompareAndSwap(2, 2),
    SlotStep::CompareAndSwap(1, 3),
    SlotStep::Swap(0),
    SlotStep::CompareAndSwap(0, 3),
    SlotStep::Load,
    SlotStep::Swap(3),
    SlotStep::Load,
];

/// Runs [`SLOT_STEPS`] over a Rust slot and a C one, each starting from `values[start]`, by the
/// Rust operation and the C function given, comparing each step's return and the slots.
fn compare_slot_sequence<T: Copy + PartialEq + core::fmt::Debug>(
    values: [T; 4],
    start: usize,
    rust: &dyn Fn(SlotStep, [T; 4]) -> T,
    rust_slot: &dyn Fn() -> T,
    c: &mut dyn FnMut(SlotStep, [T; 4]) -> T,
    c_slot: &dyn Fn() -> T,
) {
    for (i, step) in SLOT_STEPS.into_iter().enumerate() {
        let expected = c(step, values);
        assert_eq!(
            rust(step, values),
            expected,
            "from {start}, step {i}: {step:?}"
        );
        assert_eq!(rust_slot(), c_slot(), "from {start}, step {i}: {step:?}");
    }
}

/// `atomic_compare_and_swap`, `atomic_swap` and `atomic_load` over an `AtomicPtr` return and
/// leave what `git_atomic__compare_and_swap`, `git_atomic__swap` and `git_atomic__load` do over
/// a `void *`, from every value, the null pointer among them. No pointer is followed.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn pointer_slot_sequences_match_the_c() {
    let mut targets = [0_u8; 3];
    let base = targets.as_mut_ptr().cast::<c_void>();
    let values: [*mut c_void; 4] = [
        ptr::null_mut(),
        base,
        base.wrapping_add(1),
        base.wrapping_add(2),
    ];
    for start in 0..values.len() {
        let rust = AtomicPtr::new(values[start]);
        let c = core::cell::Cell::new(values[start]);
        compare_slot_sequence(
            values,
            start,
            &|step, v| match step {
                SlotStep::Load => atomic_load(&rust),
                SlotStep::Swap(new) => atomic_swap(&rust, v[new]),
                SlotStep::CompareAndSwap(old, new) => {
                    atomic_compare_and_swap(&rust, v[old], v[new])
                }
            },
            &|| rust.load(SeqCst),
            &mut |step, v| {
                let slot = c.as_ptr();
                match step {
                    // SAFETY: `slot` addresses `c`'s `void *`, live for the call and reached by
                    // no one else during it; C reads and writes it, and follows no pointer it
                    // holds.
                    SlotStep::Load => unsafe { sys::git_atomic__load(slot) },
                    // SAFETY: as above.
                    SlotStep::Swap(new) => unsafe { sys::git_atomic__swap(slot, v[new]) },
                    SlotStep::CompareAndSwap(old, new) => {
                        // SAFETY: as above.
                        unsafe { sys::git_atomic__compare_and_swap(slot, v[old], v[new]) }
                    }
                }
            },
            &|| c.get(),
        );
    }
}

/// As [`pointer_slot_sequences_match_the_c`], over an `intptr_t` slot, which C passes the same
/// functions cast to `void * volatile *`, as `config_cache.c`'s cache is; the values cross the
/// sign.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn intptr_slot_sequences_match_the_c() {
    let values: [isize; 4] = [0, 1, -1, isize::MIN];
    for start in 0..values.len() {
        let rust = AtomicIsize::new(values[start]);
        let c = core::cell::Cell::new(values[start]);
        // The C functions take and return the slot's integer as a `void *`, by its address.
        let as_c = |v: isize| ptr::without_provenance_mut::<c_void>(v.cast_unsigned());
        let from_c = |p: *mut c_void| p.addr().cast_signed();
        compare_slot_sequence(
            values,
            start,
            &|step, v| match step {
                SlotStep::Load => atomic_load(&rust),
                SlotStep::Swap(new) => atomic_swap(&rust, v[new]),
                SlotStep::CompareAndSwap(old, new) => {
                    atomic_compare_and_swap(&rust, v[old], v[new])
                }
            },
            &|| rust.load(SeqCst),
            &mut |step, v| {
                let slot = c.as_ptr().cast::<*mut c_void>();
                let found = match step {
                    // SAFETY: `slot` addresses `c`'s `intptr_t`, which has a pointer's size and
                    // alignment, live for the call and reached by no one else during it; C reads
                    // and writes it as a `void *`, as `git_atomic_compare_and_swap` casts it,
                    // and follows nothing.
                    SlotStep::Load => unsafe { sys::git_atomic__load(slot) },
                    // SAFETY: as above.
                    SlotStep::Swap(new) => unsafe { sys::git_atomic__swap(slot, as_c(v[new])) },
                    SlotStep::CompareAndSwap(old, new) => {
                        // SAFETY: as above.
                        unsafe {
                            sys::git_atomic__compare_and_swap(slot, as_c(v[old]), as_c(v[new]))
                        }
                    }
                };
                from_c(found)
            },
            &|| c.get(),
        );
    }
}

/// Threads per shared-value test, and steps per thread.
const THREADS: usize = 4;
const STEPS: i32 = 2000;

/// The C pointer to the `git_atomic32` a shared handle lends, for C's atomic operations: the
/// handle of a value Rust holds addresses its `UnsafeCell`, through which atomic writes may go.
fn c_ptr(v: Atomic32Ref<'_>) -> *mut sys::git_atomic32 {
    v.as_ptr().cast_mut()
}

/// C threads and Rust threads raise and drop one reference count at once, half through
/// `git_atomic32_inc` and `git_atomic32_dec`, half through their ports, and lose no update: the
/// count ends where it started, and a final round of each side's increments is read back by the
/// other's `get`.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn c_and_rust_share_one_count() {
    let refcount = Atomic32::new(1);
    thread::scope(|scope| {
        for t in 0..THREADS {
            let handle = refcount.as_ref();
            scope.spawn(move || {
                for _ in 0..STEPS {
                    if t % 2 == 0 {
                        assert!(handle.inc() >= 2);
                        assert!(handle.dec() >= 1);
                    } else {
                        let c = c_ptr(handle);
                        // SAFETY: `c` addresses the live `git_atomic32` the handle lends, which
                        // every thread reaches only through atomic operations of its size.
                        assert!(unsafe { sys::git_atomic32_inc(c) } >= 2);
                        // SAFETY: as above.
                        assert!(unsafe { sys::git_atomic32_dec(c) } >= 1);
                    }
                }
            });
        }
    });
    let handle = refcount.as_ref();
    // SAFETY: as above, the threads joined.
    assert_eq!(unsafe { sys::git_atomic32_get(c_ptr(handle)) }, 1);
    assert_eq!(handle.add(41), 42);
    // SAFETY: as above.
    assert_eq!(unsafe { sys::git_atomic32_add(c_ptr(handle), 8) }, 50);
    assert_eq!(handle.get(), 50);
}

/// C threads and Rust threads race to install their own value in one empty slot, half through
/// `git_atomic__compare_and_swap`, half through `atomic_compare_and_swap`: exactly one wins, and
/// both sides' loads read the winner's value.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn c_and_rust_race_to_install_one_value() {
    let candidates = [0_u8; THREADS];
    let slot: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
    let wins: usize = thread::scope(|scope| {
        let racers: Vec<_> = candidates
            .iter()
            .enumerate()
            .map(|(t, mine)| {
                let slot = &slot;
                scope.spawn(move || {
                    let mine = ptr::from_ref(mine).cast_mut().cast::<c_void>();
                    let found = if t % 2 == 0 {
                        atomic_compare_and_swap(slot, ptr::null_mut(), mine)
                    } else {
                        // SAFETY: `slot.as_ptr()` addresses the live `void *` the `AtomicPtr`
                        // holds, which every thread reaches only atomically; C follows nothing.
                        unsafe {
                            sys::git_atomic__compare_and_swap(slot.as_ptr(), ptr::null_mut(), mine)
                        }
                    };
                    usize::from(found.is_null())
                })
            })
            .collect();
        racers.into_iter().map(|racer| racer.join().unwrap()).sum()
    });
    assert_eq!(wins, 1);
    // SAFETY: as above, the threads joined.
    let c_view = unsafe { sys::git_atomic__load(slot.as_ptr()) };
    assert_eq!(atomic_load(&slot), c_view);
    assert!(
        candidates
            .iter()
            .any(|c| ptr::from_ref(c).cast_mut().cast::<c_void>() == c_view)
    );
}
