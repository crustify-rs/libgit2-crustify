//! `src/util/allocators/stdalloc.c`: the standard allocator, over the C library's `malloc`,
//! `realloc` and `free`, which libgit2 runs on by default. A foundational module: its
//! routines are bound as strategies and stay C.
