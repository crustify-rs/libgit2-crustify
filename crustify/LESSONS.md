# Lessons

Rules that hold for this campaign's target alone, each with the evidence it rests on and
its scope, as the `evolution` skill's Lessons section describes. Every run reads them
before its first step.

## Before `git_libgit2_init`, probe libgit2's allocator only through a release

**Rule.** Before `git_libgit2_init`, `git__allocator` is the failing family
(`src/util/allocators/failalloc.c`), and a test may reach it only through a release:
`git_odb_backend_data_free`, which is `git__free`, sets no error. Any allocation then
fails, and `git__malloc`, `git__realloc` and every error setter report through
`git_error_set_oom` or `set_error`, whose thread-state lookup reads TLS key 0 before
`git_error_global_init` has created libgit2's key: it may return another component's
slot and write `&oom_error` into it. The same holds for an allocator installed with
`set_allocator` before initialisation that fails a request.

**Evidence.** `git_odb_backend_data_alloc`'s finding in the ownership store (ASan
unknown-crash in `git_error_set_oom`, `errors.c:170`); `src/util/errors.c`'s
`threadstate_get`; the equivalence test of `wave-176/batch-0`
(`tests/equiv/src/util/allocators/failalloc.rs`), which compares the C release with the
Rust family's and leaves `malloc` and `realloc` uncompared for this reason. The family's
routines called directly, as `git_failalloc_malloc` through its raw binding, set no error
and are outside the rule: `wave-176/batch-1` compares them so in the test process.

**Scope.** The tests of `src/util/allocators/*.c`, `src/util/alloc.c` and
`src/util/errors.c`, and any test that runs libgit2 before its initialisation.

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

## reftable's error type is `basics::Error`, which `error.c`'s port takes over

- **Rule.** Rust reports reftable's error codes as `deps::reftable::basics::Error`, and no
  other reftable module defines an error type of its own. The batch that ports
  `enum reftable_error` (`deps/reftable/error.c`, wave 217 under planner run 12's numbering)
  makes that type the enum's port, not a second one: it moves `Error` to
  `src/libgit2/deps/reftable/error.rs` with a variant per constant of
  `deps/reftable/reftable-error.h`, each anchored to its constant, keeps `Other(c_int)` for
  codes outside the enum and `from_code` / `code`, gives `Display` `reftable_error_str`'s text,
  and leaves `pub use super::error::Error;` in `basics.rs`, so that `basics.rs`'s callers and
  tests do not change.
- **Evidence.** `deps/reftable/basics.c` returns `REFTABLE_OUT_OF_MEMORY_ERROR` and
  `REFTABLE_FORMAT_ERROR`, yet was scheduled (wave 169, foundational, now integrated) with no
  dependency on `enum reftable_error`, since wavefront records no use of an enum constant
  (`crustify/evolution/wavefront-enum-constant-uses`). Wave-169/batch-2 therefore defined
  `basics::Error`, unanchored, with three of `reftable_error_str`'s strings
  (`wave-169/batch-4`'s `conventions/reftable-error-type-homed-in-basics`;
  `crustify/evolution/reftable-error-wrapped-ahead-of-basics`).
- **Scope.** The batch of wave 217 that ports `enm::reftable_error` and
  `fn::reftable_error_str`, and every reftable module's batch until then.
