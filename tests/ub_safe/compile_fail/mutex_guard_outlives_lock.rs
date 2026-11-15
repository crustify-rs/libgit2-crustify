// A guard cannot outlive the lock it holds.
use libgit2_crustify::libgit2::src::util::thread::Mutex;

fn main() {
    let guard = {
        let lock = Mutex::new(1);
        lock.lock().unwrap()
    };
    let _ = *guard;
}
