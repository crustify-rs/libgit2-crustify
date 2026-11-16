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
pub mod hashmap;
pub mod integer;
pub mod layout;
pub mod pool;
pub mod pqueue;
pub mod runtime;
pub mod str;
pub mod strlist;
pub mod thread;
pub mod tsort;
pub mod utf8;
#[allow(clippy::module_inception)]
pub mod util;
pub mod varint;
pub mod vector;

#[doc(hidden)]
pub mod test_support;
