// The guard a wait returns holds the same lock, and cannot outlive it either.
use libgit2_crustify::libgit2::src::util::thread::{Condvar, Mutex};

fn main() {
    let cond = Condvar::new();
    let guard = {
        let lock = Mutex::new(false);
        cond.wait(lock.lock().unwrap()).unwrap()
    };
    let _ = *guard;
}
