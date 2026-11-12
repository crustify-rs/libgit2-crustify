//! A refused copy leaks what it had copied: `StrListFree::copy` and `NullStrListFree::copy`
//! call `git_strlist_copy` and `git_strlist_copy_with_null`, whose `GIT_ERROR_CHECK_ALLOC`
//! returns without releasing the array and the strings copied before the refused one.
//!
//! A libc-backed allocator that counts live allocations and refuses one request size is
//! installed before libgit2 is initialised; each copy of `["one", "four"]` is then refused
//! at the copy of `"four"` (a 5-byte request). Every allocation it made must be released.

use std::ffi::{CStr, c_void};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

use libgit2::libgit2::src::libgit2::settings::set_allocator;
use libgit2::libgit2::src::util::alloc::{Alloc, Allocator};
use libgit2::libgit2::src::util::errors::clear;
use libgit2::libgit2::src::util::strlist::{NullStrListFree, StrListFree};

unsafe extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

/// Allocations made and not yet released.
static LIVE: AtomicIsize = AtomicIsize::new(0);
/// The one request size refused; `usize::MAX` refuses none.
static REFUSE: AtomicUsize = AtomicUsize::new(usize::MAX);

struct Counting;

// SAFETY: forwards to libc's `malloc` family unchanged, which meets `Alloc`'s contract, but
// for the refused size, which it answers with `None`; the counters are atomics.
unsafe impl Alloc for Counting {
    fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
        if len == REFUSE.load(Ordering::SeqCst) {
            return None;
        }
        // SAFETY: libc's `malloc` takes any size.
        let p = NonNull::new(unsafe { malloc(len) }.cast())?;
        LIVE.fetch_add(1, Ordering::SeqCst);
        Some(p)
    }

    unsafe fn realloc(
        ptr: Option<NonNull<u8>>,
        len: usize,
        file: Option<&CStr>,
        line: i32,
    ) -> Option<NonNull<u8>> {
        let Some(ptr) = ptr else {
            return Self::malloc(len, file, line);
        };
        // SAFETY: `ptr` is a live allocation of libc's `malloc`, `len` non-zero.
        NonNull::new(unsafe { realloc(ptr.as_ptr().cast(), len) }.cast())
    }

    unsafe fn free(ptr: NonNull<u8>) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
        // SAFETY: `ptr` is a live allocation of libc's `malloc`, released once.
        unsafe { free(ptr.as_ptr().cast()) }
    }
}

static COUNTING: Allocator = Allocator::new::<Counting>();

fn main() {
    // SAFETY: installed before anything initialises libgit2, before any other thread runs.
    unsafe { set_allocator(Some(COUNTING.as_ref())) };
    // Initialises libgit2 and the thread's error state, which stay.
    drop(StrListFree::copy(&[c"warm"]).unwrap());
    drop(NullStrListFree::copy(&[c"warm"]).unwrap());
    clear();

    let strs = [c"one", c"four"];
    let mut leaked = Vec::new();
    for (name, copy) in [
        (
            "StrListFree::copy",
            (|s| StrListFree::copy(s).is_some()) as fn(&[&CStr]) -> bool,
        ),
        ("NullStrListFree::copy", |s| {
            NullStrListFree::copy(s).is_some()
        }),
    ] {
        let before = LIVE.load(Ordering::SeqCst);
        REFUSE.store(5, Ordering::SeqCst);
        assert!(!copy(&strs), "{name}: the copy of \"four\" is refused");
        REFUSE.store(usize::MAX, Ordering::SeqCst);
        clear();
        let live = LIVE.load(Ordering::SeqCst) - before;
        println!("{name}: {live} allocations left live by the refused copy");
        leaked.push((name, live));
    }
    for (name, live) in leaked {
        assert_eq!(
            live, 0,
            "{name}: the refused copy leaks the array and the copy of \"one\""
        );
    }
}
