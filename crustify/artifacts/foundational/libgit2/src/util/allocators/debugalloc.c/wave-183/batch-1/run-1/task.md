# wave-183/batch-1/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/allocators/debugalloc.c/wave-183/batch-1`, on `crustify/batches/wave-183/batch-1`.
- Worklist: `crustify/artifacts/foundational/libgit2/src/util/allocators/debugalloc.c/wave-183/batch-1/run-1/task.json` in it: the module's paths (a module pass). No `objectives` are given, so every objective
  that applies runs, `fix` included (the settings' `review-objectives` answer is `all`).
- Batch: `wave-183/batch-1` of the foundational module `libgit2/src/util/allocators/debugalloc.c`. Foundational modules
  stay `wrap` for good, so review to full depth (`reviewing`, Depth).
- Wave gates: yes, `wave_gates` is true. Module end gates: yes, this is the module pass of its last wave.
- Land on the unchecked-out wave branch `crustify/waves/wave-183`. Name it
  `refs/heads/crustify/waves/wave-183` in every git command.

## Instructions

- Use `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/allocators/debugalloc.c/wave-183/batch-1/crustify/tmp/` for every scratch file, log and scratch build, not the shared
  scratchpad: other agents run beside you and overwrite shared paths (proposal
  `scratch-shared-between-agents`). Cap builds at 16 jobs (`CARGO_BUILD_JOBS=16`); the host
  has little free memory. End only processes of your own worktree.
- Measure the consumer-facing API of your workset's items with `ffimap api`, as the `ffimap`
  skill's Accounting › API stability section and `testing-gates` describe (Crustify
  `87e3816`), and fill the dashboard's `Batches: API stability` row: a fix that changes or
  removes an item's API after its first landing needs its reason recorded. ffibox's rustdoc
  JSON, with private items, goes in your `crustify/tmp/`.
- The host refuses a subagent's writes of `report.md` files under `crustify/reviews/`. Try
  the normal file tool; if a write is refused, do not route around it: carry each finding in
  your fix commit's message and in your final report. Reproducer crates write normally.
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
- This is `debugalloc.c`'s module pass, and its only review (Crustify `8e23c1c`). Batch 0
  (`fd3e6b8e1`) bound `git_debugalloc_init_allocator` as `Allocator::debug() -> Allocator`
  and documented where the family departs from `Alloc`'s contract: storage aligned to
  `usize` only, zero-byte requests that fail, and a C defect it confirmed under ASan
  (`total = len + sizeof(size_t)` unchecked, so a request within 8 bytes of `SIZE_MAX` writes
  the length word past a short block), stated as an obligation on calling its members.
  Judge whether that obligation is the right boundary, or whether a safe API can refuse such
  sizes before C (as `alloc.rs`'s `c_alloc_len` now refuses runs past `isize::MAX`).
- The binder noted that `set_allocator`'s contract may still ask for 16-byte alignment,
  which `Allocator::debug()` does not give; wave 12's review lowered `alloc.rs`'s
  `MAX_ALIGN` to `usize`/`u64` alignment. Check the two agree.
- It added a `tests/unit_pub` suite scaffold copied from wave 176's, which the working
  branch does not have yet, and the dashboard's `Batches: API stability` tables, which other
  waves add too: both meet at integration.
- Your gates are the wave's (Miri over the whole workspace, one run per test binary,
  skipping compile-fail) and the module's end (every test under every available sanitizer,
  and BSan over the whole workspace, one invocation per test binary, each under a
  `timeout`), on the wave's tip before any change, and again on your landed tree under
  `fix`, at 16 jobs in all. Other module passes run their gates beside you: keep to your
  share of the host.
