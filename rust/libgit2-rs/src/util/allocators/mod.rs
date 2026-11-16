//! Ports of `src/util/allocators/`: the allocators libgit2 can install as
//! `git__allocator`. (`win32_leakcheck.c` is Windows-only and stays C.)

pub mod debugalloc;
pub mod failalloc;
pub mod stdalloc;
