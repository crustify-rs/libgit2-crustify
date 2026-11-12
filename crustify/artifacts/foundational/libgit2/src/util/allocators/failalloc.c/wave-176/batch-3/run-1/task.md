# wave-176/batch-3/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/allocators/failalloc.c/wave-176/batch-3`, on `crustify/batches/wave-176/batch-3`.
- Worklist: `crustify/artifacts/foundational/libgit2/src/util/allocators/failalloc.c/wave-176/batch-3/run-1/task.json` in it: the module's paths (a module pass). No `objectives` are given, so every objective
  that applies runs, `fix` included (the settings' `review-objectives` answer is `all`).
- Batch: `wave-176/batch-3` of the foundational module `libgit2/src/util/allocators/failalloc.c`. Foundational modules
  stay `wrap` for good, so review to full depth (`reviewing`, Depth).
- Wave gates: yes, `wave_gates` is true. Module end gates: yes, this is the module pass of its last wave.
- Land on the unchecked-out wave branch `crustify/waves/wave-176`. Name it
  `refs/heads/crustify/waves/wave-176` in every git command.

## Instructions

- Use `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/allocators/failalloc.c/wave-176/batch-3/crustify/tmp/` for every scratch file, log and scratch build, not the shared
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
- This is `failalloc.c`'s module pass: wave 176 is the module's only wave. Batch 0
  (`9a64abfc7`) bound it, batch 1 (`7045f4428`) rebound it to the C, and batch 2
  (`ab7d4915c`) reviewed both and fixed two documentation findings
  (`alloc-contract-forbids-its-own-c-families`, `failalloc-free-note-before-init-only`).
  Review the module whole, across those batches, including `Alloc::C_ALLOCATOR` in
  `src/libgit2/src/util/alloc.rs`, which batch 1 added for this module.
- Your gates are the wave's (Miri over the whole workspace, one run per test binary,
  skipping compile-fail) and the module's end (every test under every available sanitizer,
  and BSan over the whole workspace, one invocation per test binary, each under a
  `timeout`), on the wave's tip before any change, and again on your landed tree under
  `fix`. Run them as `testing-gates` describes, at 16 jobs in all.
- The previous review batch found that this host refuses a subagent's writes of
  `report.md` files under `crustify/reviews/`. Try to file your reports as `reviewing`
  describes. If a write is refused, do not work around the refusal: carry each finding in
  your fix commit's message and in your final report, and say so.
