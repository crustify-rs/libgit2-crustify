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
pub mod fs;
pub mod hashmap_oid;
pub mod oid;
pub mod oidarray;
pub mod parse;
pub mod refdb;
pub mod refdb_fs;
pub mod reflog;
pub mod refs;
pub mod strarray;

pub use oid::{Oid, OidShortener, OidType};
pub use oidarray::OidArray;
pub use strarray::StrArray;
pub use util::errors::{Error, ErrorClass, ErrorCode};
pub use util::net::Url;
pub use util::process::{Process, ProcessOptions, ProcessStatus};
pub use util::sortedcache::SortedCache;
