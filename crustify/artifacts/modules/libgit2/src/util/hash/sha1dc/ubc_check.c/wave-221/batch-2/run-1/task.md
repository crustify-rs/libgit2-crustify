# wave-221/batch-2/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/hash/sha1dc/ubc_check.c/wave-221/batch-2`, on `crustify/batches/wave-221/batch-2`.
- Worklist: `crustify/artifacts/modules/libgit2/src/util/hash/sha1dc/ubc_check.c/wave-221/batch-2/run-1/task.json` in it.
- Batch: `wave-221/batch-2` of the module `libgit2/src/util/hash/sha1dc/ubc_check.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-221`. Name it `refs/heads/crustify/waves/wave-221` in every git command.

## Instructions

- No `objectives` are given, so every objective that applies runs, `fix` included (the
  settings' `review-objectives` answer is `all`).
- You review `wave-221/batch-0` (`2492d2a9f`), the `wrap` of `dv_info_t` as `DvInfo` in
  `src/libgit2/src/util/hash/sha1dc/ubc_check.rs`. Wave gates: no; the module pass runs them.
  The wrapper keeps its C layout until `sha1.c` is ported: review it to the depth
  `reviewing`'s Depth section gives a wrapper a later stage replaces.
- The module's `port` batch, `wave-221/batch-1`, runs beside you on the same wave branch and
  builds `sha1_dvs` from `DvInfo::new`: a change of yours to the type's API reaches it at its
  rebase, so keep such a change to what a finding requires, and name it in your final
  message.
- The batch wrote no equivalence test, since C has no routine over the type alone, and
  added an occurrence to the open proposal `package-gate-procedure`.
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
