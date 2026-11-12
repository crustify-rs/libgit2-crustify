# wave-169/batch-5/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/deps/reftable/basics.c/wave-169/batch-5`, on `crustify/batches/wave-169/batch-5`.
- Worklist: `crustify/artifacts/foundational/libgit2/deps/reftable/basics.c/wave-169/batch-5/run-1/task.json` in it: the module's paths (a module pass). No `objectives` are given, so every objective
  that applies runs, `fix` included (the settings' `review-objectives` answer is `all`).
- Batch: `wave-169/batch-5` of the foundational module `libgit2/deps/reftable/basics.c`. Foundational modules
  stay `wrap` for good, so review to full depth (`reviewing`, Depth).
- Wave gates: yes, `wave_gates` is true. Module end gates: yes, this is the module pass of its last wave.
- Land on the unchecked-out wave branch `crustify/waves/wave-169`. Name it
  `refs/heads/crustify/waves/wave-169` in every git command.

## Instructions

- Use `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/deps/reftable/basics.c/wave-169/batch-5/crustify/tmp/` for every scratch file, log and scratch build, not the shared
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
- This is `basics.c`'s module pass: wave 169 holds the module's last wave (waves 170 to 175
  are packed with it). The wave landed batch 0 (`85ad07673`, types), batch 2 (`2032628fd`,
  20 functions and `Buf`'s lifecycle), batch 1 (`bbaafcd09`, 23 items), the types review
  (batch 3: `ec289165f`, `418ca278b`; `Buf` made `Send`/`Sync`, `BufRef: Hash`) and the
  symbols review (batch 4: `d04070734`, `9aa6520a5`; five findings, four patched; the 15
  `Replaces` ports judged allowed by their `binding-symbols` routes; `set_alloc`'s safety
  contract now orders other threads by synchronisation; `HashId::size` now `usize`). The
  reviews' reports are in those commits' messages, since the host refused their
  `report.md` files. Review the module whole, across those batches.
- Open plan correction from batch 4: `basics::Error` is an unanchored stand-in for
  `enm::reftable_error`, which wave 216 holds (proposal `reftable-error-wrapped-ahead-of-basics`).
  Leave it to the assistant.
- Your gates are the wave's (Miri over the whole workspace, one run per test binary,
  skipping compile-fail) and the module's end (every test under every available sanitizer,
  and BSan over the whole workspace, one invocation per test binary, each under a
  `timeout`), on the wave's tip before any change, and again on your landed tree under
  `fix`. Run them as `testing-gates` describes, at 16 jobs in all. `failalloc.c`'s module pass
  runs the same gates beside you on its own wave: keep to your share of the host.
- The host refuses a subagent's writes of `report.md` files under `crustify/reviews/`. Try
  the normal file tool; if a write is refused, do not route around it: carry each finding in
  your fix commit's message and in your final report. Reproducer crates write normally.
