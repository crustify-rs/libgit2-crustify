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
    build_config();
    if env::var_os("CARGO_FEATURE_LINK_C").is_none() {
        // Without the C rest, the Rust still calls the system libraries the
        // ported util layer uses directly (zlib's deflate, PCRE2); link them
        // so that the crate's own tests link (with `link-c` they come from
        // `rust-link-libs.txt`).
        println!("cargo:rustc-link-lib=dylib=z");
        println!("cargo:rustc-link-lib=dylib=pcre2-8");
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

/// The features (`git2_features.h` of `LIBGIT2_BUILD_DIR`, or the reference
/// configuration's when there is none, as under Miri) and the version
/// (`include/git2/version.h`) that `libgit2.c`'s port reports: each
/// `#define GIT_<NAME> 1` becomes the cfg `git_feature = "<NAME>"`, and
/// `GIT_BUILD_CPU` / `GIT_BUILD_COMMIT` and the version numbers become
/// environment variables of the crate.
fn build_config() {
    const REFERENCE: &[&str] = &[
        "THREADS",
        "THREADS_PTHREADS",
        "SHA1_BUILTIN",
        "SHA256_OPENSSL",
        "COMPRESSION_ZLIB",
        "NSEC",
        "NSEC_MTIM",
        "REGEX_PCRE2",
        "SSH",
        "SSH_LIBSSH2",
        "HTTP",
        "HTTPS",
        "HTTPS_OPENSSL",
        "HTTPPARSER_HTTPPARSER",
        "AUTH_NTLM",
        "AUTH_NTLM_BUILTIN",
        "AUTH_NEGOTIATE",
        "AUTH_NEGOTIATE_GSSAPI",
    ];
    println!("cargo::rustc-check-cfg=cfg(git_feature, values(any()))");
    println!("cargo::rustc-check-cfg=cfg(git_build_cpu)");
    println!("cargo::rustc-check-cfg=cfg(git_build_commit)");
    let mut features: Vec<String> = Vec::new();
    let mut cpu = None;
    let mut commit = None;
    let header = env::var_os("LIBGIT2_BUILD_DIR")
        .map(|d| Path::new(&d).join("gen_headers").join("git2_features.h"));
    match header.as_deref().and_then(|h| {
        println!("cargo:rerun-if-changed={}", h.display());
        fs::read_to_string(h).ok()
    }) {
        Some(text) => {
            for line in text.lines() {
                let mut w = line.split_whitespace();
                if w.next() != Some("#define") {
                    continue;
                }
                let (Some(name), value) = (w.next(), w.collect::<Vec<_>>().join(" ")) else {
                    continue;
                };
                match name {
                    "GIT_BUILD_CPU" => cpu = Some(value.trim_matches('"').to_string()),
                    "GIT_BUILD_COMMIT" => commit = Some(value.trim_matches('"').to_string()),
                    _ => {
                        if let Some(n) = name.strip_prefix("GIT_")
                            && value == "1"
                        {
                            features.push(n.to_string());
                        }
                    }
                }
            }
        }
        None => {
            features = REFERENCE.iter().map(|s| s.to_string()).collect();
            cpu = Some("x86_64".to_string());
        }
    }
    for f in &features {
        println!("cargo:rustc-cfg=git_feature=\"{f}\"");
    }
    if let Some(c) = cpu {
        println!("cargo:rustc-env=LIBGIT2_BUILD_CPU={c}");
        println!("cargo:rustc-cfg=git_build_cpu");
    }
    if let Some(c) = commit {
        println!("cargo:rustc-env=LIBGIT2_BUILD_COMMIT={c}");
        println!("cargo:rustc-cfg=git_build_commit");
    }

    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let version_h = Path::new(&manifest).join("../../include/git2/version.h");
    println!("cargo:rerun-if-changed={}", version_h.display());
    let text = fs::read_to_string(&version_h).expect("include/git2/version.h");
    for line in text.lines() {
        let mut w = line.split_whitespace();
        if w.next() != Some("#define") {
            continue;
        }
        let Some(name) = w.next() else { continue };
        let value = w.collect::<Vec<_>>().join(" ");
        match name {
            "LIBGIT2_VERSION"
            | "LIBGIT2_VERSION_MAJOR"
            | "LIBGIT2_VERSION_MINOR"
            | "LIBGIT2_VERSION_REVISION"
            | "LIBGIT2_VERSION_PRERELEASE" => {
                println!("cargo:rustc-env={name}={}", value.trim_matches('"'));
            }
            _ => {}
        }
    }
}
