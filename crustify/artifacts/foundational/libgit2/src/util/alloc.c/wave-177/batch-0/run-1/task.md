# wave-177/batch-0/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/alloc.c/wave-177/batch-0`, on `crustify/batches/wave-177/batch-0`.
- Worklist: `crustify/artifacts/foundational/libgit2/src/util/alloc.c/wave-177/batch-0/run-1/task.json` in it: waves 177 to 180, packed into one batch (12 items).
- Batch: `wave-177/batch-0` of the foundational module `libgit2/src/util/alloc.c`, its second
  run. Its first run (wave 12) is integrated: `src/libgit2/src/util/alloc.rs` holds
  `Allocator` (`git_allocator`, wave 12 batch 0) and the strategies over libgit2's allocator
  (`AllocBox`, `AllocVec`, `AllocStr`, their policies, `Alloc` with `C_ALLOCATOR`).
  `failalloc.c` is integrated too (`36e39853a`).
- Land on the unchecked-out wave branch `crustify/waves/wave-177`. Name it
  `refs/heads/crustify/waves/wave-177` in every git command.

## Instructions

- Your items complete `alloc.rs`:
  - the three function pointer types of `git_allocator`'s members;
  - xdiff's `xdl_*` allocation macros;
  - the hidden global `git__allocator`;
  - `git__realloc` and `git__reallocarray`;
  - `git_allocator_global_init` and `git_allocator_setup`.

  Several already have Rust stand-ins from wave 12: the `MallocFn`/`ReallocFn`/`FreeFn`
  aliases, `Allocator`'s trampolines, and `settings::set_allocator` over
  `GIT_OPT_SET_ALLOCATOR`, which reaches `git_allocator_setup`. Anchor and complete them
  rather than duplicating them, and keep every existing API as it is. `ffimap api` will
  report any change to an existing item, and each needs its reason.
- `git__allocator` is a hidden global that every libgit2 allocation reads, unsynchronised.
  Bind it within the boundary `binding-symbols`' Globals section gives a global C writes, and
  never expose a safe mutable path to it.
- Your cut dependencies sit in later waves: `src/util/integer.h` (wave 226, overflow helpers)
  and `src/util/errors.c` (waves 273 and 276, `git_error_set_oom` and the assertion macros).
  Reach them through the raw bindings or the existing `errors.rs` wrappers, as
  `scheduling`'s Wrap batches section describes, without binding their items here.
- Five batches of other modules run module passes beside you, and wave 169's.
  - Cap your builds at 16 jobs (`CARGO_BUILD_JOBS=16`, and the same for the C builds and
    tests).
  - Keep every scratch file, log and scratch build in your worktree's `crustify/tmp/`.
  - End only processes of your own worktree.
- Every C function with external linkage links through the raw bindings, since the static
  campaign link (`d21c78c70`). Bind there. Measure your items' API with `ffimap api`, as the
  `ffimap` skill's Accounting › API stability section describes. Put ffibox's private
  rustdoc JSON in your `crustify/tmp/`. The dashboard has the `Batches: API stability`
  table now.
- The workspace adopted crates for the out-of-tree libraries (`dependencies.json`'s
  `decision`s): reach libc through `std`, `nix` or `glibc-bindings` as each entity's route
  says, and report each entity's route as the `dependencies` skill's Confirmation section
  asks.
- Known failures that are not yours:
  - 5 trybuild `$FFIBOX` mismatches (13 under the nightly sanitizers);
  - the nightly `CommitBuilder` compile-fail doctest;
  - `first_discoveries_race_to_pin_the_runtime` outside a git repository;
  - under `cargo bsan`, a bindings check that differs by doc comments alone;
  - under BSan, four indexer tests that never finish:
    `indexer::unit_tests::{a_break_cancels_the_indexing_with_git_euser,
    a_panic_cancels_then_resumes_after_the_walk, libgit2_calls_the_closure_with_its_progress}`
    and `equiv`'s `src::libgit2::indexer::a_progress_callback_sees_and_cancels_as_a_c_callback`.
    Skip them by name.
  - Packaging `libgit2-sys` against an installed libgit2 (proposal
    `internal-headers-in-the-bindings`).
- C coverage: read it from the coverage variant's static archive alone (86,833 lines), not
  through `report.py`'s scope, which records `/target/libgit2-bindings/c` paths. Say which
  method you used.
