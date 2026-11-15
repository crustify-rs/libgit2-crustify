// A guard is not `Send`: the thread that took a lock releases it, as POSIX requires of a
// default mutex.
use libgit2_crustify::libgit2::src::util::thread::Mutex;

fn send<T: Send>(_: T) {}

fn main() {
    let lock = Mutex::new(1);
    send(lock.lock().unwrap());
}
