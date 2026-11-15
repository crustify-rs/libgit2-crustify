# wave-226/batch-6/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/thread.c/wave-226/batch-6`, on `crustify/batches/wave-226/batch-6`.
- Worklist: `crustify/artifacts/modules/libgit2/src/util/thread.c/wave-226/batch-6/run-1/task.json` in it.
- Batch: `wave-226/batch-6` of the module `libgit2/src/util/thread.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-226`. Name it `refs/heads/crustify/waves/wave-226` in every git command.

## Instructions

- This batch reworks landed items; it ports nothing new. Its worklist is the twelve
  `static inline` functions of `src/util/thread.c`'s headers that `wave-226/batch-1`
  (`955c56eb7`, the 11 atomics of `thread.h`) and `wave-226/batch-3` (`d3d90f565`,
  `git_threads_global_init` of `unix/pthread.h`) ported without an export. Their task
  messages wrongly said such a function stays in its header, unexported; the user has
  decided that they are exported as `porting-symbols`' Static inline functions section
  requires, the cost of the calls to be recovered later by cross-language LTO.
- The finding is filed: `conventions/header-functions-without-exports`, in
  `crustify/reviews/modules/libgit2/src/util/thread.c/wave-226/batch-5/`, with the measured
  costs. `src/util/integer.h`'s module pass made the same fix for its six range checks
  (`9e5a39183`, `src/libgit2/src/util/integer_h.rs`'s gateway, now on the working branch,
  which this wave branch has not taken): follow its shape.
- Add each of the twelve an export in the module's gateway, under the module's export
  feature `ffi-export-src-util-thread-c`, off, which batch 3 declared, each checked at
  compile time against its raw binding, with gateway tests. Each export keeps the C
  function's contract for the C that calls it: the same ordering, strength and wrapping as
  the builtin it replaces, over the caller's `git_atomic32`, `git_atomic64` or pointer slot.
  The native API of the atomics does not change. Change no header: the module's switch
  replaces the twelve definitions by prototypes, on the fork. The macros of both headers
  (`git_atomic_*`, `git_mutex_*` and the rest) stay as they are, expanding to the exports
  where they call these functions.
- Rehearse the switch in scratch, as `integer.h`'s module pass did
  (`crustify/artifacts/modules/libgit2/src/util/integer.h/wave-223/batch-1/run-1/switch-rehearsal.md`
  on `refs/heads/crustify/devel`): a copy of the tree whose `c/` has the twelve prototypes
  and no `thread.c`, the feature on, the `plain` variant built, the remaining C tests run
  through `c-tests.sh` against `c-baseline.json`, and the cost of an increment-and-decrement
  pair measured before and after. Record it as `switch-rehearsal.md` in your run dir, with
  the exact prototypes, for the switch. Commit nothing to `c/`.
- The module pass follows your landing and is your batch's review. Update the open
  proposals `static-inline-functions-kept-in-their-headers` and, if it has reached this
  branch, `header-module-exports` with what this run shows.
- The wave branch holds the module's whole port and its two reviews (`58d3dc711` its tip):
  `Mutex`, `Condvar`, `RwLock`, `spawn` and `JoinHandle`; the atomics; `TlsKey` and the
  gateway of the four `git_tlsdata_*` exports. Its `c/` is the fork's `230c9dbdb` and its
  builds `crustify/builds.gen5`: two modules switched on the working branch since
  (`integer.h`, `ubc_check.c`), which this branch takes at its integration.
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
