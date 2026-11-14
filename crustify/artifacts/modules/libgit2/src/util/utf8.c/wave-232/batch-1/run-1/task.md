# wave-232/batch-1/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/utf8.c/wave-232/batch-1`, on `crustify/batches/wave-232/batch-1`.
- Worklist: `crustify/artifacts/modules/libgit2/src/util/utf8.c/wave-232/batch-1/run-1/task.json` in it.
- Batch: `wave-232/batch-1` of the module `libgit2/src/util/utf8.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-232`. Name it `refs/heads/crustify/waves/wave-232` in every git command.

## Instructions

- This review batch takes wave 232's one landing, `wave-232/batch-0` (`d48feaced`), the
  campaign's first later stage: `git_utf8_char_length`'s `port/unexport`. It removed the
  export from `src/libgit2/src/util/utf8.rs`'s gateway with its signature check, its
  allowlist entry and its raw binding, deleted `tests/util/utf8.c` from `c/` (its one case
  carried over by `char_count_carries_over_the_c_test`) and the function's declaration from
  `src/util/utf8.h`, on the fork's local `fork/main` (`9859ed5e7`), and lowered every
  variant's record in `crustify/artifacts/c-baseline.json` by that one case. It is the
  wave's last review batch: `wave_gates` is true. The wave is promoted to the working branch
  after you land, integrating nothing.
- Points to judge: the declaration's removal from `utf8.h`, with four call sites left under
  `GIT_WIN32`, which no variant builds (`crustify/evolution/later-stage-c-changes`, a
  `porting-workflow` and `porting-symbols` change on an evolution branch: judge the code
  against the skills as they are on your revision); the carried-over case against
  `testing`'s C tests carried over section; the baseline's change, the `bsan` record being
  derived rather than run; and what still checks `char_count` now that neither an export nor
  a C test reaches it from C.
- Your worktree's `crustify/builds` links to `crustify/builds.gen4`, the generation batch 0
  built from its `c/`. `crustify/scripts/c-tests.sh` runs the remaining C tests.
- No `objectives` are given, so every objective that applies runs, `fix` included. Your gates
  are the wave's, without BSan: the workspace's tests, and Miri per test binary, on the
  wave's tip before any change and again on your landed tree where your fix could affect
  them. The dashboard's usage cells are the assistant's; fill your landing cells only.
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
  the variants' generation of your wave, `crustify/builds.gen4`, whose link holds the safe crate's
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
