//! `src/util/allocators/stdalloc.c`: the value `stdalloc::allocator` returns, checked
//! through its handles alone: every member set, and the same functions on every call.

use std::ptr;

use libgit2::libgit2::src::util::allocators::stdalloc;

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn every_member_is_set() {
    let standard = stdalloc::allocator();
    let members = standard.as_ref();
    assert!(members.malloc().is_some());
    assert!(members.realloc().is_some());
    assert!(members.free().is_some());
    let shown = format!("{standard:?}");
    assert!(shown.starts_with("Allocator { malloc: 0x"), "{shown}");
    assert!(
        !shown.contains(": 0x0,") && !shown.contains(": 0x0 }"),
        "{shown}"
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn every_call_returns_the_same_functions() {
    let (a, b) = (stdalloc::allocator(), stdalloc::allocator());
    let (a, b) = (a.as_ref(), b.as_ref());
    assert!(ptr::fn_addr_eq(a.malloc().unwrap(), b.malloc().unwrap()));
    assert!(ptr::fn_addr_eq(a.realloc().unwrap(), b.realloc().unwrap()));
    assert!(ptr::fn_addr_eq(a.free().unwrap(), b.free().unwrap()));
}
