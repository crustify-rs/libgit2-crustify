# wave-200/batch-1/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/deps/ntlmclient/compat.h/wave-200/batch-1`, on `crustify/batches/wave-200/batch-1`.
- Worklist: `crustify/artifacts/modules/libgit2/deps/ntlmclient/compat.h/wave-200/batch-1/run-1/task.json` in it.
- Batch: `wave-200/batch-1` of the module `libgit2/deps/ntlmclient/compat.h` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-200`. Name it `refs/heads/crustify/waves/wave-200` in every git command.

## Instructions

- This is `deps/ntlmclient/compat.h`'s module pass and its only review: its run landed one
  translation batch, `wave-200/batch-0` (`4b6e5a4db`), which ported the `MIN` macro as the
  generic `pub fn min<T: PartialOrd>(x: T, y: T) -> T`, with no unsafe code and no export.
  The module's switch at its integration changes no C: the header has no translation unit,
  and `ntlm.c` keeps including it.
- The batch ported the macro as a function although it evaluates the operand it returns
  twice, and filed `crustify/evolution/macro-incidental-double-evaluation` on it; it wrote no
  equivalence test, the macro's C users (`ntlm.c`) being unreachable through the raw bindings.
  Judge both against `porting-symbols`' Macros section and `testing`.
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
