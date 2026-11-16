//! Ports of `src/util`: the foundation every other module builds on.
//!
//! See `README.md` in this directory for a guide to the API: errors,
//! allocation, `git_str`, `git_vector`, threads, layout tests, exports and
//! the panic barrier.

pub mod alloc;
pub mod allocators;
pub mod array;
pub mod ctype_compat;
pub mod errors;
pub mod export;
pub mod integer;
pub mod layout;
pub mod runtime;
pub mod str;
pub mod thread;
pub mod tsort;
#[allow(clippy::module_inception)]
pub mod util;
pub mod vector;

#[doc(hidden)]
pub mod test_support;
