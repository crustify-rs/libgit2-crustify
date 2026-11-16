//! libgit2's core, ported to Rust.
//!
//! The crate serves two faces from the same safe Rust:
//!
//! - libgit2's C ABI, as `#[unsafe(no_mangle)]` exports in each module's
//!   `ffi` submodule, for the C that stays C and for C consumers; and
//! - a safe, idiomatic Rust API, re-exported from this root.
//!
//! See `rust/PORTING.md` for the design and the conventions every module
//! follows, and `src/util/README.md` for the foundation's API.

pub mod sys;
pub mod util;

pub mod buf;
pub mod strarray;

pub use strarray::StrArray;
pub use util::errors::{Error, ErrorClass, ErrorCode};
