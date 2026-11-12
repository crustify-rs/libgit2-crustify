//! UB tests of `Allocator::debug` (`src/util/allocators/debugalloc.c`) on the safe API.
//!
//! The value is built by C writing into a local and is then plain data: these tests build
//! it on many threads at once, before and while libgit2 runs, and share and copy it across
//! threads, for `ASan` and `TSan` to catch a write C makes outside the value or a race on
//! state C would share. That a handle cannot outlive its value is the compile-fail doctest
//! of the module's documentation.

use std::sync::{Barrier, OnceLock};
use std::thread;

use libgit2::libgit2::src::util::alloc::{Allocator, StrFree};

/// The three members' addresses.
fn addresses(a: Allocator) -> [usize; 3] {
    let h = a.as_ref();
    [
        h.malloc().map(|f| f as usize).unwrap(),
        h.realloc().map(|f| f as usize).unwrap(),
        h.free().map(|f| f as usize).unwrap(),
    ]
}

const THREADS: usize = 8;

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn debug_built_on_many_threads_at_once() {
    let start = Barrier::new(THREADS);
    let built: Vec<[usize; 3]> = thread::scope(|s| {
        let handles: Vec<_> = (0..THREADS)
            .map(|i| {
                let start = &start;
                s.spawn(move || {
                    start.wait();
                    // Half the threads also run libgit2 meanwhile, which the routine must
                    // not disturb: it touches no state of libgit2's.
                    if i % 2 == 0 {
                        let copy = StrFree::dup(c"concurrent").unwrap();
                        assert_eq!(copy.to_bytes(), b"concurrent");
                    }
                    let mut last = addresses(Allocator::debug());
                    for _ in 0..64 {
                        let again = addresses(Allocator::debug());
                        assert_eq!(again, last);
                        last = again;
                    }
                    last
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(built.windows(2).all(|w| w[0] == w[1]));
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn debug_shared_and_copied_across_threads() {
    static SHARED: OnceLock<Allocator> = OnceLock::new();
    let expected = addresses(*SHARED.get_or_init(Allocator::debug));
    thread::scope(|s| {
        for _ in 0..THREADS {
            s.spawn(move || {
                // Readers of the shared value, through its shared handle, race nothing.
                let shared = SHARED.get().unwrap();
                let handle = shared.as_ref();
                assert!(handle.malloc().is_some());
                assert_eq!(addresses(*shared), expected);
                // A copy moved to this thread is its own: changing it leaves the shared
                // value alone.
                let mut own = *shared;
                let free = Allocator::debug().as_ref().free().unwrap();
                own.as_mut().set_free(free);
                assert_eq!(addresses(own), expected);
            });
        }
    });
    assert_eq!(addresses(*SHARED.get().unwrap()), expected);
}
