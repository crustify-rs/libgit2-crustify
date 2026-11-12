# wave-176/batch-2/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/allocators/failalloc.c/wave-176/batch-2`, on `crustify/batches/wave-176/batch-2`.
- Worklist: `crustify/artifacts/foundational/libgit2/src/util/allocators/failalloc.c/wave-176/batch-2/run-1/task.json` in it: the commits of batches 0, 1. No `objectives` are given, so every objective
  that applies runs, `fix` included (the settings' `review-objectives` answer is `all`).
- Batch: `wave-176/batch-2` of the foundational module `libgit2/src/util/allocators/failalloc.c`. Foundational modules
  stay `wrap` for good, so review to full depth (`reviewing`, Depth).
- Wave gates: no.
- Land on the unchecked-out wave branch `crustify/waves/wave-176`. Name it
  `refs/heads/crustify/waves/wave-176` in every git command.

## Instructions

- Use `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/allocators/failalloc.c/wave-176/batch-2/crustify/tmp/` for every scratch file, log and scratch build, not the shared
  scratchpad: other agents run beside you and overwrite shared paths (proposal
  `scratch-shared-between-agents`). Cap builds at 16 jobs (`CARGO_BUILD_JOBS=16`); the host
  has little free memory. End only processes of your own worktree.
- Campaign mode links each variant's C statically since `d21c78c70` (Crustify `20500ce`,
  `setup` Builds and `bindgen` Build modes): every C function with external linkage links
  through the raw bindings. A Rust reproduction of a C body, or a route around a primitive
  through another export, kept only because a symbol was hidden is a finding.
- Known failures that are not this module's: 5 trybuild mismatches in `ub_safe::compile_fail`
  that print `/opt/venv/share/ffibox/...` for `$FFIBOX`; under the nightly, the `CommitBuilder`
  compile-fail doctest (E0624); `first_discoveries_race_to_pin_the_runtime` in a tree that is
  not a git repository; and, under `cargo bsan`, a bindings check that differs by doc comments
  alone. Packaging `libgit2-sys` against an installed libgit2 now fails once the crate calls
  an internal function: the spec's bundled-mode packaging lives on the unmerged
  `crustify/migrations/bundled-ffi`, and proposal `internal-headers-in-the-bindings` holds
  the evidence. Report it there, not as a finding.
- Batch 0 (`9a64abfc7`) reproduced the three routines in Rust; batch 1 (`7045f4428`) rebound
  them to the C and added `Alloc::C_ALLOCATOR` in `src/libgit2/src/util/alloc.rs`, alloc.c's
  file, so that `Allocator::new::<FailAlloc>()` holds C's own routines. Review that addition
  as part of this workset. Wave 12's `batch-2` rewrote the rest of that file on its own
  branch: the two meet at integration, not here.
- A module pass of `failalloc.c` follows you in this wave and runs the wave's and module's
  gates.
