//! `src/util/allocators/failalloc.c`: the allocator whose every call fails, which
//! libgit2 installs before initialisation and after shutdown. A foundational module: its
//! routines are bound as strategies and stay C.
