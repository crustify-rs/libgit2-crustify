// A lock cannot be destroyed while a guard holds it, which POSIX leaves undefined for
// `pthread_mutex_destroy` (`git_mutex_free`).
use libgit2_crustify::libgit2::src::util::thread::Mutex;

fn main() {
    let lock = Mutex::new(1);
    let guard = lock.lock().unwrap();
    drop(lock);
    let _ = *guard;
}
