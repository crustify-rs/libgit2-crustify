# wave-229/batch-0/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/utf8.c/wave-229/batch-0`, on `crustify/batches/wave-229/batch-0`.
- Worklist: `crustify/artifacts/modules/libgit2/src/util/utf8.c/wave-229/batch-0/run-1/task.json` in it.
- Batch: `wave-229/batch-0` of the module `libgit2/src/util/utf8.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-229`. Name it `refs/heads/crustify/waves/wave-229` in every git command.

## Instructions

- Your worklist is `src/util/utf8.c`'s whole `port`: the items of waves 229, 230 and 231,
  folded into this batch on wave 229's branch, since each depends on the one before
  (`utf8proc_utf8class`, then `utf8_charlen`, then the three `git_utf8_*` functions). The
  module's file is scaffolded at `src/libgit2/src/util/utf8.rs` (base `2216b0cd0`).
- The module switches from C at its integration, once its module pass has landed: the C
  that remains then calls `git_utf8_char_length`, `git_utf8_iterate` and
  `git_utf8_valid_buf_length` through your exports, whose `port/unexport` stages are waves
  232, 860 and 1531. Write the `ffi_export` gateway under the module's export feature, off,
  as the conventions' Exports and Export features sections describe; it is the first export
  feature of the crate, so the feature's declaration in `Cargo.toml` is yours too.
- List in your final message the C test cases tied to the module (`modules.json`'s `tests` of
  `src/util/utf8.c`) that none of your `unit_pub` tests carries over, for its switch.
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
