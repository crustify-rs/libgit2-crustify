# wave-194/batch-5/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/deps/llhttp/api.c/wave-194/batch-5`, on `crustify/batches/wave-194/batch-5`.
- Worklist: `crustify/artifacts/modules/libgit2/deps/llhttp/api.c/wave-194/batch-5/run-1/task.json` in it.
- Batch: `wave-194/batch-5` of the module `libgit2/deps/llhttp/api.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-194`. Name it `refs/heads/crustify/waves/wave-194` in every git command.

## Instructions

- This review batch takes the `type` route's landings of `deps/llhttp/api.c`'s folded waves:
  `wave-194/batch-0` (`1ead111ee`), which bound `llhttp.c`'s parser struct and four enums
  ahead of `api.c` (`Parser`, `ParserRef`, `ParserMut`, `Errno`, `Method`, `Status`,
  `ParserType`, in `src/libgit2/deps/llhttp/llhttp.rs`), and `wave-194/batch-2`
  (`97dea9da3`), which bound `api.c`'s callback table (`Settings`, `SettingsRef`,
  `SettingsMut`, in `src/libgit2/deps/llhttp/api.rs`). All are `wrap`: the struct and the
  table keep their C layouts until their `port/layout` stages (waves 1167 and 1165), and
  `llhttp.c`'s wave 238 ports the enums later, so review them on their public API, for every
  objective that applies there, `ergonomics` included, as `reviewing`'s Depth section gives.
  Points to judge: `data` and `settings` staying raw addresses behind `unsafe` setters, and
  `ParserMut::set_settings(Option<SettingsRef<'static>>)`; the six scheduled fields batch 0
  gave no accessor (`crustify/evolution/wrap-ahead-field-users`); `Settings` as `Send` and
  `Sync`.
- Later batches built on these types and are reviewed apart: batch 1 (`2c69341c3`, the
  constructor, `parse`, the callback types and the maps), batch 3 (`2a51ccb3f`) and batch 4,
  which is running beside you in `api.rs`: its landing may meet yours under the landing
  lock, each keeping the other's work. This batch carries no wave gates: run the module's
  tests and the scoped gates of what you change.
- The wave's tree is past `src/util/utf8.c`'s switch from C: `crustify/builds` in your
  worktree links to the variants' new generation, `crustify/builds.gen1`. Read the switch's
  Notes entry in the dashboard and its lesson in `crustify/LESSONS.md` before your first
  build.
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
