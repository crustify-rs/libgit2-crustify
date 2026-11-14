# wave-186/batch-3/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/git2/src/cli/opt.c/wave-186/batch-3`, on `crustify/batches/wave-186/batch-3`.
- Worklist: `crustify/artifacts/modules/git2/src/cli/opt.c/wave-186/batch-3/run-1/task.json` in it.
- Batch: `wave-186/batch-3` of the module `git2/src/cli/opt.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-186`. Name it `refs/heads/crustify/waves/wave-186` in every git command.

## Instructions

- This review batch takes the `type` route's landings of `src/cli/opt.c`'s folded waves
  (186 to 188): `wave-186/batch-0` (`8a149ac08`), which ported the four enums as `Kind`,
  `Status`, `Usage` and `ParseFlags`, and `wave-186/batch-2` (`fe6b85bfa`), which bound the
  structs `cli_opt_spec`, `cli_opt` and `cli_opt_parser` as `Spec<'p>`, `Opt<'p>` and
  `Parser<'p>`, `wrap`, with `Target<'p>`, `Arg<'p>`, `ArgsSlot` and `SpecTable`. Depth, as
  `reviewing`'s Depth section gives it: the structs are wrappers whose `port/layout` is
  planned (waves 1988 and 2404), so review them on their public API, for every objective that
  applies there, `ergonomics` included; the enums are ported for good.
- The module's remaining symbols (waves 188 to 193, `port`) consume this API natively in the
  next batches, and are reviewed apart; the module pass follows them. This batch carries no
  wave gates: run the module's tests and the scoped gates of what you change.
- Batch 2 filed `type-erased-field-tagged-by-a-sibling` (a `binding-types` change on an
  evolution branch) and `compile-fail-doctest-codes-unchecked-on-stable`; batch 0 filed
  `enum-integer-form-crate-private` and `packaging-gate-procedure`. Judge the code against
  the skills as they are on your revision, not against the unlanded changes.
- No `objectives` are given, so every objective that applies runs, `fix` included. The
  dashboard's usage cells are the assistant's; fill your landing cells only.
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
