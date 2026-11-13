# wave-186/batch-2/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/git2/src/cli/opt.c/wave-186/batch-2`, on `crustify/batches/wave-186/batch-2`.
- Worklist: `crustify/artifacts/modules/git2/src/cli/opt.c/wave-186/batch-2/run-1/task.json` in it.
- Batch: `wave-186/batch-2` of the module `git2/src/cli/opt.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-186`. Name it `refs/heads/crustify/waves/wave-186` in every git command.

## Instructions

- Your worklist is the `type` route of `src/cli/opt.c`'s waves 187 and 188, folded onto wave
  186's branch: the structs `cli_opt_spec`, then `cli_opt` and `cli_opt_parser`, which use
  it, as `wrap`: other modules' C (the CLI's commands) uses their layouts, so they keep them
  behind a safe API until their `port/layout` stages (waves 1988 and 2404).
- `wave-186/batch-0` (`8a149ac08`) ported the module's four enums in
  `src/git2/src/cli/opt.rs`: `Kind` (`cli_opt_type_t`), `Status` (`cli_opt_status_t`),
  `Usage` and `ParseFlags`, with crate-private conversions to and from C's integers that wait
  for their first callers, your accessors among them. `wave-186/batch-1` is porting the
  macro `spec_is_option_type` and `support_gnu_style` beside you in the same file: its
  landing may meet yours under the landing lock, each keeping the other's items. The
  module's other symbols (waves 188 to 193, `port`) follow you and consume your types
  natively.
- `opt.c` is the first module of the `git2` link unit (the CLI): its raw bindings are
  `sys/git2/` (`crate::sys::git2`). Batch 0 found that packaging the crate in bundled mode
  needs the unpublished wrappers patched on `crates-io` and the OpenSSL, PCRE2 and zlib
  installs on `CMAKE_PREFIX_PATH` (`crustify/evolution/packaging-gate-procedure`, on the wave
  branch): use the command it records.
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
