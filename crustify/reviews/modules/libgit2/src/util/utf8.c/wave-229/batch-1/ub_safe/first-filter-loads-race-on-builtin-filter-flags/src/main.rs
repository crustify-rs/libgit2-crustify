//! Two threads, each with a repository of its own, hash a file that a `* text` attribute
//! selects the `crlf` filter for, at once: the process's first filter-list loads. libgit2
//! writes the built-in filter's `initialized` flag (`src/libgit2/filter.c:344`) under only
//! its registry's read lock, and the other load reads it (`:569`), unordered: a data race,
//! from the safe API alone.
//!
//! Run from the repository root, under the TSan variant, with the batch's caps:
//!
//! ```text
//! CARGO_TARGET_DIR=crustify/tmp/target-repro-tsan crustify/scripts/cargo-variant.sh tsan \
//!   run --manifest-path <this dir>/Cargo.toml
//! ```
//!
//! It exits 66 (TSan's `halt_on_error`) on the affected commit and 0 once the runtime's
//! pinning initialises the built-in filters.

#![forbid(unsafe_code)]

use std::ffi::CString;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::{fs, thread};

use libgit2::libgit2::src::libgit2::object::ObjectType;
use libgit2::libgit2::src::libgit2::repository::Repository;

/// A fresh repository under the system's temporary dir, whose `.gitattributes` selects
/// `crlf` for every path, with one file to hash.
fn workdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "crustify-filter-race-{}-{name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temporary dir");
    fs::write(dir.join(".gitattributes"), "* text\n").expect(".gitattributes");
    fs::write(dir.join("a.txt"), "one\r\ntwo\r\n").expect("a.txt");
    dir
}

fn main() {
    let barrier = Arc::new(Barrier::new(2));
    let threads: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|name| {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let dir = workdir(name);
                let path = CString::new(dir.to_str().expect("UTF-8 path")).expect("no NUL");
                let repo = Repository::init(&path, false).expect("init");
                barrier.wait();
                let id = repo.as_ref().hash_file(c"a.txt", ObjectType::Blob, None);
                drop(repo);
                let _ = fs::remove_dir_all(&dir);
                id.expect("hashed")
            })
        })
        .collect();
    let ids: Vec<_> = threads
        .into_iter()
        .map(|t| t.join().expect("no panic"))
        .collect();
    assert_eq!(ids[0], ids[1], "one content, one id");
    println!("hashed {:?} on both threads", ids[0]);
}
