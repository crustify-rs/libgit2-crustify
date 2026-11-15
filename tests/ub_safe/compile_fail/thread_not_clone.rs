// A `Thread` stores an id that joins once, so it cannot be copied or cloned.
use libgit2_crustify::libgit2::src::util::thread::Thread;

fn main() {
    let thread = Thread::default();
    let _copy = thread.clone();
}
