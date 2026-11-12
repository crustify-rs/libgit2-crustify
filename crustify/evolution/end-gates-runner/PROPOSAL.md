# A runner for a wave's and a module's end gates

Status: `landed`, with `module-end-gate-runner`, as crustify `039b394` on `devel`: `skills/verification/testing/gates/end-gates`, with its tests. The BSan bindings' comment-only difference is left to a stage's `--stage-env` (such as `bsan:CRUSTIFY_BINDINGS_UPDATE=1`, harmless in the snapshot), not fixed at its source.

## Signal

One procedure repeated by hand: every last review batch and module pass writes its own
script for the gates `testing-gates`' Regressions section gives a wave's and a module's end,
and works around the same obstacles each time.

## Evidence

- `foundational/libgit2/src/util/allocators/stdalloc.c/wave-181/batch-1/run-1` (this run)
  wrote `gates.sh`, kept beside this proposal: a `git archive` snapshot of the tree under
  `crustify/tmp/` so that work continues while the gates run, the workspace on `plain`,
  `asan-ubsan` and `tsan` one after another, Miri built once and then run per test binary
  side by side (concurrent Miri builds sharing `target/miri` race, as the dashboard's
  `diff_parse.c` wave 229 note records), and BSan per test binary under a `timeout`, with
  the coordinator's list of tests that never finish under BSan skipped by name.
- `deps/reftable/basics.c/wave-169/batch-5` built the same `snap-pre` / `snap-post`
  snapshots by hand the same day, and the campaign's memory records the snapshot recipe
  (`git init` in it, `builds` symlinked) as a pitfall found in an earlier wave.
- Under `cargo bsan`, the raw bindings' campaign-mode check fails, because BSan's clang
  makes bindgen emit doc comments for the macro constants that the committed bindings lack.
  This run, `wave-181/batch-0` and the task messages of the wave's batches each work around
  it by copying the generated `out/bindings.rs` into a snapshot after checking that only
  comments differ; this run's first BSan pass failed to build until it did, and its
  coverage run of the same snapshot then failed the plain check until the committed file
  was restored.

## Destination

The spec and a script beside it: `testing-gates`, as `authoring` places a procedure that
must run exactly so, a script beside the skill documented by its `--help`, like the TODO on
`versioning`'s landing lock.

## Change

A script, `skills/verification/testing/gates/end-gates`, taking the tree, a log dir and
the stages to run (`san`, `miri`, `bsan`), with the job count from the environment, as
`concurrency` requires:

1. snapshots the tree's `HEAD` under `crustify/tmp/<label>`, with `c/` and `builds`
   symlinked and `git init` run in it;
2. runs `cargo-variant.sh <variant> test --workspace --no-fail-fast` for `plain` and every
   sanitizer variant `build.json` records but `bsan`;
3. builds Miri's test binaries once, then runs one `cargo miri test` per test binary,
   side by side, each skipping `compile_fail`;
4. for BSan, in a snapshot of its own, adopts the bindings BSan generates when they differ
   from the committed ones by comments alone, refuses otherwise, and runs one invocation per
   test binary under a `timeout`, skipping the tests named in a skip file; and
5. writes a summary of each step's exit status and test-result lines.

The comment-only bindings difference could instead be removed at its source, by making
`libgit2-sys/build.rs`'s check compare the bindings without comments, which `bindgen`'s
owner would decide.

## Validation

The draft ran this run's gates on the wave's tip and on the landed tree: Miri, `plain`,
`asan-ubsan`, `tsan` and BSan, with the results the dashboard's note for the pass records.
