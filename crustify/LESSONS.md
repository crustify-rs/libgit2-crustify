# Lessons

Rules that hold for this campaign's target alone, as the `evolution` skill's Lessons
section describes. Each run reads them before its first step.

## libgit2's allocator family is reached through `alloc.rs`'s strategies

- **Rule.** Rust allocates, copies and releases memory in libgit2's allocator family
  (`git__malloc`, `git__calloc`, `git__mallocarray`, `git__strdup`, `git__strndup`,
  `git__substrdup`, `git__free`) through the strategies of `src/libgit2/src/util/alloc.rs`:
  `Free` / `AllocBox`, `ArrayFree` / `AllocVec` and `StrFree` / `AllocStr`, which call
  those primitives through their raw bindings, the two `static inline` ones through the
  wrappers in `libgit2-sys/src/static_fns.c`. New owners of memory libgit2 frees with
  `git__free` use those policies, not new ones over `git_odb_backend_data_alloc` /
  `git_odb_backend_data_free`, whose bodies are `git__malloc` / `git__free` but which are
  odb.c's exports and loop back through the strategies once odb.c is ported.
- **Evidence.** wave 12 batch 1 (`src/util/alloc.c`) reached the family through odb.c's
  exports while the primitives were hidden in `libgit2.so`; the static campaign link
  (`d21c78c70`) made them link, and wave 12 batch 2 rebound the strategies over them. The
  `OdbData*` policies of `src/libgit2/src/libgit2/odb.rs` (`OdbDataBox`, `OdbDataVec`,
  `OdbDataStr`, `OdbDataStrVec`) and the raw `git_odb_backend_data_alloc` calls of
  `oidarray.rs` predate them.
- **Scope.** Every module's owners of strings, singletons and arrays libgit2 frees with
  `git__free`. The existing `OdbData*` uses are odb.c's batches' to revisit: where they
  hold memory C frees with `git__free` rather than with `git_odb_backend_data_free`, they
  take these strategies.

## libgit2's allocator storage is 8-aligned, not `malloc`-aligned

- **Rule.** A policy over libgit2's allocator family (`git__malloc` and its siblings, and the
  exports whose bodies are theirs, such as `git_odb_backend_data_alloc`) vouches for the
  wider of `usize`'s and `u64`'s alignment, never `max_align_t`'s. libgit2 configured with
  `DEBUG_STRICT_ALLOC`, as its own CI is, runs on `debugalloc`, which returns `malloc`'s block
  offset by a `size_t`. A type aligned to 16 is refused at compile time, not allocated.
- **Evidence.** wave 12 batch 3's `ub_safe/strict-alloc-misaligns-wide-runs`: with
  `MAX_ALIGN = align_of::<max_align_t>()`, `AllocVec::<u128>::from_slice` formed a misaligned
  slice from safe code on a private `-DDEBUG_STRICT_ALLOC=ON` build. `odb.rs`'s `DATA_ALIGN`
  already followed this rule.
- **Scope.** Every lifecycle policy, owner or alignment claim over storage libgit2's
  allocator makes, whether it comes from Rust or from C.

## Test allocators start from libgit2's standard allocator

- **Rule.** A test that installs an allocator which refuses, counts or delays some requests
  builds it as libgit2's own out-of-memory tests do (`tests/util/str/oom.c`): it copies
  `stdalloc::allocator()` (`src/libgit2/src/util/allocators/stdalloc.rs`), replaces the
  members it changes, and forwards the rest of each request to the standard member it
  replaced, read once from that value. It does not declare the C library's `malloc`,
  `realloc` and `free` in an `extern` block of its own, nor reimplement an `Alloc` over them
  only to stand in for the standard family.
- **Evidence.** wave 181 batch 0 bound `git_stdalloc_init_allocator` once the static
  campaign link (`d21c78c70`) made it link; it had been hidden from `libgit2.so`. Before it,
  about thirty test files (the `Refusing` and `Countdown` allocators of `buf.rs`,
  `config.rs`, `index.rs`, `refs.rs`, `settings.rs` and others, and their `equiv` and
  `ub_unsafe` counterparts) each declared libc's `malloc` family to build one. The module
  pass (wave 181 batch 1) built the out-of-memory equivalence test this way, as batch 0
  did, and restores the default with `set_allocator(None)` as `test_str_oom__cleanup` does.
- **Scope.** Every test allocator written from wave 181 on. The existing ones stay until
  their modules' batches revisit them; they are not wrong, since the standard family is the
  C library's heap, but they repeat a binding by hand.
