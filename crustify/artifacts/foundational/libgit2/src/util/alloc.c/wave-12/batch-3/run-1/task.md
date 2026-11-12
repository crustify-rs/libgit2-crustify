# wave-12/batch-3/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/alloc.c/wave-12/batch-3`, on `crustify/batches/wave-12/batch-3`.
- Worklist: `crustify/artifacts/foundational/libgit2/src/util/alloc.c/wave-12/batch-3/run-1/task.json` in it: the commits of batches 1, 2. No `objectives` are given, so every objective
  that applies runs, `fix` included (the settings' `review-objectives` answer is `all`).
- Batch: `wave-12/batch-3` of the foundational module `libgit2/src/util/alloc.c`. Foundational modules
  stay `wrap` for good, so review to full depth (`reviewing`, Depth).
- Wave gates: yes, `wave_gates` is true.
- Land on the unchecked-out wave branch `crustify/waves/wave-12`. Name it
  `refs/heads/crustify/waves/wave-12` in every git command.

## Instructions

- Use `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/alloc.c/wave-12/batch-3/crustify/tmp/` for every scratch file, log and scratch build, not the shared
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
- Batch 1 (`c0c753430`) composed the family over odb.c's exports because it was hidden;
  batch 2 (`1dc9911d7`) rebound all seven routines to the C. Review the tree as batch 2
  left it, and check nothing of batch 1's composition survives. Batch 0 (`8d14c83d9`,
  `git_allocator`) is the wrap campaign's and not on your worklist, but its docs in
  `alloc.rs` still say `git_stdalloc_init_allocator` is "compiled hidden", which is stale.
- This is not `alloc.c`'s last wave (waves 177 to 180 follow in a later run), so no module
  pass follows: you run the wave's gates (Miri over the whole workspace).
