# wave-226/batch-5/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/thread.c/wave-226/batch-5`, on `crustify/batches/wave-226/batch-5`.
- Worklist: `crustify/artifacts/modules/libgit2/src/util/thread.c/wave-226/batch-5/run-1/task.json` in it.
- Batch: `wave-226/batch-5` of the module `libgit2/src/util/thread.c` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-226`. Name it `refs/heads/crustify/waves/wave-226` in every git command.

## Instructions

- No `objectives` are given, so every objective that applies runs, `fix` included (the
  settings' `review-objectives` answer is `all`).
- You review the three `port` batches of `src/util/thread.c`'s symbols, in
  `src/libgit2/src/util/thread.rs`, as they landed on the wave branch: `wave-226/batch-2`
  (`8aa231a69`), the 19 lock and thread macros as `Mutex<T>`, `Condvar`, `RwLock<T>`, their
  guards, `spawn` and `JoinHandle<T>` over the standard library's; `wave-226/batch-1`
  (`955c56eb7`), the 17 atomics as methods of `Atomic32Ref` and `Atomic64Ref` and the generic
  `atomic_compare_and_swap`, `atomic_swap` and `atomic_load` over a sealed `AtomicWord`; and
  `wave-226/batch-3` (`d3d90f565`), thread-local storage as `TlsKey<T>` over a registry and a
  `thread_local!` table, with the module's gateway under `ffi-export-src-util-thread-c`, off.
  Wave gates: no; the module pass, which follows you, runs them. The types' batch,
  `wave-226/batch-0`, was reviewed by `wave-226/batch-4`.
- One departure, the assistant's, named so you keep it. The batches' task messages wrongly
  said that a `static inline` function the remaining C expands stays in its header,
  unexported; `porting-symbols`' Static inline functions section exports each at the switch,
  its header definition replaced by a prototype. So the 11 atomics of `thread.h` and
  `git_threads_global_init` of `unix/pthread.h` have no export. Whether they get one is with
  the user, since an export makes every reference count the remaining C changes a call into
  Rust (`crustify/evolution/static-inline-functions-kept-in-their-headers`, and
  `header-module-exports` on `integer.h`'s wave branch, `crustify/waves/wave-223`). File it
  as a finding with what each choice costs, measured where you can, and do not patch it:
  add no export for those twelve functions and change no header.
- Judge above all:
  - batch 2's `impl Drop` blocks written only to carry the `_free` macros' anchors: an
    evolution review noted that an empty `Drop` makes `Mutex<&'a T>` stricter under dropck
    than std's and blocks moving fields out; decide where those anchors belong
    (`crustify/evolution/std-symbol-alias-stand-ins`, whose change was rejected for stating
    "that type's method" where the batch anchored unlocks on guards, `spawn` as a free
    function and `join` on `JoinHandle`, and for contradicting the conventions' anchor rule);
  - poisoning, guards that are not `Send`, and `'static` thread captures, against what the
    C callers that will be ported onto these types do (`git_odb.lock`, the cache, the
    refdb), since a later module's struct may embed a lock both Rust and C take;
  - batch 3's thread-local storage: destructor order and re-entry at thread exit, the main
    thread's values destroyed at `exit`, key ids never reused where glibc reuses them, and
    the exports `errors.c` and `runtime.c` call once the module switches, against the lesson
    on `threadstate_get` reading TLS key 0 before `git_error_global_init`;
  - the atomics' orderings against C's builtins, and C's plain reads of `.val`.
- `build.rs` now passes clang's coverage flags to the shims under the coverage variant
  (batch 1): check that it changes no other variant.
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
