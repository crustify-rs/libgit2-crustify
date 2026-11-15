# modules/libgit2/src/util/integer.h/integration/run-1: the module's switch from C

- Repository: the module's integration worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/integer.h/integration`, on the module's branch
  `crustify/modules/libgit2/src/util/integer.h`, checked out there.
- This run has no worklist and lands on no wave branch: `task.json` beside this file holds
  its run fields only. The assistant merged the working branch into the module's branch
  (`12dfd634d`, format, clippy and the plain workspace's 3199 tests green, no C changed),
  and asks you, an agent of the module's batch kind, to carry out the module's switch from C
  to Rust on top of it, in this worktree, as `porting-workflow`'s Builds section orders it.
  Commit on the checked-out branch; push nothing, anywhere. The assistant fast-forwards the
  working branch to your tip once you report.

## Instructions

This is the campaign's fifth switch and the first of a header-only module: no file leaves
`c/`, but six `static inline` definitions become prototypes. The module pass rehearsed it
on `plain` (`crustify/artifacts/modules/libgit2/src/util/integer.h/wave-223/batch-1/run-1/switch-rehearsal.md`): follow
it. `deps/llhttp/http.c`'s switch (`crustify/artifacts/modules/libgit2/deps/llhttp/http.c/integration/run-1/task.md`,
its Notes entry in the dashboard) is the closest precedent for the builds and the gates.
Read `crustify/LESSONS.md` and the scripts under `crustify/scripts/` (`build-variants.sh`,
`rust-staticlib.sh`, `c-tests.sh`, `build.json` version 7) first.

1. **The Rust side, one commit.** Delete the export feature
   `ffi-export-src-util-integer-h`, every mention of it in the crate, so that the gateway
   of `src/libgit2/src/util/integer_h.rs` compiles from now on, with its gateway tests.
   Retire the equivalence tests of the six `git__is_*` functions, which after the switch
   would compare the Rust with itself, each one that needs no C carried over to the unit
   tests first; keep those of the six overflow macros, whose C the header keeps for the
   remaining C. The raw bindings of the six functions reach the exports from now on:
   regenerate the bindings (the six `link_name` lines and the six `static_fns.c` wrappers
   go), against the builds in use.
2. **The C side.** In `c/src/util/integer.h`, replace the six `GIT_INLINE` definitions by
   their prototypes and leave the macros. `tests/util/integer.c`'s three cases compile empty
   on this build and test the macros, which stay: it stays. The fork's line is the submodule
   repository's local branch `fork/main` (at `230c9dbdb`): check it out in your `c/`
   worktree, commit the C change on it as `integrate src/util/integer.h`, with the
   campaign's identity, and record the new submodule commit in the superproject. Change no
   git remote and push nothing.
3. **The variants, in a new generation.** Build the five variants from the C that remains
   under `/target/libgit2-crustify/crustify/builds.gen6/<variant>`, with
   `build-variants.sh`, and make your worktree's `crustify/builds` a symlink to that
   directory (it reaches `builds.gen5` now, which you do not touch: three batches of other
   modules build against it).
4. **The gates.** A module's switch, as `testing-gates` has it: the workspace's tests in
   campaign mode over the rebuilt variants, plain and under ASan/UBSan and TSan, Miri per
   test binary and BSan, less the slow list (`crustify/scripts/slow-list.txt`), through
   `end-gates`; the target's remaining C tests through `c-tests.sh`, compared with
   `crustify/artifacts/c-baseline.json`; and packaging the safe crate in bundled mode,
   since `c/` changes. Every end gate passed before the switch: a failure is the switch's.
   `git__is_ssizet`'s export refuses counts past `SSIZE_MAX`, where C's never refused: say
   what in the C tests exercises it.
5. **Records.** A Notes entry in the dashboard on the switch and its gates. Commit this run
   dir's `task.json` and `task.md`. Subject of your Rust-side commit: `integer.h: switch
   from C to Rust (modules/libgit2/src/util/integer.h/integration/run-1)`.

- Three batches of two other modules run beside you, and another campaign shares the host.
  Cap builds and tests at 48 jobs (`CARGO_BUILD_JOBS=48`, `CMAKE_BUILD_PARALLEL_LEVEL=48`,
  `RUST_TEST_THREADS=48`). Keep scratch in this worktree's `crustify/tmp/`, and delete
  scratch builds before you end: the disk is 94% full. End only processes of your own
  worktree. A wait loop over `pgrep -f` matches its own command line and never ends: wait on
  a PID.
- If a step cannot be done as the procedure describes, stop at the last tree that builds
  and passes, commit what holds, and report exactly what blocks the rest.
- The process records of your run are kept by the assistant in the base checkout. The
  dashboard's usage cells are the assistant's.
