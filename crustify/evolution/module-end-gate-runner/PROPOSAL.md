# A script for the wave's and the module's end gates

Status: `landed`, with `end-gates-runner`, as crustify `039b394` on `devel`; skips are checked against each binary's test list before anything runs, as this proposal asks.

## Signal

One procedure repeated by hand. `testing-gates`' Regressions section gives the wave's
gate (Miri, one run per test binary, compile-fail skipped) and the module's end gate
(every test under every sanitizer; BSan one invocation per test binary, each under a
`timeout`), run on a snapshot of the wave's tip and again on the landed tree. Each module
pass writes that runner itself.

## Evidence

- `foundational/libgit2/src/util/allocators/failalloc.c/wave-176/batch-3/run-1` and
  `foundational/libgit2/src/util/alloc.c/wave-177/batch-1/run-1` each wrote a snapshot
  procedure (`git archive`, `c/` and `crustify/builds` symlinks, `git init`, since a
  discovery test needs a repository) and a sequential runner; the second's is
  `crustify/tmp/gates.sh` in its worktree.
- The BSan skips are given by test name, and the names differ by binary (`lib` prefixes
  `libgit2::src::libgit2::`, `equiv` does not): the coordinator had to correct the names
  in the second run's task, and the run checked each with `--list`.
- `cargo bsan` regenerates the bindings with different doc comments, so every BSan run
  sets `CRUSTIFY_BINDINGS_UPDATE` and restores `bindings.rs` and `static_fns.c` after.

## Destination and change

A tool: a script beside `testing-gates`, documented by its `--help`, that snapshots a
commit, runs the stages it is named (plain, each sanitizer variant, Miri and BSan per
test binary) one after another under a job count and a per-stage `timeout`, takes BSan
skips as exact names and checks each against the binary's `--list` before running, and
writes one log per stage and a summary of exit codes and durations. The skill's
Regressions section names it.

## Validation

To be validated by its tests: a stage list over a small workspace, and a skip that matches
no test, which must fail before anything runs.
