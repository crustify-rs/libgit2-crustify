//! libgit2's core, ported to Rust.
//!
//! The crate serves two faces from the same safe Rust:
//!
//! - libgit2's C ABI, as `#[unsafe(no_mangle)]` exports in each module's
//!   `ffi` submodule, for the C that stays C and for C consumers (the
//!   exports are symbols, not Rust paths: C and Rust consumers that want the
//!   C ABI declare it in an `extern "C"` block, as C does); and
//! - a safe, idiomatic Rust API, re-exported from this root (and the
//!   [`opts`] module, libgit2's global options).
//!
//! Every module is crate-private: the public surface is exactly what this
//! root re-exports. No public item has a raw pointer, an `unsafe fn`, a C
//! type or a C-layout internal in its signature, so safe code cannot cause
//! undefined behaviour through it. See `rust/API.md` for a guide, and
//! `rust/PORTING.md` §5 for the conventions.
//!
//! The library initializes itself on first use and is never torn down by
//! the safe API (see [`Libgit2`]).

// Without `test-internals`, the internal modules' `pub` helpers that only
// the tests use (the port keeps libgit2's internal API whole) and their
// `pub use`s are unreachable; with it (every `cargo test`/`clippy
// --all-targets`), they are reachable through `__test`, so the lints see
// the same as before the modules became crate-private.
#![cfg_attr(not(feature = "test-internals"), allow(dead_code, unused_imports))]

// Every module is crate-private (the public surface is the re-exports
// below). The `#[macro_export]` macros of `util` keep their crate-root
// paths (`crate::git_error_set!` & co.), so the modules are declared here,
// not by a macro.
pub(crate) mod api;
pub(crate) mod blob;
pub(crate) mod buf;
pub(crate) mod cache;
pub(crate) mod commit;
pub(crate) mod commit_list;
pub(crate) mod config;
pub(crate) mod config_cache;
pub(crate) mod config_file;
pub(crate) mod config_list;
pub(crate) mod config_mem;
pub(crate) mod config_parse;
pub(crate) mod config_snapshot;
pub(crate) mod delta;
pub(crate) mod fs;
pub(crate) mod grafts;
pub(crate) mod hashmap_oid;
pub(crate) mod hashsig;
pub(crate) mod index;
pub(crate) mod libgit2;
pub(crate) mod mwindow;
pub(crate) mod object;
pub(crate) mod object_api;
pub(crate) mod odb;
pub(crate) mod odb_loose;
pub(crate) mod odb_mempack;
pub(crate) mod odb_pack;
pub(crate) mod oid;
pub(crate) mod oidarray;
pub(crate) mod pack;
pub(crate) mod parse;
pub(crate) mod refdb;
pub(crate) mod refdb_fs;
pub(crate) mod reflog;
pub(crate) mod refs;
pub(crate) mod refs_api;
pub(crate) mod repository;
pub(crate) mod revwalk;
pub(crate) mod settings;
pub(crate) mod signature;
pub(crate) mod strarray;
pub(crate) mod sys;
pub(crate) mod tag;
pub(crate) mod tree;
pub(crate) mod tree_cache;
pub(crate) mod util;

/// With the `test-internals` feature (for the crate's own integration tests
/// only), re-exports the public items of the internal modules under the
/// hidden, unstable `__test`.
macro_rules! test_internals {
    ($($name:ident),* $(,)?) => {
        /// The crate's internals, for its own integration tests (feature
        /// `test-internals`). Not part of the API: no stability, and its
        /// safe functions keep only the crate's internal contracts.
        #[cfg(feature = "test-internals")]
        #[doc(hidden)]
        pub mod __test {
            $( pub mod $name {
                #[allow(unused_imports)]
                pub use crate::$name::*;
            } )*
        }
    };
}

test_internals! {
    sys,
    util,
    api,
    blob,
    buf,
    cache,
    commit,
    commit_list,
    config,
    config_cache,
    config_file,
    config_list,
    config_mem,
    config_parse,
    config_snapshot,
    delta,
    fs,
    grafts,
    hashmap_oid,
    hashsig,
    index,
    libgit2,
    mwindow,
    object,
    object_api,
    odb,
    odb_loose,
    odb_mempack,
    odb_pack,
    oid,
    oidarray,
    pack,
    parse,
    refdb,
    refdb_fs,
    reflog,
    refs,
    refs_api,
    repository,
    revwalk,
    settings,
    signature,
    strarray,
    tag,
    tree,
    tree_cache,
}

pub use api::opts;

pub use api::error::{Error, ErrorClass, ErrorCode, Result};
pub use blob::Blob;
pub use commit::api::{Commit, CommitBuilder};
pub use config::{Config, ConfigBackend, ConfigEntries, ConfigEntry, ConfigLevel};
pub use hashsig::{HashSig, HashSigOptions};
pub use index::{Index, IndexEntry, IndexTime, Stage};
pub use libgit2::{Features, Libgit2};
pub use object::Object;
pub use odb::{
    Backend, ExpandId, LooseOptions, Mempack, ObjectType, Odb, OdbBackend, OdbObject, OdbReader,
    OdbWriter,
};
pub use oid::{Oid, OidShortener, OidType};
pub use refs_api::{RefdbBackend, Reference, ReferenceType, References, Reflog, ReflogEntry};
pub use repository::{Repository, RepositoryInitOptions, RepositoryOpenFlags, RepositoryState};
pub use revwalk::api::{Revwalk, Sort};
pub use signature::{Signature, SignatureRef, Time};
pub use tag::Tag;
pub use tree::api::{Tree, TreeBuilder, TreeEntry, TreeEntryRef};
