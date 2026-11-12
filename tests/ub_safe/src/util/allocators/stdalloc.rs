//! `src/util/allocators/stdalloc.c`: `stdalloc::allocator` driven through the safe API
//! alone. C fills a `git_allocator` on the caller's frame, so the tests build the value
//! many times, from many threads at once, and copy, change and drop it on each: `ASan` flags
//! a write past the frame's storage, `UBSan` a member C left unset that Rust would read as
//! a function, and `TSan` a write C shares between callers.

use std::sync::Barrier;
use std::thread;

use libgit2::libgit2::src::util::alloc::Allocator;
use libgit2::libgit2::src::util::allocators::stdalloc;

/// The members' addresses, the only part of a value safe code can observe beyond calling them.
fn members(a: Allocator) -> [usize; 3] {
    let r = a.as_ref();
    [
        r.malloc().map_or(0, |f| f as usize),
        r.realloc().map_or(0, |f| f as usize),
        r.free().map_or(0, |f| f as usize),
    ]
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn built_on_many_threads_at_once_alike() {
    const THREADS: usize = 8;
    let expected = members(stdalloc::allocator());
    assert!(expected.iter().all(|&a| a != 0));
    let start = Barrier::new(THREADS);
    let got: Vec<[usize; 3]> = thread::scope(|s| {
        let workers: Vec<_> = (0..THREADS)
            .map(|_| {
                s.spawn(|| {
                    start.wait();
                    (0..256)
                        .map(|_| members(stdalloc::allocator()))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect()
    });
    assert_eq!(got.len(), THREADS * 256);
    assert!(got.iter().all(|m| *m == expected));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn values_cross_threads_and_change_apart() {
    let standard = stdalloc::allocator();
    let expected = members(standard);
    // A value built on one thread and moved to another, as `set_allocator`'s caller may.
    let moved = thread::spawn(stdalloc::allocator).join().unwrap();
    assert_eq!(members(moved), expected);

    // A copy changed through its handle and a reborrow of it, from this thread. The safe API
    // offers no functions but the standard ones, so the members stay what they were; the
    // inline unit test `a_changed_copy_leaves_the_standard_value` replaces them with others.
    let mut changed = standard;
    let realloc = standard.as_ref().realloc().unwrap();
    let mut handle = changed.as_mut();
    handle.set_realloc(realloc);
    handle
        .as_mut()
        .set_free(stdalloc::allocator().as_ref().free().unwrap());
    assert_eq!(members(changed), expected);
    assert_eq!(members(standard), expected);
    assert_eq!(
        format!("{changed:?}"),
        format!("{:?}", stdalloc::allocator())
    );
}
