# Lessons

Rules that hold for this campaign's target alone, as the `evolution` skill's Lessons
section describes. Each run reads them before its first step.

## libgit2's allocator primitives are reached through the odb backend data routines

- **Rule.** `git__malloc`, `git__free`, `git__calloc`, `git__mallocarray`, `git__strdup`,
  `git__strndup`, `git__substrdup` and the global `git__allocator` are hidden in
  `libgit2.so`. Rust allocates and releases in libgit2's allocator family through the
  strategies of `src/libgit2/src/util/alloc.rs` (`Free` / `AllocBox`, `ArrayFree` /
  `AllocVec`, `StrFree` / `AllocStr`), which reach `git__malloc` and `git__free` through
  `git_odb_backend_data_alloc` and `git_odb_backend_data_free`, whose bodies are those
  alone, and compose the rest. New owners of libgit2-allocated memory use those policies,
  not new ones over the odb routines.
- **Evidence.** wave 12 batch 1 (`src/util/alloc.c`), and the earlier `OdbData*` policies
  of `src/libgit2/src/libgit2/odb.rs`, which predate them.
- **Scope.** Every module's owners of strings, singletons and arrays libgit2 frees with
  `git__free`, until `src/libgit2/odb.c` is integrated: its port must not reach the
  allocator through `alloc.rs`'s strategies while they reach it through its exports.
