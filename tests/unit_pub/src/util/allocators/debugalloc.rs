//! Unit tests of `Allocator::debug` (`src/util/allocators/debugalloc.c`): the value it
//! builds holds the family's routines, every member set, the same on every call.

use std::ptr;

use libgit2::libgit2::src::util::alloc::Allocator;

/// The three members' addresses, each read once from one value.
fn addresses(a: Allocator) -> [usize; 3] {
    let h = a.as_ref();
    [
        h.malloc().map(|f| f as usize).unwrap(),
        h.realloc().map(|f| f as usize).unwrap(),
        h.free().map(|f| f as usize).unwrap(),
    ]
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn debug_sets_every_member_to_one_family() {
    let debug = Allocator::debug();
    let handle = debug.as_ref();
    assert!(handle.malloc().is_some() && handle.realloc().is_some() && handle.free().is_some());
    // Three distinct routines: `malloc`, `realloc` and `free`.
    let [malloc, realloc, free] = addresses(debug);
    assert!(malloc != realloc && realloc != free && malloc != free);
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn debug_returns_the_same_routines_every_time() {
    let first = Allocator::debug();
    let free = first.as_ref().free().unwrap();
    for _ in 0..4 {
        let again = Allocator::debug();
        assert!(ptr::fn_addr_eq(again.as_ref().free().unwrap(), free));
        assert_eq!(addresses(again), addresses(first));
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn debug_value_is_a_plain_copy() {
    let a = Allocator::debug();
    let mut b = a;
    // Replacing a member of the copy leaves the original whole.
    let other = Allocator::debug();
    b.as_mut().set_free(other.as_ref().free().unwrap());
    assert_eq!(addresses(a), addresses(b));
    // Its `Debug` names three set members, none of them NULL.
    let shown = format!("{a:?}");
    assert!(shown.starts_with("Allocator {"), "{shown}");
    assert_eq!(shown.matches("0x").count(), 3, "{shown}");
    assert!(
        !shown.contains("0x0,") && !shown.contains("0x0 }"),
        "{shown}"
    );
}
