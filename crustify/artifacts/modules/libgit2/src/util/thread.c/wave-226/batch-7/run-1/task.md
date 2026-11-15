# wave-226/batch-7/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/thread.c/wave-226/batch-7`, on `crustify/batches/wave-226/batch-7`.
- Worklist: `crustify/artifacts/modules/libgit2/src/util/thread.c/wave-226/batch-7/run-1/task.json` in it.
- Batch: `wave-226/batch-7` of the module `libgit2/src/util/thread.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-226`. Name it `refs/heads/crustify/waves/wave-226` in every git command.

## Instructions

- No `objectives` are given, so every objective that applies runs, `fix` included (the
  settings' `review-objectives` answer is `all`).
- This is `src/util/thread.c`'s module pass. The module's run landed five translation
  batches and two reviews on this wave branch (waves 226 to 228 folded): `wave-226/batch-0`
  (`3f7f3ab89`), the `wrap` of `git_atomic32`, `git_atomic64` and `git_thread`, reviewed by
  `wave-226/batch-4` (`5c7b62f0e`); `batch-2` (`8aa231a69`), `batch-1` (`955c56eb7`) and
  `batch-3` (`d3d90f565`), the 43 symbols, reviewed by `wave-226/batch-5` (`58d3dc711`),
  which fixed a forked child's hang at `exit()`, moved the `_free` anchors off empty `Drop`
  impls and added `TlsKey::with`; and `wave-226/batch-6` (`2f2ffb73d`), which gave the
  twelve `static inline` functions of `thread.h` and `unix/pthread.h` their exports, the
  user's decision, and has no review but you: review it for every objective to full depth.
- Wave gates: yes. Module end gates: yes, this is the last wave of the module's `wrap` and
  `port` items. A departure, the user's, named so you keep it: from 2026-11-15 the settings'
  `sanitizers` answer runs BSan over the workspace only at a ship point. Your end gates
  are every stage but BSan (format, lints with the feature off and on, plain, ASan+UBSan,
  TSan, Miri per test binary), less the slow list; run `end-gates` without its BSan stage.
  You still run BSan on the tests you add or change and on your reproducers, each named and
  alone, under a `timeout`.
- The module switches from C at its integration, right after you land: `thread.c` leaves
  `c/`, the twelve header definitions become prototypes, the feature
  `ffi-export-src-util-thread-c` is deleted, and every remaining C file reaches the
  atomics, thread-local storage and `git_threads_global_init` through the gateway: all of
  libgit2's reference counts, its error state (`errors.c`'s `threadstate_get`) and its
  runtime initialisation run on these exports past the first ship point. Review the gateway
  thoroughly for what that C relies on: batch 6's exports abort on a null pointer where C's
  builtin would crash, and build their handles through `CRefSeam::from_ptr` while other
  threads write the count (`crustify/evolution/ffibox-atomic-layout-types`). Batch 6's
  `switch-rehearsal.md`, in its run dir, holds the prototypes and the rehearsal on `plain`
  (the C tests match the baseline).
- Before the equivalence tests retire at the switch, check that the unit tests keep what
  they check, as `ubc_check.c`'s module pass did by seeding faults
  (`crustify/evolution/module-pass-seeds-faults-before-the-switch` on
  `refs/heads/crustify/devel`), and list each equivalence test with the unit test that keeps
  it or the reason it retires, as `equiv-retirement.md` in your run dir.
- Say in your final message what the switch must delete or rewrite in the Rust beside the
  feature, what its gates should run, and the C test cases tied to the module that no
  `unit_pub` test carries over and that reach its internals, which leave at the switch
  (the batches' lists agree: `object/cache.c`'s two, `pack/sharing.c`, `pack/threadsafety.c`,
  the `repo/getters.c` and `repo/setters.c` refcount cases, `threads/basic.c`'s `cache` and
  the second half of `exit`, `threads/diff.c`'s two and `threads/refdb.c`'s one; say which
  of them only call the module and so stay).
- Batch 6 measured an increment-and-decrement pair at 101 ns through the exports in the
  dev-profile Rust library the variants link, against 16 ns inlined and 21 to 27 ns on the
  release profile. The user plans cross-language LTO later: change no build setting.
- The wave branch's `c/` is the fork's `230c9dbdb` and its builds `crustify/builds.gen5`;
  the working branch has moved to `e717996da` and gen7 with two switches, which this branch
  takes at its integration.
- No other batch of the campaign runs now; another campaign shares the host. Cap your builds
  and tests at 48 jobs (`CARGO_BUILD_JOBS=48`, `RUST_TEST_THREADS=48`, and the same for C
  builds). Keep every scratch file, log and scratch build in your worktree's
  `crustify/tmp/`, and delete scratch builds you no longer need before you end: the disk is
  94% full. End only processes of your own worktree. A wait loop over `pgrep -f` matches its
  own command line and never ends: wait on a PID.
- Your run dir in the worktree holds `task.json` and `task.md`; commit them with your
  landing. The process records of your run (`session_id`, `stream.jsonl`, `stderr.log`,
  `exit_code`) are kept by the assistant in the base checkout, not in your worktree.
- Four modules have switched from C: `src/util/utf8.c`, `src/cli/opt.c`, `deps/llhttp/api.c`
  and `deps/llhttp/http.c`. `c/` is the fork's without them (`230c9dbdb` on this wave branch), `build.json` is at
  version 7, and your worktree's `crustify/builds` links to the variants' current
  generation, `crustify/builds.gen5` (the working branch is at gen7), whose link holds the safe crate's Rust static library;
  you never rebuild it. Read `crustify/LESSONS.md` before your first build; the lessons on
  test placement (`tests/unit_pub`) and on a module's switch apply to this module.
- Every end gate passed at `fa1c04754`, the tip before your module's base: there is no
  known failure to set aside, so a failing test is your change's or a finding. BSan runs as
  the instruction above has it; `crustify/scripts/slow-list.txt` is the slow list.
- An ffimap scan of this crate names `--sys-crates crate::sys,pcre2_sys` and your module's
  export feature, if it has one.
- `crustify/scripts/coverage/report.py`'s scope check fails when run from the repository
  root (proposal `coverage-report-relative-rows`): run it from the scratch tree's
  `src/<link-unit>`.
- The dashboard's usage cells are the assistant's; fill your landing cells only.
- The campaign's first ship point is `src/libgit2/repository.c` with the modules it reaches
  (the settings' `ship` answer): this module is one of them, so its exports serve C that
  stays past that point.
