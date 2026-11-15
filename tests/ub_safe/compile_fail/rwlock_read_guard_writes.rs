// A read guard lends its data shared only: it cannot write through it.
use libgit2_crustify::libgit2::src::util::thread::RwLock;

fn main() {
    let lock = RwLock::new(1);
    let guard = lock.read().unwrap();
    *guard = 2;
}
