//! `src/util/allocators/failalloc.c`: the unsafe methods the failing family's
//! strategies add, called within the contracts they state, edge cases included, and the
//! `unsafe extern "C"` members of the `git_allocator` built over the family, which are
//! C's own routines. One suite per method.
//!
//! The family never allocates, so the contracts that ask for one of its allocations
//! (`Alloc::free`, `Alloc::realloc` with `Some`, `CLenDrop::c_drop_len`) cannot be met
//! here; the equivalence suite probes those calls against C's. Every method calls the
//! family's C routines, so none of these runs under Miri.

use core::ffi::{CStr, c_char};
use core::ptr::{self, NonNull};

use ffibox::CDrop;
use libgit2::libgit2::src::util::alloc::{Alloc, Allocator};
use libgit2::libgit2::src::util::allocators::failalloc::FailAlloc;

/// The `git_allocator` over the family: C's three routines.
static FAIL: Allocator = Allocator::new::<FailAlloc>();

/// The sizes every allocation is asked for: empty, small, page-sized and unmeetable.
const SIZES: [usize; 5] = [0, 1, 24, 4096, usize::MAX];

/// The call sites a request may name: none, and one as libgit2 names them.
fn sites() -> [(*const c_char, i32); 2] {
    [(ptr::null(), 0), (c"src/util/alloc.h".as_ptr(), 21)]
}

// --- CDrop::c_drop ------------------------------------------------------------------

mod c_drop {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn releases_nothing_it_is_handed() {
        // `c_drop` asks for a live `T` whose claim the caller owns: a boxed value is one.
        // The family releases nothing, so the box is still whole afterwards, and the claim
        // is still the test's to settle.
        let boxed = Box::into_raw(Box::new([0x5Au8; 32]));
        let ptr = NonNull::new(boxed).unwrap();
        // SAFETY: `ptr` addresses a live `[u8; 32]` this test owns and hands over.
        unsafe { FailAlloc.c_drop(ptr) };
        // SAFETY: `FailAlloc` released and kept nothing, so `boxed` is still the box
        // `Box::into_raw` made, and still the test's alone.
        let back = unsafe { Box::from_raw(boxed) };
        assert_eq!(*back, [0x5A; 32]);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn releases_nothing_of_any_type() {
        // One impl serves objects, strings and storage alike.
        let mut text = *b"text\0";
        let mut slot = core::mem::MaybeUninit::<u64>::uninit();
        // SAFETY: each pointer addresses a live local this test owns for the call.
        unsafe {
            FailAlloc.c_drop(NonNull::from(&mut text).cast::<c_char>());
        }
        // SAFETY: as above.
        unsafe { FailAlloc.c_drop(NonNull::from(&mut slot)) };
        assert_eq!(&text, b"text\0");
    }
}

// --- CLenDrop::c_drop_len ----------------------------------------------------------

mod c_drop_len {
    use ffibox::CLenDrop;
    use libgit2::libgit2::src::util::allocators::failalloc::FailArrayAlloc;

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn releases_nothing_of_the_buffer() {
        // `c_drop_len` asks for a buffer whose claim the caller hands over, with its byte
        // length: a boxed slice is one. The family releases nothing, so it is whole after.
        for len in [1usize, 16, 4096] {
            let boxed = Box::into_raw(vec![0xA5u8; len].into_boxed_slice());
            // SAFETY: `boxed` addresses `len` live bytes this test owns and hands over.
            unsafe { FailArrayAlloc.c_drop_len(boxed.cast::<u8>(), len) };
            // SAFETY: `FailArrayAlloc` released and kept nothing, so `boxed` is still the
            // box `Box::into_raw` made, and still the test's alone.
            let back = unsafe { Box::from_raw(boxed) };
            assert!(back.iter().all(|&b| b == 0xA5));
        }
    }
}

// --- Alloc::realloc -----------------------------------------------------------------

mod realloc {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn fresh_requests_fail() {
        for len in SIZES {
            for file in [None, Some(c"src/util/alloc.h")] {
                // SAFETY: `None` asks for a fresh allocation, which any `len` may request.
                let got = unsafe { FailAlloc::realloc(None, len, file, 30) };
                assert_eq!(got, None, "realloc(None, {len})");
            }
        }
    }
}

// --- Alloc::malloc (safe), for completeness of the family -------------------------

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn malloc_fails_every_request() {
    for len in SIZES {
        assert_eq!(FailAlloc::malloc(len, None, 0), None);
        assert_eq!(FailAlloc::malloc(len, Some(c"x.c"), -1), None);
    }
}

// --- the git_allocator members ------------------------------------------------------

mod members {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn malloc_returns_null() {
        let malloc = FAIL.as_ref().malloc().unwrap();
        for len in SIZES {
            for (file, line) in sites() {
                // SAFETY: `file` is NULL or a C string live for the call.
                let p = unsafe { malloc(len, file, line) };
                assert!(p.is_null(), "malloc({len})");
            }
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn realloc_of_null_returns_null() {
        let realloc = FAIL.as_ref().realloc().unwrap();
        for len in SIZES {
            for (file, line) in sites() {
                // SAFETY: a NULL `ptr` allocates afresh, for any `size`; `file` as above.
                let p = unsafe { realloc(ptr::null_mut(), len, file, line) };
                assert!(p.is_null(), "realloc(NULL, {len})");
            }
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn free_of_null_is_a_no_op() {
        let free = FAIL.as_ref().free().unwrap();
        for _ in 0..3 {
            // SAFETY: NULL is always accepted.
            unsafe { free(ptr::null_mut()) };
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls C")]
    fn a_failed_request_leaves_nothing_to_release() {
        // libgit2's sequence on failure: ask, get NULL, release nothing (`git__free(NULL)`
        // on its error paths), ask again.
        let (malloc, realloc, free) = (
            FAIL.as_ref().malloc().unwrap(),
            FAIL.as_ref().realloc().unwrap(),
            FAIL.as_ref().free().unwrap(),
        );
        let file: *const c_char = CStr::as_ptr(c"src/libgit2/odb.c");
        for len in [16, 0, 1 << 20] {
            // SAFETY: `file` is a C string live for each call; NULL is any member's input.
            let p = unsafe { malloc(len, file, 1668) };
            assert!(p.is_null());
            // SAFETY: as above; `p` is NULL, so the request is a fresh one.
            let q = unsafe { realloc(p, len + 1, file, 83) };
            assert!(q.is_null());
            // SAFETY: NULL.
            unsafe { free(q) };
        }
    }
}
