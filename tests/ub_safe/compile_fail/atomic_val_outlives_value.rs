// The atomic an `Atomic32` lends cannot outlive the value it lends.
use libgit2_crustify::libgit2::src::util::thread::Atomic32;

fn main() {
    let val = {
        let count = Atomic32::new(1);
        count.as_ref().val()
    };
    let _ = val.load(core::sync::atomic::Ordering::SeqCst);
}
