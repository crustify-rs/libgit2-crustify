//! Unit tests of the public interface: the Rust contract of the safe API, decided by Rust
//! assertions without comparing with the C. Each tested TU gets a module, homed at its path
//! below the C root, in `tests/unit_pub/<TU path>.rs` (`deps/reftable/basics.rs` for
//! `deps/reftable/basics.c`).
//!
//! The suite uses the safe API alone, as a consumer of the crate does.
#![forbid(unsafe_code)]

mod deps;
