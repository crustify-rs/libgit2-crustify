//! A list of names stands in for a `Vec` of strings: `Names` owns its strings and array in
//! reftable's allocator, which any thread may release; `NamesVec` holds `&CStr` borrows and
//! the array of their pointers; `NamesRef` reads either, as a `&[&CStr]` does. Their
//! standard counterparts cross threads. This program moves a parsed list to another thread,
//! shares a built one and its borrowed view with two readers, and releases the parsed one
//! there; it compiles once the three types are `Send` and `Sync`.
#![forbid(unsafe_code)]

use std::thread;

use libgit2::libgit2::deps::reftable::basics::{NamesRef, NamesVec, parse_names};

fn count(names: NamesRef<'_>) -> usize {
    names.len()
}

fn main() {
    let parsed = parse_names(b"0x01-0x02-a.ref\n0x03-0x04-b.ref\n").expect("parses");
    let built = NamesVec::new([c"0x01-0x02-a.ref", c"0x03-0x04-b.ref"]);
    let view = built.as_ref();
    thread::scope(|scope| {
        let mover = scope.spawn(move || {
            let n = parsed.len();
            drop(parsed);
            n
        });
        let sharer = scope.spawn(|| count(built.as_ref()));
        let viewer = scope.spawn(move || count(view));
        assert_eq!(mover.join().expect("moves"), 2);
        assert_eq!(sharer.join().expect("shares"), 2);
        assert_eq!(viewer.join().expect("views"), 2);
    });
    println!("names crossed threads");
}
