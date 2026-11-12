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

## The tree's inline `mod unit_tests` modules are no precedent for test placement

- **Rule.** A new unit test of a module's public interface goes in `tests/unit_pub`, at the
  tested TU's path (`tests/unit_pub/src/util/...`), and one of an internal routine in an
  inline `#[cfg(test)] mod unit_priv` beside it, as the conventions' Testing section places
  them, whatever the module's file or its neighbours hold. The 34 inline `mod unit_tests`
  modules under `src/` (`alloc.rs` and `errors.rs` among them) predate that placement and
  mix both kinds; a batch that adds tests to one of those files puts its new tests where the
  conventions place them, and leaves the old ones to the batches that port their modules.
  Until `tests/unit_pub` reaches the working branch, a wave that needs it creates it with
  the same `main.rs`, `src.rs`, directory files and `[[test]] unit_pub` entry as waves 169
  and 176, so that the branches merge with only the one-line `mod` additions conflicting.
- **Evidence.** wave 182 batch 0 (`cff595a09`) put `win32_leakcheck.rs`'s three
  public-interface tests in an inline `mod unit_tests`, following `alloc.rs` and
  `errors.rs`; its module pass (`wave-182/batch-1`) moved them to `tests/unit_pub` and
  renamed the module `unit_priv` (a `conventions` finding).
- **Scope.** Every batch that writes unit tests, until the legacy `unit_tests` modules are
  gone.
