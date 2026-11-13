# wave-194/batch-0/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/deps/llhttp/api.c/wave-194/batch-0`, on `crustify/batches/wave-194/batch-0`.
- Worklist: `crustify/artifacts/modules/libgit2/deps/llhttp/api.c/wave-194/batch-0/run-1/task.json` in it.
- Batch: `wave-194/batch-0` of the module `libgit2/deps/llhttp/api.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-194`. Name it `refs/heads/crustify/waves/wave-194` in every git command.

## Instructions

- This is a wrap batch ahead of its consumer: `deps/llhttp/api.c`'s `port` waves (194 to 196)
  use twelve units of `deps/llhttp/llhttp.c`, which the order cuts after it. You bind the five
  types among them, `llhttp__internal_s` (with its typedefs and the fields its consumers use)
  and the enums `llhttp_errno`, `llhttp_method`, `llhttp_status` and `llhttp_type`, as `wrap`,
  in their own module's file, `src/libgit2/deps/llhttp/llhttp.rs` (scaffolded at `2216b0cd0`;
  `llhttp.c` is frozen). `llhttp.c`'s own wave 238 ports them later, keeping the safe API you
  give them. A second wrap batch binds the symbols (`llhttp_cb`, `llhttp_data_cb`,
  `llhttp__internal_init`, `llhttp__internal_execute` and the three `HTTP_*_MAP` macros) once
  you have landed, and `api.c`'s `llhttp_settings_s` and its ports follow.
- Your batch belongs to `api.c`'s wave 194 and lands on its branch; your task's `module` is
  `deps/llhttp/api.c`, your items' homes are `llhttp.c`'s.
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
