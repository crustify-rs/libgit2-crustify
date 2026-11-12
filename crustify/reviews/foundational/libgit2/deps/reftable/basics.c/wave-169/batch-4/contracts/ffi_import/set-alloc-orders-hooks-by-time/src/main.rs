//! `set_alloc`'s `# Safety` asks that no other thread allocate, resize or release reftable
//! memory "during the call". This program honours that literally: its worker allocates half
//! a second after the call returned, no reftable allocation is live when the hooks change,
//! and libgit2's runtime stays initialised throughout. Nothing orders the worker's read of
//! the hooks after `reftable_set_alloc`'s write, so the two race, and TSan reports it.

use core::ffi::c_void;
use core::num::NonZeroUsize;
use core::ptr::{self, NonNull};
use std::thread;
use std::time::Duration;

use libgit2::libgit2::deps::reftable::basics::{Alloc, ReftableStr, set_alloc};
use libgit2::libgit2::src::libgit2::libgit2::init;

unsafe extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

/// libc's allocator.
struct Libc;

// SAFETY: forwards to libc's `malloc` family unchanged.
unsafe impl Alloc for Libc {
    fn malloc(len: NonZeroUsize) -> Option<NonNull<u8>> {
        // SAFETY: a plain libc allocation.
        NonNull::new(unsafe { malloc(len.get()) }.cast())
    }

    unsafe fn realloc(p: Option<NonNull<u8>>, len: NonZeroUsize) -> Option<NonNull<u8>> {
        let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
        // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
        NonNull::new(unsafe { realloc(p, len.get()) }.cast())
    }

    unsafe fn free(p: NonNull<u8>) {
        // SAFETY: `p` is a live libc block the caller gives up.
        unsafe { free(p.as_ptr().cast()) }
    }
}

fn main() {
    let _runtime = init().expect("libgit2 initialises");
    // Pins the crate's runtime here, and releases what it allocated: no reftable
    // allocation is live from now on.
    drop(ReftableStr::from_c_str(c"pin").expect("allocates"));
    let worker = thread::spawn(|| {
        // Well after `set_alloc` has returned.
        thread::sleep(Duration::from_millis(500));
        let s = ReftableStr::from_c_str(c"after").expect("allocates");
        assert_eq!(s.as_c_str(), c"after");
    });
    // SAFETY: no other thread allocates, resizes or releases reftable memory during the
    // call (the worker starts half a second after it); no reftable allocation is live, so
    // none crosses families; libgit2's runtime stays initialised while `Libc` is installed.
    unsafe { set_alloc::<Libc>() };
    worker.join().expect("the worker allocates");
    println!("no race reported");
}
