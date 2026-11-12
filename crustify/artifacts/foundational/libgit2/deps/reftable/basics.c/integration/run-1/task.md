# deps/reftable/basics.c integration, run-1

- Repository: the module's worktree, `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/deps/reftable/basics.c/integration`, checked out on
  `crustify/foundational/libgit2/deps/reftable/basics.c` at `81dede2b1` (wave 169 promoted: waves 169 to 175, batches 0 to 5, its module pass included).
- Run dir: `crustify/artifacts/foundational/libgit2/deps/reftable/basics.c/integration/run-1/`.

## Departures from your role, named so you keep them

This is not a scheduled batch. You carry out the integration merge of `deps/reftable/basics.c` into the working
branch, as `versioning`'s Integration section describes: merge, resolve, gate, commit. You
have no items and no worklist, and your translation steps do not apply. You land by
committing the merge on the module's branch in this worktree; the assistant fast-forwards
the working branch to it. Use no landing lock, and push nothing.

## What to do

1. Merge the working branch's tip (`crustify/devel`, at `1043d550f` or later):
   `git merge crustify/devel`.
   - Additive conflicts keep both sides: dashboard rows, `LESSONS.md`, proposals' evidence,
     `allowlist.txt`, `wrapper.h`, module lists such as `tests/*/main.rs`, `src.rs` and
     `util.rs`, and `Cargo.toml`'s `[[test]]` entries (one each).
   - The generated bindings (`libgit2-sys/src/bindings.rs`, `static_fns.c`) are regenerated
     from the merged input in campaign mode with `CRUSTIFY_BINDINGS_UPDATE=1`, whether or
     not they conflict.
   - In the dashboard, keep every row both sides added. Recompute each additive `Σ` cell as
     the sum of the merged rows (ours + theirs − base). Usage `Σ` cells are summed over the
     rows that have usage. Coverage cells and the Coverage stats percentages take the later
     landing; say they were not measured on the merged tree. One `Batches: API stability`
     table per route, one `Module passes` section and one legend line, however many times
     each side added them.
   - Any conflict that is not additive is resolved without discarding either side; describe
     it.
   - No conflict marker may stay (`git diff --check`, `git grep -n '^<<<<<<< '`).
2. The merged tree passes `testing-gates`' Regressions gates:
   - check, `cargo +nightly-2026-10-08 fmt --all --check`, and clippy over the workspace,
     warnings denied;
   - the whole workspace's tests on `plain` and `asan-ubsan`;
   - Miri and TSan over the tests of the modules both sides touched, where their code
     meets.

   Both sides passed their module and wave gates, so judge further gates by what the
   combination could break, and say what you ran. Known failures, not yours:
   - 5 trybuild `$FFIBOX` mismatches (13 under the nightly sanitizers);
   - the nightly `CommitBuilder` compile-fail doctest;
   - `first_discoveries_race_to_pin_the_runtime` outside a git repository;
   - under BSan, four indexer tests that never finish;
   - packaging against an installed libgit2.
3. Commit the merge as `crustify: integrate deps/reftable/basics.c (foundational)`, with the settings'
   identity: author Marius Momeu, the single `Co-Authored-By: crustify
   <crustify@berkeley.edu>` trailer. The message describes every non-additive resolution.
4. Run `ffimap api` from the module branch's tip before the merge, and from the working
   branch's tip, to the merged tree (ffibox's private rustdoc JSON in `crustify/tmp/`).
   Report any item that is not `kept`, `extended` or `new`.

## Host

Other agents run module passes and a binder beside you. Cap builds at 16 jobs, keep all
scratch in `crustify/tmp/`, and end only processes of this worktree.

## Report

Report:
- the merge commit, and each conflict's resolution;
- the gates and their results;
- the API measure.
