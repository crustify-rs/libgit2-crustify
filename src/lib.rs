//! Safe Rust wrappers over libgit2's public API.
//!
//! The tree mirrors `crustify/artifacts/modules.json`: one module per link unit, and
//! within it one module per translation unit, at the TU's path below the C root
//! (`libgit2_crustify::libgit2::src::libgit2::repository` for `src/libgit2/repository.c`),
//! holding the items of the TU and of the headers bundled with it. The raw bindings are the
//! public [`sys`] module, one module per link unit, through which the C that is not wrapped
//! yet is reached.

pub mod sys;

#[cfg(feature = "libgit2")]
pub mod libgit2;

#[cfg(feature = "git2")]
pub mod git2;
