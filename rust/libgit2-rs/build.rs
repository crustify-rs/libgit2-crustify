//! Links the C that stays in C when the `link-c` feature is on.
//!
//! The CMake build links the Rust port's static library into libgit2 itself,
//! so it needs nothing from here.  The crate's own tests and the Rust API's
//! consumers, which are linked by cargo, need the rest of libgit2's C: with
//! `--features link-c` and `LIBGIT2_BUILD_DIR` naming a CMake build directory
//! configured with `USE_RUST=ON`, this links `libgit2_c_rest.a` and the system
//! libraries it needs, as listed in `rust-link-libs.txt`.

use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-env-changed=LIBGIT2_BUILD_DIR");
    println!("cargo:rerun-if-changed=build.rs");
    if env::var_os("CARGO_FEATURE_LINK_C").is_none() {
        return;
    }
    let dir = env::var("LIBGIT2_BUILD_DIR").expect(
        "the link-c feature needs LIBGIT2_BUILD_DIR, a CMake build directory with USE_RUST=ON",
    );
    let dir = Path::new(&dir);
    let archive = dir.join("libgit2_c_rest.a");
    assert!(
        archive.exists(),
        "{} does not exist; build the git2_c_rest target",
        archive.display()
    );
    println!("cargo:rerun-if-changed={}", archive.display());
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rustc-link-lib=static:+whole-archive,-bundle=git2_c_rest");
    let libs = dir.join("rust-link-libs.txt");
    println!("cargo:rerun-if-changed={}", libs.display());
    for lib in fs::read_to_string(&libs)
        .expect("rust-link-libs.txt")
        .lines()
    {
        let lib = lib.trim();
        if lib.is_empty() {
            continue;
        }
        if lib.starts_with('/') {
            println!("cargo:rustc-link-arg={lib}");
        } else {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
    }
}
