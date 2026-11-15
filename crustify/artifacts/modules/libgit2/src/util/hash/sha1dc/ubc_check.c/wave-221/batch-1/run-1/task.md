# wave-221/batch-1/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/hash/sha1dc/ubc_check.c/wave-221/batch-1`, on `crustify/batches/wave-221/batch-1`.
- Worklist: `crustify/artifacts/modules/libgit2/src/util/hash/sha1dc/ubc_check.c/wave-221/batch-1/run-1/task.json` in it.
- Batch: `wave-221/batch-1` of the module `libgit2/src/util/hash/sha1dc/ubc_check.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-221`. Name it `refs/heads/crustify/waves/wave-221` in every git command.

## Instructions

- Your worklist is the symbols of `src/util/hash/sha1dc/ubc_check.c`, waves 221 and 222
  folded into this batch on wave 221's branch, since wave 222's two items use wave 221's:
  `DVMASKSIZE`, the 33 `DV_*_bit` constants, the table `sha1_dvs` and `ubc_check`. The
  module's file is `src/libgit2/src/util/hash/sha1dc/ubc_check.rs` (base `267e98c2f`).
- `wave-221/batch-0` (`2492d2a9f`) wrapped `dv_info_t` there as `DvInfo`, with `DvInfoRef`
  and `DvInfoMut`: `Copy`, `Send` and `Sync`, and a `const fn new` that returns `None` for a
  step outside `0..80`, a bit outside `0..32` or a non-zero mask word, so that `sha1_dvs`
  can be a `static` table. Read its Notes entry in the dashboard. A review of that batch,
  `wave-221/batch-2`, runs beside you and lands on the same wave branch: expect to rebase
  over it under the landing lock.
- The module switches from C at its integration, once its module pass has landed:
  `ubc_check.c` then leaves `c/`, and `sha1.c`, which stays C until its own port, reaches
  `sha1_dvs` and `ubc_check` through your exports. Write the `ffi_export` gateway under the
  module's export feature, off, as the conventions' Exports and Export features sections
  describe, and declare the feature in `Cargo.toml`.
- List in your final message the C test cases tied to the module that none of your
  `unit_pub` tests carries over (`modules.json` records none tied to it), and what in the
  remaining C exercises `ubc_check` and `sha1_dvs`, for the switch's gates.
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
