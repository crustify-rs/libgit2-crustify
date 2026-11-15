// A join through a `Thread` follows the id it stores, so storing one is `unsafe`.
use libgit2_crustify::libgit2::src::util::thread::Thread;

fn main() {
    let mut thread = Thread::default();
    thread.as_mut().set_thread(1);
}
