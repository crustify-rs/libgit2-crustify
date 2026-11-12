//! `src/util/allocators/win32_leakcheck.c`: the allocator that records allocations for the
//! Windows CRT's leak checker; off Windows, its initialiser only reports that it is not
//! built. A foundational module: its routines are bound as strategies and stay C.
