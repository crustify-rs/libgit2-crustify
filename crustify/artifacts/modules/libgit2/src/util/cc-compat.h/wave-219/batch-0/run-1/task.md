# wave-219/batch-0/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/cc-compat.h/wave-219/batch-0`, on `crustify/batches/wave-219/batch-0`.
- Worklist: `crustify/artifacts/modules/libgit2/src/util/cc-compat.h/wave-219/batch-0/run-1/task.json` in it.
- Batch: `wave-219/batch-0` of the module `libgit2/src/util/cc-compat.h` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-219`. Name it `refs/heads/crustify/waves/wave-219` in every git command.

## Instructions

- Your worklist is `src/util/cc-compat.h`'s whole `port`: the five macros of wave 219, the module's only wave. The
  module's file is scaffolded at `src/libgit2/src/util/cc_compat_h.rs` (base `f7f0eaaa0`). Its module pass follows your landing,
  and is its only review.
- `src/util/cc-compat.h` is a header-only module with no translation unit: its switch at integration removes
  nothing from `c/`, and the header stays while remaining C includes it. An item the
  remaining C still expands from the header needs no export; say in the module's docs what
  stays in the header and why. Where the header has several configured branches, port what
  this build defines, as `porting-symbols`' Macros section and `strnlen_h.rs` do.
- List in your final message the C test cases tied to the module that none of your
  `unit_pub` tests carries over (`modules.json` records none tied to it).
- Five batches of five modules run at once on the campaign's side (`wave-219/batch-0`,
  `wave-220/batch-0`, `wave-221/batch-0`, `wave-223/batch-0`, `wave-226/batch-0`), and
  another campaign shares the host. Cap your builds and tests at 24 jobs
  (`CARGO_BUILD_JOBS=24`, `RUST_TEST_THREADS=24`, and the same for C builds). Keep every
  scratch file, log and scratch build in your worktree's `crustify/tmp/`, and delete scratch
  builds you no longer need before you end: the disk is 93% full. End only processes of your
  own worktree. A wait loop over `pgrep -f` matches its own command line and never ends:
  wait on a PID.
- Your run dir in the worktree holds `task.json` and `task.md`; commit them with your
  landing. The process records of your run (`session_id`, `stream.jsonl`, `stderr.log`,
  `exit_code`) are kept by the assistant in the base checkout, not in your worktree.
- Four modules have switched from C: `src/util/utf8.c`, `src/cli/opt.c`, `deps/llhttp/api.c`
  and `deps/llhttp/http.c`. `c/` is the fork's without them (`230c9dbdb`), `build.json` is at
  version 7, and your worktree's `crustify/builds` links to the variants' current
  generation, `crustify/builds.gen5`, whose link holds the safe crate's Rust static library;
  you never rebuild it. Read `crustify/LESSONS.md` before your first build; the lessons on
  test placement (`tests/unit_pub`) and on a module's switch apply to this module.
- Every end gate passed at `fa1c04754`, the tip before your module's base: there is no
  known failure to set aside, so a failing test is your change's or a finding. BSan runs as
  `testing-gates` has it; `crustify/scripts/slow-list.txt` is the slow list.
- An ffimap scan of this crate names `--sys-crates crate::sys,pcre2_sys` and your module's
  export feature, if it has one.
- `crustify/scripts/coverage/report.py`'s scope check fails when run from the repository
  root (proposal `coverage-report-relative-rows`): run it from the scratch tree's
  `src/<link-unit>`.
- The dashboard's usage cells are the assistant's; fill your landing cells only.
- The campaign's first ship point is `src/libgit2/repository.c` with the modules it reaches
  (the settings' `ship` answer): this module is one of them, so its exports serve C that
  stays past that point.
