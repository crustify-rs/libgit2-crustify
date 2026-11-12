//! `alloc.rs`'s module docs: a run whose size passes `isize::MAX` bytes "is refused before
//! C is asked", and "ffibox's own constructors of a run (`AllocVec::new_uninit` and its
//! siblings) refuse such a size without reporting it, as they ask libgit2 nothing". They do
//! ask: the size reaches libgit2's allocator through `git__malloc`.
//!
//! A libc-backed allocator that records the largest request is installed before libgit2 is
//! initialised; each constructor is then asked for `isize::MAX + 1` bytes.

use std::ffi::{CStr, c_void};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

use libgit2::libgit2::src::libgit2::settings::set_allocator;
use libgit2::libgit2::src::util::alloc::{Alloc, AllocVec, Allocator, ArrayFree};

unsafe extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

/// The largest request that reached the allocator.
static LARGEST: AtomicUsize = AtomicUsize::new(0);

struct Recording;

// SAFETY: forwards to libc's `malloc` family unchanged, which meets `Alloc`'s contract; the
// record is an atomic.
unsafe impl Alloc for Recording {
    fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
        LARGEST.fetch_max(len, Ordering::SeqCst);
        // SAFETY: plain libc allocation.
        NonNull::new(unsafe { malloc(len) }.cast())
    }

    unsafe fn realloc(
        p: Option<NonNull<u8>>,
        len: usize,
        _: Option<&CStr>,
        _: i32,
    ) -> Option<NonNull<u8>> {
        LARGEST.fetch_max(len, Ordering::SeqCst);
        let p = p.map_or(std::ptr::null_mut(), |p| p.as_ptr().cast());
        // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
        NonNull::new(unsafe { realloc(p, len) }.cast())
    }

    unsafe fn free(p: NonNull<u8>) {
        // SAFETY: a live libc block, given up.
        unsafe { free(p.as_ptr().cast()) }
    }
}

static RECORDING: Allocator = Allocator::new::<Recording>();

fn main() {
    // SAFETY: `Recording` meets `Alloc`'s contract; installed before anything initialises
    // libgit2, on the only thread.
    unsafe { set_allocator(Some(RECORDING.as_ref())) };
    let past = isize::MAX as usize + 1;
    let builds: [(&str, fn(usize) -> bool); 3] = [
        ("ArrayFree::new_uninit", |n| ArrayFree::new_uninit::<u8>(n).is_none()),
        ("AllocVec::new_uninit", |n| AllocVec::<u8>::new_uninit(n).is_none()),
        ("AllocVec::from_fn", |n| AllocVec::<u8>::from_fn(n, |_| 0).is_none()),
    ];
    for (what, build) in builds {
        assert!(build(past), "{what}");
        let largest = LARGEST.load(Ordering::SeqCst);
        assert!(largest < past, "{what}: libgit2's allocator was asked for {largest} bytes");
        println!("{what}: refused before libgit2 was asked");
    }
}
