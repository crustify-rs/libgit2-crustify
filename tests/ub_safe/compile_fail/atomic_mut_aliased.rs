// A shared handle cannot coexist with the exclusive one of the same `Atomic64`.
use libgit2_crustify::libgit2::src::util::thread::Atomic64;

fn main() {
    let mut size = Atomic64::new(0);
    let shared = size.as_ref();
    let exclusive = size.as_mut();
    let _ = (shared.val(), exclusive);
}
