//! The C side of `tests/equiv/deps/reftable/basics.rs`, alone: a `reftable_buf` built and
//! grown through the raw bindings, as `RawBuf::new` and `apply`'s `Step::Add` (or `pair`)
//! build and grow one, each call under the suite's own `// SAFETY:` justification. Another
//! thread meanwhile makes the process's first use of a safe owner of reftable's memory,
//! which pins libgit2's runtime, as the suite's other tests do on the harness's other
//! threads.
//!
//! `reftable_buf_add` reads reftable's allocator hooks (`reftable_realloc`), and the first
//! `git_libgit2_init` writes them (`git_reftable_global_init` -> `reftable_set_alloc`),
//! both without synchronisation. Nothing orders the raw call after the pin, so the two
//! race, and TSan reports it.

use core::mem::MaybeUninit;
use std::thread;

use libgit2::ffi;
use libgit2::libgit2::deps::reftable::basics::ReftableStr;

fn main() {
    // Another test of the suite, on another harness thread: its first safe allocation
    // pins the runtime.
    let other = thread::spawn(|| {
        drop(ReftableStr::from_c_str(c"pins the runtime").expect("allocates"));
    });

    let mut raw = MaybeUninit::uninit();
    // SAFETY: `reftable_buf_init` writes the whole struct without reading it (the suite's
    // `RawBuf::new`).
    unsafe { ffi::reftable_buf_init(raw.as_mut_ptr()) };
    // SAFETY: initialised above.
    let mut raw: ffi::reftable_buf = unsafe { raw.assume_init() };
    let data = b"0x000000000001";
    // SAFETY: `raw` is a live buffer and `data` readable for its length (the suite's
    // `apply`, `Step::Add`).
    let rc = unsafe { ffi::reftable_buf_add(&raw mut raw, data.as_ptr().cast(), data.len()) };
    assert_eq!(rc, 0);

    other.join().expect("the other thread allocates");
    // SAFETY: the struct owns `buf`, released once here (the suite's `RawBuf::drop`).
    unsafe { ffi::reftable_buf_release(&raw mut raw) };
    println!("no race reported");
}
