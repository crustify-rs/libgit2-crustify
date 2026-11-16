//! Declarations of the C that stays C, with safe wrappers (PORTING.md §4.6).

pub mod annotated_commit;
pub mod attrcache;
pub mod branch;
pub mod diff_driver;
pub mod filter;
pub mod hashmap_str;
pub mod indexer;
pub mod merge;
pub mod merge_driver;
pub mod pcre2;
pub mod path;
pub mod pending;
pub mod refdb_reftable;
pub mod remote;
pub mod revparse;
pub mod streams;
pub mod submodule;
pub mod sysdir;
pub mod transports;
pub mod worktree;
pub mod zlib;
