# wave-169/batch-4/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/deps/reftable/basics.c/wave-169/batch-4`, on `crustify/batches/wave-169/batch-4`.
- Worklist: `crustify/artifacts/foundational/libgit2/deps/reftable/basics.c/wave-169/batch-4/run-1/task.json` in it: the commits of batches 1, 2. No `objectives` are given, so every objective
  that applies runs, `fix` included (the settings' `review-objectives` answer is `all`).
- Batch: `wave-169/batch-4` of the foundational module `libgit2/deps/reftable/basics.c`. Foundational modules
  stay `wrap` for good, so review to full depth (`reviewing`, Depth).
- Wave gates: no.
- Land on the unchecked-out wave branch `crustify/waves/wave-169`. Name it
  `refs/heads/crustify/waves/wave-169` in every git command.

## Instructions

- Use `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/deps/reftable/basics.c/wave-169/batch-4/crustify/tmp/` for every scratch file, log and scratch build, not the shared
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
- Wave 169's base merge (`4730ad9ba`) was committed with conflict markers in
  `libgit2-sys/build.rs`, `wrapper.h` and `bindings.rs`; batch 2 (`2032628fd`) resolved them
  to `d21c78c70`'s side. That is the assistant's error, already known: check only that the
  resolution kept the fix's side.
- Batches 1 and 2 measured C coverage differently: batch 2's denominator (190,798 lines)
  counted every object, batch 1's (86,833) the static archive alone (proposal
  `coverage-c-scope-matches-no-path`). Use batch 1's method and say which you used.
- The previous review batch found that this host refuses a subagent's writes of
  `report.md` files under `crustify/reviews/`. Try to file your reports as `reviewing`
  describes. If a write is refused, do not work around the refusal: carry each finding in
  your fix commit's message and in your final report, and say so.
- A module pass of `basics.c` follows the two review batches of this wave and runs the
  wave's and module's gates.
- Your route is `symbol`: batch 1 (`bbaafcd09`, 23 items) and batch 2 (`2032628fd`, 20 items).
  The types' review batch runs beside you on batch 0's `Buf` and `HashId`.
- `basics.c` is a foundational module, whose items stay `wrap` (`decomposition`, Foundational
  modules). The batches nonetheless ported 15 items with `Replaces`: batch 2's
  `reftable_alloc_grow`, and batch 1's `put_be16/24/32/64`, `get_be16/24/32/64`,
  `reftable_alloc_size`, `REFTABLE_SWAP`, `REFTABLE_FREE_AND_NULL`, `REFTABLE_BUF_INIT`,
  `REFTABLE_FORMAT_ID_SHA1` and `_SHA256`. Their stated reason: wrapping an internal
  `static inline` function generates a `static_fns.c` that includes internal headers, which
  fails the installed-library build. That build already fails since `wave-12/batch-2`, and
  the spec's packaging is bundled mode (on the unmerged `crustify/migrations/bundled-ffi`).
  Judge each port against `binding-symbols` (its static inline functions and macros routes)
  and the foundational rule: a port that the spec routes as a binding is a finding, unless
  the route itself allows it (a macro whose expansion the spec lets Rust restate, say).
  Report the outcome per item. Under `fix`, rebind what should be bound.
- Two public `unsafe fn`s, `set_alloc::<A>()` and `set_default_alloc()` over
  `reftable_set_alloc`, are argued unavoidable (unsynchronised global hooks). Try to find a
  safe form, or confirm the argument.
