# wave-232/batch-0/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/utf8.c/wave-232/batch-0`, on `crustify/batches/wave-232/batch-0`.
- Worklist: `crustify/artifacts/modules/libgit2/src/util/utf8.c/wave-232/batch-0/run-1/task.json` in it.
- Batch: `wave-232/batch-0` of the module `libgit2/src/util/utf8.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-232`. Name it `refs/heads/crustify/waves/wave-232` in every git command.

## Instructions

- Your worklist is `src/util/utf8.c`'s first later stage, wave 232: `git_utf8_char_length`,
  `port/unexport`. The module switched from C at `012d1d34c`; a later-stage wave forks from
  the working branch and is promoted back to it, integrating nothing. Remove the export and
  the raw binding that reaches it, with its allowlist entry, and carry over the C test case
  its record names, `test_utf8__char_length` of `tests/util/utf8.c`, the file's only case,
  removing it from `c/`. `char_count_carries_over_the_c_test`
  (`tests/unit_pub/src/util/utf8.rs`) already carries its ten assertions: check it against
  `testing`'s C tests carried over section rather than writing it again. The other two
  exports, `git_utf8_iterate` and `git_utf8_valid_buf_length`, stay: their stages are waves
  860 and 1531.
- What still names the function in C, checked on this build: `src/util/fs_path.c` (two
  sites), `src/util/fs_path.h` and `tests/libgit2/refs/basic.c` call it only under
  `GIT_WIN32`, which no variant defines, so the plan counts no referrer; `src/util/utf8.h`
  still declares it, for those. Decide what the header's declaration becomes, as the
  conventions' Exports section and `porting-symbols` lead, and say what a `GIT_WIN32` build
  of the fork would now meet.
- Your stage changes `c/`, a later stage's exception: commit the C change on the submodule
  repository's local branch `fork/main` (at `c54c0c180`, api.c's switch; check it out in your
  `c/` worktree), as `unexport git_utf8_char_length`, with the campaign's identity, and
  record the new submodule commit in the superproject. Change no git remote and push
  nothing. The C tests changed, so build the variants' next generation from your `c/`
  under `/target/libgit2-crustify/crustify/builds.gen4/<variant>` with
  `crustify/scripts/build-variants.sh`, link your worktree's `crustify/builds` to it (it
  links to `builds.gen3`, which you do not touch: another batch builds against it), and run
  the remaining C tests through `crustify/scripts/c-tests.sh`. Their tally loses the case
  you carried over: update `crustify/artifacts/c-baseline.json` for it, and say exactly what
  moved. The switch procedure and what three switches learned are in
  `crustify/evolution/module-switch-procedure`.
- A review batch with the wave's gates follows your landing.
- One other batch of the campaign runs beside you, and another campaign shares the host. Cap your
  builds and tests at 32 jobs (`CARGO_BUILD_JOBS=32`, `RUST_TEST_THREADS=32`, and the same for
  C builds). Keep every scratch file, log and scratch build in your worktree's
  `crustify/tmp/`, and delete scratch builds you no longer need before you end. End only
  processes of your own worktree. A wait loop over `pgrep -f` matches its own command line
  and never ends: wait on a PID.
- Your run dir in the worktree holds `task.json` and `task.md`; commit them with your
  landing. The process records of your run (`session_id`, `stream.jsonl`, `stderr.log`,
  `exit_code`) are kept by the assistant in the base checkout, not in your worktree.
- Three modules have switched from C: `src/util/utf8.c`, `src/cli/opt.c` and
  `deps/llhttp/api.c`. `c/` is the fork's without them (`c54c0c180` on its local
  `fork/main`), `build.json` is at version 7, and your worktree's `crustify/builds` links to
  the variants' current generation, `crustify/builds.gen3`, whose link holds the safe crate's
  Rust static library; you never rebuild it. Read the three switches' Notes entries in the
  dashboard and `crustify/LESSONS.md` before your first build.
- Known failures that are not yours: under the nightly variants (asan-ubsan, tsan), 9
  `compile_fail` snapshots blessed on stable mismatch, and the `CommitBuilder` compile-fail
  doctest (E0624). The plain suite passes at your base.
- libc's wrapper is pinned at `24da93f`, a commit of `/target/glibc-crustify` that is not
  pushed: cargo's git cache holds it on this host, so offline resolution works.
- `crustify/scripts/coverage/report.py`'s scope check fails when run from the repository
  root (proposal `coverage-report-relative-rows`): run it from the scratch tree's
  `src/<link-unit>`.
- Packaging in bundled mode: the procedure is `crustify/evolution/package-gate-procedure`,
  with the workspace's Git-source patch for ffibox, as opt.c's switch found.
- BSan is no regression gate of yours (the settings' `sanitizers` answer, from 2026-11-13, a
  departure from `testing-gates`' Regressions section the user asked for): never run
  `cargo bsan` over the workspace, a whole test binary or as a batch, wave or module-end
  gate; that runs once before each ship point. You may run it on small things: the tests
  your own work adds or changes, a reproducer and the regression test it becomes, each test
  named and under a `timeout`.
