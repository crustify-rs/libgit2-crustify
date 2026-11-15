# wave-221/batch-3/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/hash/sha1dc/ubc_check.c/wave-221/batch-3`, on `crustify/batches/wave-221/batch-3`.
- Worklist: `crustify/artifacts/modules/libgit2/src/util/hash/sha1dc/ubc_check.c/wave-221/batch-3/run-1/task.json` in it.
- Batch: `wave-221/batch-3` of the module `libgit2/src/util/hash/sha1dc/ubc_check.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-221`. Name it `refs/heads/crustify/waves/wave-221` in every git command.

## Instructions

- No `objectives` are given, so every objective that applies runs, `fix` included (the
  settings' `review-objectives` answer is `all`).
- This is `src/util/hash/sha1dc/ubc_check.c`'s module pass. A departure from `reviewing`'s
  Review batches section, the assistant's, named so you keep it: the module's run landed two
  translation batches, and only the first, `wave-221/batch-0` (`2492d2a9f`, the `wrap` of
  `dv_info_t` as `DvInfo`), had a review of its own, `wave-221/batch-2` (`edfd99a0e`,
  `aa4ec4d3f`). The second, `wave-221/batch-1` (`e816c88bb`), the module's 35 symbols, has
  none: you are its only review, as a module pass is for a module of one batch, so review
  its code for every objective to full depth, and the type's wrapper as a module pass does.
- Batch 1 ported `DVMASKSIZE`, the 32 `DV_*_BIT` constants, the table `SHA1_DVS:
  [DvInfo; 33]`, built at compile time through `DvInfo::new`, and `ubc_check(&[u32; 80]) ->
  [u32; DVMASKSIZE]`, a statement-for-statement translation of the generated C produced by
  a script. Its gateway exports `ubc_check` and `sha1_dvs` under
  `ffi-export-src-util-hash-sha1dc-ubc-check-c`, off. With the feature off the table is a
  Rust static; with it on, the gateway's exported static is the storage, re-exported as
  `SHA1_DVS` (`crustify/evolution/exported-global-storage-before-the-switch`).
- Wave gates: yes. Module end gates: yes, this is the last wave of the module's `wrap` and
  `port` items. The module switches from C at its integration, right after you land:
  `ubc_check.c` leaves `c/`, the feature is deleted, and `sha1.c`, which stays C until its
  own port and past the first ship point, calls both exports on every SHA-1 block the C
  hashes. Review the gateway thoroughly for what `sha1.c` relies on. Your gates include the
  export feature on: check and lint the crate with it, and run the module's tests, and the
  tests that hash through `sha1.c`, with the exports replacing C's `ubc_check.c.o` in the
  link if you can do so without changing `c/` or the shared builds.
- No C test is tied to the module; `test_sha1__detect_collision_attack` (`tests/util/sha1.c`)
  is the one C case in which a recompression finds a collision. Say in your final message
  what the switch's gates should run, and list what the switch must delete or rewrite in
  the Rust beside the feature (the batch names the `cfg(not(feature))` static and the `cfg`
  on the re-export).
- Batches of five modules run at once on the campaign's side (`src/util/cc-compat.h`,
  `ctype_compat.h`, `integer.h`, `hash/sha1dc/ubc_check.c` and `thread.c`), and another
  campaign shares the host. Cap your builds and tests at 16 jobs (`CARGO_BUILD_JOBS=16`,
  `RUST_TEST_THREADS=16`, and the same for C builds), each BSan and Miri invocation under a
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
