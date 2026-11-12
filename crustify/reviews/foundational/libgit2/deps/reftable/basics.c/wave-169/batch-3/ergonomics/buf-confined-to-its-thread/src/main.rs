//! A reftable buffer filled on one thread is grown, read and released on another, and two
//! threads read one buffer at once, as a `Vec<u8>` allows, and as the `ReftableStr` that
//! `BufMut::detach` hands out already allows.
//!
//! On the affected commit this does not compile: `Buf` is neither `Send` nor `Sync`
//! (E0277, "`*const ()` cannot be sent between threads safely"), so `cargo run` exits
//! non-zero. Run from this directory:
//!
//! ```text
//! CRUSTIFY_BUILD_VARIANT=plain cargo run
//! ```
#![forbid(unsafe_code)]

use std::thread;

use libgit2::libgit2::deps::reftable::basics::{Buf, BufVal};

fn main() {
    let mut value: BufVal = Buf::new();
    value.as_mut().add(b"refs/heads/main").unwrap();
    let value = thread::spawn(move || {
        value.as_mut().add(b"-1").unwrap();
        value
    })
    .join()
    .unwrap();
    thread::scope(|scope| {
        for _ in 0..2 {
            scope.spawn(|| assert_eq!(value.as_ref().as_bytes(), b"refs/heads/main-1"));
        }
    });
    thread::spawn(move || drop(value)).join().unwrap();
}
