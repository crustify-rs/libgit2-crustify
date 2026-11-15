# wave-226/batch-2/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/thread.c/wave-226/batch-2`, on `crustify/batches/wave-226/batch-2`.
- Worklist: `crustify/artifacts/modules/libgit2/src/util/thread.c/wave-226/batch-2/run-1/task.json` in it.
- Batch: `wave-226/batch-2` of the module `libgit2/src/util/thread.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-226`. Name it `refs/heads/crustify/waves/wave-226` in every git command.

## Instructions

- Your worklist is the locks and threads of `src/util/thread.c`, 19 macros of wave 226 defined
  in `unix/pthread.h`: `git_mutex` and its four operations, `git_cond` and its four,
  `git_rwlock` and its six, `git_thread_create` and `git_thread_join`.
- The type batch left a safe way to store a thread's id, around a `JoinHandle` or otherwise,
  to `git_thread_create`'s port: that design is yours.
- `wave-226/batch-0` (`3f7f3ab89`) wrapped the module's types in
  `src/libgit2/src/util/thread.rs` (base `6faca73b7`): `Atomic32` and `Atomic64`
  (`AtomicSsize` the alias of `git_atomic_ssize`), laid out by hand over `UnsafeCell`, whose
  shared handles lend the integer as `&AtomicI32` / `&AtomicI64` through `val()`, and
  `Thread` (`git_thread`), whose `pthread_t` is std's `RawPthread` and which is written only
  through the `unsafe fn ThreadMut::set_thread`. Read its Notes entry in the dashboard and
  `crustify/evolution/ffibox-atomic-layout-types`.
- Three `port` batches of this module run at once and land on the same wave branch, each
  writing its own items into `thread.rs` and its tests: `wave-226/batch-1`, the atomics;
  `wave-226/batch-2`, the mutex, condition, read-write lock and thread macros;
  `wave-226/batch-3`, thread-local storage and `git_threads_global_init`. A review of the
  type batch, `wave-226/batch-4`, runs beside them. Expect to rebase over the others under
  the landing lock, keeping both sides of every additive conflict. Keep your items in a
  section of their own in the file and in test files or test modules of their own, so that
  the rebases conflict as little as possible.
- The module switches from C at its integration, once its module pass has landed:
  `thread.c` then leaves `c/`, and the remaining C reaches what it still calls through the
  module's exports. The module has one export feature, `ffi-export-src-util-thread-c`, off:
  the first of the three batches to need it declares it in `Cargo.toml`, as
  `ffi-export-src-util-utf8-c` was, and the others find it at their rebase. A macro or a
  `static inline` function the remaining C expands from `thread.h` or `unix/pthread.h` stays
  in the header and needs no export, as `porting-symbols` describes.
- List in your final message the C test cases tied to the module (`modules.json`'s `tests`
  of `src/util/thread.c`) that none of your `unit_pub` tests carries over, for its switch.
- The lesson "Before `git_libgit2_init`, probe libgit2's allocator only through a release"
  applies to any test of yours that runs libgit2 before its initialisation.
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
