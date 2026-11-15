// A `Thread`'s handles are not `Send`: neither is sent to another thread, so no thread reads
// or writes another's id through one.
use libgit2_crustify::libgit2::src::util::thread::Thread;

fn send<T: Send>(_: T) {}

fn main() {
    let mut thread = Thread::default();
    send(thread.as_ref());
    send(thread.as_mut());
}
