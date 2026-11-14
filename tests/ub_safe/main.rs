//! UB tests that target the safe public API. Each tested TU gets a module, homed at
//! its path below the C root, in `tests/ub_safe/<TU path>.rs` (`src/libgit2/commit.rs` for
//! `src/libgit2/commit.c`); the tests of a TU's `sys/` header are its `sys` module.
//!
//! The suite uses the safe API alone and forbids `unsafe_code`: a type that no safe
//! constructor or producer yields yet gets its tests here once one is wrapped. libgit2's
//! state is process-global, so tests needing it untouched rerun alone in a child process.
//!
//! Miri cannot call libgit2, so every test that allocates through it is ignored
//! there; the Rust-only paths (empty buffers, rejected sizes) still run.
#![forbid(unsafe_code)]

mod deps;
mod fixture;
mod src;

/// Illegal safe paths that must fail to compile, one file each under `compile_fail/`, and
/// those whose diagnostics the pinned stable and nightly word differently, each written once
/// under `compile_fail/toolchain/` and checked against the expectation of the toolchain
/// trybuild compiles with, under `compile_fail/<channel>/`.
#[test]
#[cfg_attr(miri, ignore = "trybuild runs cargo")]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ub_safe/compile_fail/*.rs");
    t.compile_fail(format!("tests/ub_safe/compile_fail/{}/*.rs", channel()));
}

/// The channel, `stable` or `nightly`, of the compiler trybuild's cargo runs: the `RUSTC`
/// cargo would take, or the `rustc` this process resolves, as cargo resolves it.
fn channel() -> &'static str {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let out = std::process::Command::new(rustc)
        .arg("-vV")
        .output()
        .unwrap();
    assert!(out.status.success(), "rustc -vV failed");
    let release = String::from_utf8(out.stdout).unwrap();
    let release = release
        .lines()
        .find_map(|line| line.strip_prefix("release: "))
        .unwrap()
        .to_owned();
    if release.ends_with("-nightly") {
        "nightly"
    } else {
        "stable"
    }
}
