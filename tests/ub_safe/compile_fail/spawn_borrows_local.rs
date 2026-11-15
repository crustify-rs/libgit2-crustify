// A thread `spawn` starts owns what it captures: it cannot borrow its caller's data, which
// may be gone before it runs.
use libgit2_crustify::libgit2::src::util::thread::spawn;

fn main() {
    let data = vec![1, 2, 3];
    let thread = spawn(|| data.len()).unwrap();
    let _ = thread.join();
}
