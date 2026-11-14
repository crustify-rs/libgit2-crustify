# wave-186/batch-7/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/git2/src/cli/opt.c/wave-186/batch-7`, on `crustify/batches/wave-186/batch-7`.
- Worklist: `crustify/artifacts/modules/git2/src/cli/opt.c/wave-186/batch-7/run-1/task.json` in it.
- Batch: `wave-186/batch-7` of the module `git2/src/cli/opt.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-186`. Name it `refs/heads/crustify/waves/wave-186` in every git command.

## Instructions

- This is `src/cli/opt.c`'s module pass: the module's `port` is whole on this wave's branch
  (waves 186 to 193 folded), in `src/git2/src/cli/opt.rs`: four enums, three structs kept in
  their C layouts (`wrap`, until waves 1988 and 2404), 21 symbols and five exports under
  `ffi-export-src-cli-opt-c`, off. Its landings were reviewed in the wave: the types by batch
  3 (`3829c7a06`), the symbols by batch 6 (`961150dd9`), which moved the status export onto
  the libc wrapper's byte-exact `FileRef::write_all` and fixed an interrupted write being
  written twice. Review the module whole, across its batches: its unsafe regions, its
  exports and its tests, as `reviewing`'s Review batches section describes.
- The module switches from C at its integration, right after you land: its export feature
  is deleted, `opt.c` leaves `c/`, and the CLI's C that remains (`c/src/cli/`, `main.c` and
  the `cmd_*.c`) calls the five exports and builds and reads the three structs in place.
  Review the exports and the structs' layouts for what that C relies on. Seven C defects are
  deliberately not carried over (the Notes entries of batches 4, 5 and 6 list them, with
  what the CLI sees of each after the switch); weigh the bare `-`, which batch 6 left to you.
- The module has no C tests tied to it (`modules.json`'s `tests` is empty for it): confirm
  it, and say what checks the CLI's parsing after the switch.
- Your gates are the wave's and the module's end, without BSan, and with the export feature
  on as well: check and lint the crate with it, and run the workspace's tests with the
  exports replacing C's `opt.c` in a scratch link, without changing `c/` or the shared
  builds, as the batches did.
- Two proposals batch 6 asked the assistant to file are not filed yet; the assistant files
  them: the libc wrapper's `write_all` docs leading an `io::Write` implementer to the retry
  bug, and a reproducer package repeating the workspace's `[patch]` tables.
- The wave's tree is past `src/util/utf8.c`'s switch from C: `crustify/builds` in your
  worktree links to the variants' new generation, `crustify/builds.gen1`. Read the switch's
  Notes entry in the dashboard and its lesson in `crustify/LESSONS.md` before your first
  build. `build.json`'s `test` command and `crustify/scripts/c-tests.sh` run the target's
  remaining C tests over the link with the Rust static library.
- No `objectives` are given, so every objective that applies runs, `fix` included (the
  settings' `review-objectives` answer is `all`).
- Your gates are the wave's and the module's end, without BSan: the workspace's tests on
  plain and under ASan/UBSan and TSan, and Miri per test binary, on the wave's tip before any
  change and again on your landed tree where your fix could affect them.
- The dashboard's usage cells are the assistant's; fill your landing cells only.
- Five batches of four modules run at once on this host (`wave-186/batch-0` and `batch-1`,
  `wave-194/batch-0`, `wave-200/batch-0`, `wave-229/batch-0`), and a side campaign's runs may
  join them. Cap your builds and tests at 24 jobs (`CARGO_BUILD_JOBS=24`,
  `RUST_TEST_THREADS=24`, and the same for C builds). Keep every scratch file, log and scratch
  build in your worktree's `crustify/tmp/`, and delete scratch builds you no longer need
  before you end: the disk is 92% full. End only processes of your own worktree.
- Your run dir in the worktree holds `task.json` and `task.md`; commit them with your
  landing. The process records of your run (`session_id`, `stream.jsonl`, `stderr.log`,
  `exit_code`) are kept by the assistant in the base checkout, not in your worktree.
- The tree changed on 2026-11-13: the safe crate is `libgit2-crustify` (lib
  `libgit2_crustify`), the raw bindings are `sys/<link-unit>/` reached as
  `crate::sys::<link-unit>`, stable is Rust 1.100.0, and each variant's C is the static
  archive `crustify/builds/<variant>/crustify/libcrustify.a`. `crustify/builds` in your
  worktree is a link to the base checkout's, which you never rebuild.
- Known failures that are not yours: under the nightly variants (asan-ubsan), 8 `compile_fail`
  snapshots blessed on stable mismatch, and the `CommitBuilder` compile-fail doctest (E0624).
  The plain suite passes at your base (2768 passed, 1 ignored).
- `crustify/scripts/coverage/report.py`'s scope check fails when run from the repository
  root (proposal `coverage-report-relative-rows`): run it from the scratch tree's
  `src/<link-unit>`, as `wave-225/batch-0` did.
- BSan is no regression gate of yours (the settings' `sanitizers` answer, from 2026-11-13, a
  departure from `testing-gates`' Regressions section the user asked for): never run
  `cargo bsan` over the workspace, a whole test binary or as a batch, wave or module-end
  gate; that runs once before each ship point. You may run it on small things: the tests
  your own work adds or changes, a reproducer and the regression test it becomes, each test
  named and under a `timeout`.
