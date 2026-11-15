// A read guard cannot outlive the lock it holds.
use libgit2_crustify::libgit2::src::util::thread::RwLock;

fn main() {
    let guard = {
        let lock = RwLock::new(1);
        lock.read().unwrap()
    };
    let _ = *guard;
}
