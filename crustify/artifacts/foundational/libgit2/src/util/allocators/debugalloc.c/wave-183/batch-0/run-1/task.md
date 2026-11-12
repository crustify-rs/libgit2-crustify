# wave-183/batch-0/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/allocators/debugalloc.c/wave-183/batch-0`, on `crustify/batches/wave-183/batch-0`.
- Worklist: `crustify/artifacts/foundational/libgit2/src/util/allocators/debugalloc.c/wave-183/batch-0/run-1/task.json` in it.
- Batch: `wave-183/batch-0` of the foundational module `libgit2/src/util/allocators/debugalloc.c` (`modules.json`'s
  `foundational: true` record of that name), frozen at base `69ff068e8`.
- Land on the unchecked-out wave branch `crustify/waves/wave-183`. Name it
  `refs/heads/crustify/waves/wave-183` in every git command.

## Instructions

- Four batches run at once on this host (`wave-181/batch-0`, `wave-182/batch-0`,
  `wave-183/batch-0`, `wave-184/batch-0`), beside two module passes (waves 169 and 176).
  Cap your builds at 16 jobs (`CARGO_BUILD_JOBS=16`, and the same for C builds and tests).
  Keep every scratch file, log and scratch build in your worktree's `crustify/tmp/` (the
  `layout` skill's Scratch section), never the session scratchpad. End only processes of
  your own worktree, as the `concurrency` skill describes.
- Campaign mode links each variant's C statically (`d21c78c70`), so every C function with
  external linkage links through the raw bindings. Bind it there; do not reproduce a C body
  in Rust, or route around a primitive through another export, because a symbol was hidden.
- New since the last wave (Crustify `87e3816`): every batch measures its items'
  consumer-facing API with `ffimap api`, as the `ffimap` skill's Accounting › API stability
  section and `testing-gates` describe, and fills the dashboard's `Batches: API stability`
  row. Generate ffibox's rustdoc JSON with private items, from `/opt/venv/share/ffibox`,
  into your `crustify/tmp/`, with the same nightly as the crate's. Your items are new, so
  expect `new`; anything else the measure reports about existing items needs its reason.
- `src/util/alloc.c`'s first run is integrated (`3dbb73d6d`): `src/libgit2/src/util/alloc.rs`
  holds `Allocator` (the `git_allocator` value) and the strategies over libgit2's allocator
  (`AllocBox`, `AllocVec`, `AllocStr` and their policies). Use them; read their docs and
  `crustify/LESSONS.md` (libgit2's allocator storage is only 8-aligned under
  `DEBUG_STRICT_ALLOC`). `src/util/allocators/failalloc.c` is not integrated yet: its wave
  176 adds `Alloc::C_ALLOCATOR` to `alloc.rs`, so that `Allocator::new::<A>()` can hold C's
  own routines; do not depend on it, and expect its integration to meet yours.
- The workspace adopted crates for the out-of-tree libraries (`dependencies.json`'s
  `decision`s): reach libc through `std`, `nix` or `glibc-bindings` as each entity's route
  says, and report each entity's route as the `dependencies` skill's Confirmation section
  asks.
- Known failures that are not yours: 5 trybuild mismatches in `ub_safe::compile_fail` that
  print `/opt/venv/share/ffibox/...` for `$FFIBOX`; under the nightly, the `CommitBuilder`
  compile-fail doctest (E0624); `first_discoveries_race_to_pin_the_runtime` in a tree that is
  not a git repository; under `cargo bsan`, a bindings check that differs by doc comments
  alone. Packaging `libgit2-sys` against an installed libgit2 fails once the crate calls an
  internal function; the spec's bundled-mode packaging is on the unmerged
  `crustify/migrations/bundled-ffi` (proposal `internal-headers-in-the-bindings`). Add
  evidence there if you meet it; it is not yours to fix.
- The C coverage figure is read from the coverage variant's static archive alone
  (86,833 lines); `report.py`'s scope filter does not apply it (proposals
  `coverage-c-scope-matches-no-path`, `coverage-c-scope-unfiltered`). Say which method you
  used.
- `git_debugalloc_init_allocator` fills a `git_allocator` with the debug allocator, which
  records each block's size in a `size_t` before it, so its storage is only 8-aligned
  (`crustify/LESSONS.md`, and wave 12's review finding `strict-alloc-misaligns-wide-runs`).
