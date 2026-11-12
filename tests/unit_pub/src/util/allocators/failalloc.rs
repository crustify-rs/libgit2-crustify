//! `src/util/allocators/failalloc.c`: the failing family's policies keep their contract,
//! checked in Rust alone: every allocation they are asked for fails, whatever its size or
//! call site, and the policies are plain, stateless values.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ffibox::{CAlloc, CLenAlloc};
use libgit2::libgit2::src::util::alloc::{Alloc, Allocator};
use libgit2::libgit2::src::util::allocators::failalloc::{FailAlloc, FailArrayAlloc};

/// A layout type for the singleton policy to allocate; any `CCell` does.
#[repr(C)]
#[expect(
    unreachable_pub,
    reason = "`define_ctype!` makes its newtype `pub`, which names this layout"
)]
pub struct probe_st {
    pub value: u64,
}
ffibox::define_ctype!(Probe, ProbeRef, ProbeMut, probe_st);

/// The sizes every allocation is asked for: empty, small, page-sized and unmeetable.
const SIZES: [usize; 5] = [0, 1, 24, 4096, usize::MAX];

/// The `git_allocator` over the family, built at compile time.
static FAIL: Allocator = Allocator::new::<FailAlloc>();

/// A policy as `Default` builds it, as an owner's `new` builds its policy.
fn defaulted<T: Default>() -> T {
    T::default()
}

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut h = DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

#[test]
fn singleton_allocation_fails() {
    assert!(<FailAlloc as CAlloc<Probe>>::c_alloc(&FailAlloc).is_none());
    assert!(<FailAlloc as CAlloc<Probe>>::c_alloc(&defaulted()).is_none());
}

#[test]
fn array_allocation_fails_whatever_its_size() {
    for len in SIZES.into_iter().filter(|&n| n != 0) {
        assert!(
            FailArrayAlloc.c_alloc_len(len).is_none(),
            "c_alloc_len({len})"
        );
    }
    assert_eq!(<FailArrayAlloc as CLenAlloc>::ALIGN, 16);
}

#[test]
fn alloc_malloc_fails_whatever_its_call_site() {
    for len in SIZES {
        assert_eq!(FailAlloc::malloc(len, None, 0), None, "malloc({len})");
        assert_eq!(FailAlloc::malloc(len, Some(c"src/util/str.c"), 49), None);
        assert_eq!(FailAlloc::malloc(len, Some(c""), i32::MIN), None);
    }
}

#[test]
fn the_allocator_sets_every_member() {
    let fail = Allocator::new::<FailAlloc>();
    let members = fail.as_ref();
    assert!(members.malloc().is_some() && members.realloc().is_some() && members.free().is_some());
    // A value copied out of a static reads the same members.
    let copy = FAIL;
    assert!(copy.as_ref().malloc().is_some());
}

#[test]
fn policies_are_plain_values() {
    assert_eq!(format!("{FailAlloc:?}"), "FailAlloc");
    assert_eq!(format!("{FailArrayAlloc:?}"), "FailArrayAlloc");
    let (a, b): (_, FailAlloc) = (FailAlloc, defaulted());
    assert_eq!(a, b);
    assert_eq!(hash_of(&a), hash_of(&b));
    let (c, d): (_, FailArrayAlloc) = (FailArrayAlloc, defaulted());
    assert_eq!(c, d);
    assert_eq!(hash_of(&c), hash_of(&d));
    assert_eq!(size_of::<FailAlloc>() + size_of::<FailArrayAlloc>(), 0);
}

#[test]
fn policies_cross_threads() {
    fn shareable<T: Send + Sync + Unpin + 'static>() {}
    shareable::<FailAlloc>();
    shareable::<FailArrayAlloc>();
    let failed = std::thread::spawn(|| FailAlloc::malloc(8, None, 0).is_none());
    assert!(failed.join().unwrap());
}
