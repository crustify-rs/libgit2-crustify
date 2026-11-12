//! Unit tests of the public API: its contract checked through Rust assertions alone,
//! without comparing with C. Each tested TU gets a module, homed at its path below the C
//! root, in `tests/unit_pub/<TU path>.rs` (`src/libgit2/commit.rs` for
//! `src/libgit2/commit.c`).
//!
//! The suite uses the safe API alone and forbids `unsafe_code`.
#![forbid(unsafe_code)]

mod src;
