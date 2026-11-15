//! A lock over a borrow, declared before what it borrows, as `std::sync::Mutex` and
//! `std::sync::RwLock` allow: `thread.rs`'s locks refuse it while an empty `impl Drop`
//! carries their `_free` macros' anchors (E0597).
#![forbid(unsafe_code)]

use libgit2::libgit2::src::util::thread::{Mutex, RwLock};

fn main() {
    let mutex;
    let rwlock;
    let value = 7;
    mutex = Mutex::new(&value);
    rwlock = RwLock::new(&value);
    assert_eq!(**mutex.lock().unwrap(), 7);
    assert_eq!(**rwlock.read().unwrap(), 7);
    println!("a lock over a borrow drops after what it borrows");
}
