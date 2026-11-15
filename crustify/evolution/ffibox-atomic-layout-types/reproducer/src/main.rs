//! A `define_ctype!` value Rust holds, written atomically through its shared handle, as an
//! atomic C field's accessor would write it.
use core::sync::atomic::{AtomicI32, Ordering::SeqCst};

use ffibox::{CRefSeam, CZeroable, define_ctype};

#[repr(C)]
pub struct atomic_st {
    pub val: i32,
}

define_ctype!(Atomic, AtomicRef, AtomicMut, atomic_st);
// SAFETY: one integer; zero is valid.
unsafe impl CZeroable for Atomic {}

fn main() {
    let count = Atomic::zeroed();
    let handle = count.as_ref();
    // SAFETY: the projection forms no reference.
    let val = unsafe { &raw mut (*handle.as_ptr().cast_mut()).val };
    // SAFETY: live and aligned; nothing else accesses it.
    let atomic = unsafe { AtomicI32::from_ptr(val) };
    atomic.fetch_add(1, SeqCst);
    println!("{}", atomic.load(SeqCst));
}
