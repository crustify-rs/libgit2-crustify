# wave-177/batch-1/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/alloc.c/wave-177/batch-1`, on `crustify/batches/wave-177/batch-1`.
- Worklist: `crustify/artifacts/foundational/libgit2/src/util/alloc.c/wave-177/batch-1/run-1/task.json` in it: the module's paths (a module pass). No `objectives` are given, so every objective
  that applies runs, `fix` included (the settings' `review-objectives` answer is `all`).
- Batch: `wave-177/batch-1` of the foundational module `libgit2/src/util/alloc.c`. Foundational modules
  stay `wrap` for good, so review to full depth (`reviewing`, Depth).
- Wave gates: yes, `wave_gates` is true. Module end gates: yes, this is the module pass of its last wave.
- Land on the unchecked-out wave branch `crustify/waves/wave-177`. Name it
  `refs/heads/crustify/waves/wave-177` in every git command.

## Instructions

- Use `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/alloc.c/wave-177/batch-1/crustify/tmp/` for every scratch file, log and scratch build, not the shared
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
- This is `alloc.c`'s module pass. Wave 177 holds the last wave of its `wrap` items (waves 177
  to 180, packed into batch 0, `fce21ef4e`), and its run landed one translation batch, so the
  pass is the run's only review (Crustify `8e23c1c`). Review the module whole, both runs:
  - wave 12: batch 0, `git_allocator`, the wrap campaign's; batches 1 and 2, the strategies;
    batch 3's review;
  - `failalloc.c`'s integration where it touched `alloc.rs` (`Alloc::C_ALLOCATOR`). The other
    allocator modules and `basics.c` integrate after this run forked (`1043d550f`), so they
    are not in your tree;
  - batch 0 of wave 177.
- Wave 177's batch changed the behaviour of existing items, which `ffimap api` cannot see: the
  trampolines `Allocator::new` builds now return NULL when `malloc` or `realloc` panics, where
  they aborted before, and `Alloc`'s contract gained a clause for a panicking `realloc`. Judge
  that against `binding-symbols`' callbacks and panic rules, and against the API stability
  rule (`testing-gates`): a behaviour change to an item after its first landing needs its
  reason recorded.
- It bound `git__allocator` as `pub unsafe fn installed() -> AllocatorRef<'static>` and
  `git_allocator_global_init` as `pub unsafe fn global_init()`. Check both against
  `binding-symbols`' Globals section, and look for a safe form.
- Crustify `57f14ff` (reviewing, Contracts) is new since wave 12: an obligation only C's calls
  can meet is no contract. Apply it to `alloc.rs`'s contracts (`ReallocFn`'s zero-size rule,
  `Alloc`'s).
- Your gates are the wave's (Miri over the whole workspace, one run per test binary,
  skipping compile-fail) and the module's end (every test under every available sanitizer,
  and BSan over the whole workspace, one invocation per test binary, each under a
  `timeout`), on the wave's tip before any change, and again on your landed tree under
  `fix`, at 16 jobs in all. Under BSan, skip by exact name the four indexer tests that never
  finish (`lib`: `indexer::unit_tests::a_break_cancels_the_indexing_with_git_euser`,
  `indexer::unit_tests::a_panic_cancels_then_resumes_after_the_walk`,
  `indexer::unit_tests::libgit2_calls_the_closure_with_its_progress`; `equiv`:
  `src::libgit2::indexer::a_progress_callback_sees_and_cancels_as_a_c_callback`). Other
  agents run beside you: keep to your share of the host, and end every background wait loop
  you start before you report.
