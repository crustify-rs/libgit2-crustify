# wave-223/batch-1/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/integer.h/wave-223/batch-1`, on `crustify/batches/wave-223/batch-1`.
- Worklist: `crustify/artifacts/modules/libgit2/src/util/integer.h/wave-223/batch-1/run-1/task.json` in it.
- Batch: `wave-223/batch-1` of the module `libgit2/src/util/integer.h` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-223`. Name it `refs/heads/crustify/waves/wave-223` in every git command.

## Instructions

- No `objectives` are given, so every objective that applies runs, `fix` included (the
  settings' `review-objectives` answer is `all`).
- This is `src/util/integer.h`'s module pass and its only review: its run landed one
  translation batch, `wave-223/batch-0` (`9b43c913d`), the module's whole `port`: six range
  checks generic over `TryInto` (`fits_usize`, `fits_isize`, `fits_u16`, `fits_u32`,
  `fits_c_ulong`, `fits_c_int`) and six overflow-checked `const fn`s over `checked_*` that
  return `None` on overflow, with no unsafe code and no export. Wave gates: yes. Module end
  gates: yes, this is the module's last wave. The module's integration follows your
  landing; a header-only module's switch removes nothing from `c/`.
- Judge three decisions of the batch above all. It does not copy three C behaviours it
  calls defects (`git__is_ssizet` never false; `git__is_sizet` and `git__is_ulong` accepting
  negative values on this build): confirm each is a C defect and not a contract a later
  port of a C caller relies on. It returns nothing on overflow where C's intrinsics write
  the wrapped result, which `try_delta` in `pack-objects.c` reads. And it follows this
  build's branch of `git__multiply_int64_overflow`, whose plain-C fallback differs on four
  inputs, filing `crustify/evolution/config-branches-the-target-never-builds`.
- It allowlisted the six `git__is_*` functions for its equivalence tests, which
  `static_fns.c` now wraps; `git__add_int_overflow` and `git__sub_int_overflow` are not
  compared with C. All three cases of `tests/util/integer.c` are said to be carried over:
  confirm it, and list any case none carries over.
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
