# thread.c's switch, rehearsed under ASan+UBSan and TSan

What `wave-226/batch-7/run-1`, the module pass, ran under `crustify/tmp/`, beside
`wave-226/batch-6/run-1`'s rehearsal on `plain` (its `switch-rehearsal.md`), whose step 1 it
repeats. Nothing was committed to `c/`, and the scratch was deleted after the run.

1. A copy of the wave's tip (`2f2ffb73d`, `git archive`), its `c/` the fork at `230c9dbdb`
   with the fork's `.git` (`git clone --shared` of the worktree's `c/`; a `git archive` alone
   fails `perf::merge` and skips three cases, `crustify/LESSONS.md`'s
   `switch-rehearsal-c-is-a-git-checkout`). `src/util/thread.c` is removed, and the twelve
   `GIT_INLINE` definitions of `thread.h` and `unix/pthread.h` are replaced by batch 6's
   prototypes, by the script `switch-headers.py`'s edit: in `thread.h`, the `GIT_THREADS`
   branch from `git_atomic32_set`'s comment to the `#else` of the build without threads
   becomes the eleven `extern` prototypes; in `unix/pthread.h`,
   `GIT_INLINE(int) git_threads_global_init(void) { return 0; }` becomes
   `extern int git_threads_global_init(void);`.
2. The copy's `crustify/scripts/rust-staticlib.sh` builds with
   `--features bindgen,ffi-export-src-util-thread-c`. `asan-ubsan` and `tsan` were configured
   and built from `build.json`'s commands (version 7) in build dirs of the copy, with
   `CRUSTIFY_BINDINGS_UPDATE=1`. Both `libcrustify.a` hold 208 objects, `thread.c.o` gone;
   both `libgit2_tests` define `git_atomic32_inc`, `git_threads_global_init` and
   `git_tlsdata_get` from the Rust library, and no `<name>__extern` wrapper of the module.
3. `crustify/scripts/c-tests.sh` over each, 16 suites side by side:

   | variant | passed | failed | skipped | non-zero suites | baseline |
   |---|---:|---:|---:|---|---|
   | `asan-ubsan` | 3442 | 0 | 55 | none | matches |
   | `tsan` | 3442 | 0 | 55 | the baseline's 10 | matches |

   Under TSan, `threads::diff` and `threads::refdb` exit 66, as in the baseline, each with one
   report and no Rust frame: `filter.c`'s lazy `filter_initialize` and glibc's `tzset`. The
   module's tied suites, on both: `threads::atomic` 13, `threads::basic` 4,
   `threads::tlsdata` 2, `threads::diff` 2, `threads::refdb` 1, `pack::sharing` 1,
   `pack::threadsafety` 1, `repo::getters` 5, `repo::setters` 5 and `util_tests`' `init` 3,
   all passing. Under ASan+UBSan, LeakSanitizer finds nothing: the error states that
   `threads::basic::set_error`'s C threads allocate are freed at their exit by the Rust
   registry's thread-local destructor, as POSIX's key destructor freed them.

So the switch, as batch 6 recorded it, leaves the C tests at the baseline on `plain`,
`asan-ubsan` and `tsan`. Not rehearsed: `coverage`, `bsan` and the packaging gate.
