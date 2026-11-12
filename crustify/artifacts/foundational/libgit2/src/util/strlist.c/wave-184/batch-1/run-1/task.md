# wave-184/batch-1/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/strlist.c/wave-184/batch-1`, on `crustify/batches/wave-184/batch-1`.
- Worklist: `crustify/artifacts/foundational/libgit2/src/util/strlist.c/wave-184/batch-1/run-1/task.json` in it: the module's paths (a module pass). No `objectives` are given, so every objective
  that applies runs, `fix` included (the settings' `review-objectives` answer is `all`).
- Batch: `wave-184/batch-1` of the foundational module `libgit2/src/util/strlist.c`. Foundational modules
  stay `wrap` for good, so review to full depth (`reviewing`, Depth).
- Wave gates: yes, `wave_gates` is true. Module end gates: yes, this is the module pass of its last wave.
- Land on the unchecked-out wave branch `crustify/waves/wave-184`. Name it
  `refs/heads/crustify/waves/wave-184` in every git command.

## Instructions

- Use `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/strlist.c/wave-184/batch-1/crustify/tmp/` for every scratch file, log and scratch build, not the shared
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
- This is `strlist.c`'s module pass, and its only review (Crustify `8e23c1c`). Wave 184 holds
  waves 184 and 185, packed into batch 0 (`aba03725e`, 6 functions). It bound the owners
  `StrList` and `NullStrList` over `CVec<ManuallyDrop<AllocStr>, P>`, their policies
  `StrListFree` and `NullStrListFree`, and the searches `contains_prefix` and `contains_key`.
  It documented and pinned two C defects rather than refusing them: a refused copy leaks what
  it copied, and `git_strlist_contains_key` compares without the delimiter (`"FOO=1"` matches
  `["FOOBAR=2"]`). Judge each against `binding`'s rule for C defects: is a Rust-side refusal
  or a corrected answer the binding's to give?
- Its safety audit moved `pub raw ptr` +4 and `pub unsafe fn` +4: the owners' hand-off seams
  (`as_ptr`, `into_raw_parts`, `into_raw`, `from_raw_parts`, `from_raw`) and the `CLenDrop`
  methods. Check each against the `ffimap` skill's Binding › Safe boundary section: a site a
  safe variant could replace is a finding.
- The batch filed `elementwise-releasers`: ffibox's ownership row for "freed with its elements
  by one routine" would let `CVec<*mut c_char, P>` hand out raw pointers the routine frees.
  `strlist.rs` avoids that shape through `ManuallyDrop<AllocStr>` elements; check that it is
  sound for every element operation the owners expose (mutable deref, `try_clone`,
  `into_raw_parts` and `from_raw_parts`).
- Its unit tests are inline, while other waves added a `tests/unit_pub` suite; the dashboard's
  `Batches: API stability` tables meet other waves' at integration.
- Your gates are the wave's (Miri over the whole workspace, one run per test binary,
  skipping compile-fail) and the module's end (every test under every available sanitizer,
  and BSan over the whole workspace, one invocation per test binary, each under a
  `timeout`), on the wave's tip before any change, and again on your landed tree under
  `fix`, at 16 jobs in all. Under BSan, skip by exact name the four indexer tests that never
  finish (`lib`: `indexer::unit_tests::a_break_cancels_the_indexing_with_git_euser`,
  `indexer::unit_tests::a_panic_cancels_then_resumes_after_the_walk`,
  `indexer::unit_tests::libgit2_calls_the_closure_with_its_progress`; `equiv`:
  `src::libgit2::indexer::a_progress_callback_sees_and_cancels_as_a_c_callback`), and report
  them skipped. Other module passes run beside you: keep to your share of the host.
