//! Ports of `src/util`: the foundation every other module builds on.
//!
//! See `README.md` in this directory for a guide to the API: errors,
//! allocation, `git_str`, `git_vector`, threads, layout tests, exports and
//! the panic barrier.

pub mod alloc;
pub mod allocators;
pub mod array;
pub mod ctype_compat;
pub mod date;
pub mod errors;
pub mod export;
pub mod filebuf;
pub mod fs_path;
pub mod futils;
pub mod hash;
pub mod hashmap;
pub mod integer;
pub mod layout;
pub mod net;
pub mod pool;
pub mod posix;
pub mod pqueue;
pub mod process;
pub mod rand;
pub mod regexp;
pub mod runtime;
pub mod sortedcache;
pub mod str;
pub mod strlist;
pub mod thread;
pub mod tsort;
pub mod utf8;
#[allow(clippy::module_inception)]
pub mod util;
pub mod varint;
pub mod vector;
pub mod wildmatch;
pub mod zstream;

#[cfg(any(test, feature = "test-internals"))]
pub mod test_support;
