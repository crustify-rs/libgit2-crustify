# wave-220/batch-1/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/ctype_compat.h/wave-220/batch-1`, on `crustify/batches/wave-220/batch-1`.
- Worklist: `crustify/artifacts/modules/libgit2/src/util/ctype_compat.h/wave-220/batch-1/run-1/task.json` in it.
- Batch: `wave-220/batch-1` of the module `libgit2/src/util/ctype_compat.h` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-220`. Name it `refs/heads/crustify/waves/wave-220` in every git command.

## Instructions

- No `objectives` are given, so every objective that applies runs, `fix` included (the
  settings' `review-objectives` answer is `all`).
- This is `src/util/ctype_compat.h`'s module pass and its only review: its run landed one
  translation batch, `wave-220/batch-0` (`a3f4f5958`), the module's whole `port`: eight
  native `const fn`s over `u8` (`is_alnum`, `is_alpha`, `is_digit`, `is_print`, `is_space`,
  `is_xdigit`, `to_lower`, `to_upper`), with no unsafe code and no export. Wave gates: yes.
  Module end gates: yes, this is the module's last wave. The module's integration follows
  your landing; a header-only module's switch removes nothing from `c/`.
- The batch took each macro's argument after the header's cast to `unsigned char`, and
  filed `crustify/evolution/config-branches-over-the-rust-domain` on it. Judge that reading
  against `porting-symbols`' Macros section. It left six macros uncompared at their C
  expansion sites, comparing them with glibc's functions instead, and noted that
  `mailmap.rs` holds a private closure that duplicates `git__isspace`.
- It ran no TSan, as it asserts no `Send` or `Sync`; your module end gates run every stage.
- The batch recorded the dashboard's test counts as it measured them (the recorded `equiv`
  and unit counts did not match its base); check its note.
- Batches of five modules run at once on the campaign's side (`src/util/cc-compat.h`,
  `ctype_compat.h`, `integer.h`, `hash/sha1dc/ubc_check.c` and `thread.c`), and another
  campaign shares the host. Cap your builds and tests at 24 jobs (`CARGO_BUILD_JOBS=24`,
  `RUST_TEST_THREADS=24`, and the same for C builds), each BSan and Miri invocation under a
  `timeout`. Keep every scratch file, log and scratch build in your worktree's
  `crustify/tmp/`, and delete scratch builds you no longer need before you end: the disk is
  93% full. End only processes of your own worktree. A wait loop over `pgrep -f` matches its
  own command line and never ends: wait on a PID.
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
