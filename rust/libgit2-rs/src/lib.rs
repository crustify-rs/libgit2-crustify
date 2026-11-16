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
pub mod cache;
pub mod delta;
pub mod fs;
pub mod hashmap_oid;
pub mod libgit2;
pub mod mwindow;
pub mod odb;
pub mod odb_loose;
pub mod odb_mempack;
pub mod odb_pack;
pub mod oid;
pub mod oidarray;
pub mod pack;
pub mod parse;
pub mod repository;
pub mod settings;
pub mod strarray;

pub use libgit2::Libgit2;
pub use odb::{
    Backend, ExpandId, LooseOptions, Mempack, ObjectType, Odb, OdbBackend, OdbObject, OdbReader,
    OdbWriter,
};
pub use oid::{Oid, OidShortener, OidType};
pub use oidarray::OidArray;
pub use repository::{Repository, RepositoryInitOptions, RepositoryOpenFlags, RepositoryState};
pub use strarray::StrArray;
pub use util::errors::{Error, ErrorClass, ErrorCode};
pub use util::net::Url;
pub use util::process::{Process, ProcessOptions, ProcessStatus};
pub use util::sortedcache::SortedCache;
