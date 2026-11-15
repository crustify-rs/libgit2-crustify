# modules/libgit2/src/util/thread.c/integration/run-1: the module's switch from C

- Repository: the module's integration worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/thread.c/integration`, on the module's branch
  `crustify/modules/libgit2/src/util/thread.c`, checked out there.
- This run has no worklist and lands on no wave branch: `task.json` beside this file holds
  its run fields only. The assistant merged the working branch into the module's branch
  (`bf9833a37`, format, clippy and the plain workspace's 3353 tests green, no C of the
  module's changed), and asks you, an agent of the module's batch kind, to carry out the
  module's switch from C to Rust on top of it, in this worktree, as `porting-workflow`'s
  Builds section orders it. Commit on the checked-out branch; push nothing, anywhere. The
  assistant fast-forwards the working branch to your tip once you report.

## Instructions

This is the campaign's seventh switch and its widest: `thread.c` leaves, twelve
`static inline` definitions in two headers become prototypes, and every remaining C file
reaches libgit2's reference counts, error state and runtime initialisation through the
module's exports. `src/util/integer.h`'s switch (prototypes in a header, `b95941fa6`) and
`src/util/hash/sha1dc/ubc_check.c`'s (a file removed, `f42cb56c4`) are the precedents:
read their Notes entries in the dashboard and their task messages under
`crustify/artifacts/modules/libgit2/src/util/`. The switch was rehearsed three times: by
`wave-226/batch-6` on `plain` (`switch-rehearsal.md` in its run dir, with the twelve
prototypes) and by the module pass, `wave-226/batch-7`, under ASan+UBSan and TSan
(`equiv-retirement.md` and `fault-seeding.md` in its run dir). Read `crustify/LESSONS.md`,
the new `switch-rehearsal-c-is-a-git-checkout` among its lessons, and the scripts under
`crustify/scripts/` first.

1. **The C side, committed on the fork.** Remove `src/util/thread.c` from `c/`, and in
   `src/util/thread.h` and `src/util/unix/pthread.h` replace the twelve definitions (the
   eleven atomics and `git_threads_global_init`) by batch 6's prototypes; the branch for
   builds without threads, the macros and the lock and thread aliases stay. No tied C test
   leaves: each only calls the module or reads a layout the header keeps. The fork's line
   is the submodule repository's local branch `fork/main` (at `e717996da`): check it out
   in your `c/` worktree, commit the change on it as `integrate src/util/thread.c`, with
   the campaign's identity. Change no git remote and push nothing.
2. **The Rust side, one commit,** which records the submodule at the fork's new commit.
   Delete the export feature `ffi-export-src-util-thread-c`, every mention of it: the
   `Cargo.toml` entry, the gateway's `cfg`, the four
   `cfg_attr(not(any(test, feature = …)), expect(dead_code))` on `CDestructor::new`,
   `CWord::new`, `CWord::as_ptr` and `mod c_keys`, and the module docs that say "until the
   module's switch" or "compile only under the … feature". Retire
   `tests/equiv/src/util/thread.rs`, `thread/atomics.rs`, `thread/tlsdata.rs` and the
   gateway's `equiv_priv`, as `equiv-retirement.md` maps them, carrying over first any that
   needs no C. Regenerate the bindings from your checkout's `c/` at the fork's commit,
   against the builds in use: the twelve `__extern` link names and their `static_fns.c`
   wrappers go, and the twelve allowlist entries stay. Revert the feature flag batch 6 added
   to `crustify/scripts/rust-staticlib.sh` for its rehearsal, if the merged tree holds it.
3. **The variants, in a new generation.** Build the five variants from the C that remains
   under `/target/libgit2-crustify/crustify/builds.gen8/<variant>`, with
   `build-variants.sh`, and make your worktree's `crustify/builds` a symlink to that
   directory (it reaches `builds.gen7` now, which you do not touch). Check with `nm` that
   the twelve functions and the four `git_tlsdata_*` come from the Rust in every variant.
4. **The gates.** A module's switch, with the settings' departure of 2026-11-15, the
   user's: BSan runs over the workspace only at a ship point, so run `end-gates` without
   its BSan stage (format, lints, plain, ASan+UBSan, TSan, Miri per test binary, less the
   slow list), and BSan only on tests you add or change, each named and alone. The bsan
   variant is still built. Then the target's remaining C tests through `c-tests.sh` on
   every variant, compared with `crustify/artifacts/c-baseline.json`, the threading suites
   above all (`threads::*`, `object::cache`, `pack::threadsafety`, `core::init`), with
   `c/` a real git checkout; and packaging the safe crate in bundled mode. Every end gate
   passed before the switch: a failure is the switch's. The gateway's own tests ran under no
   gate until now, since they compile only with the feature: say that they pass on every
   stage.
5. **Two measurements, for the user.** On `plain`, gen7 against gen8: the cost of an
   increment-and-decrement pair as the remaining C runs it (batch 6's `bench/`), and one
   whole-library workload, such as `libgit2_tests`' offline suites' wall and CPU time, or a
   clone and a revwalk of a large local repository through `git2`. Say what profile the
   Rust library was built on. The user plans a release profile or cross-language LTO later:
   change no build setting.
6. **Records.** A Notes entry in the dashboard on the switch and its gates. Commit this run
   dir's `task.json` and `task.md`. Subject of your Rust-side commit: `thread.c: switch
   from C to Rust (modules/libgit2/src/util/thread.c/integration/run-1)`.

- No other agent of the campaign runs now; another campaign shares the host. Cap builds
  and tests at 48 jobs (`CARGO_BUILD_JOBS=48`, `CMAKE_BUILD_PARALLEL_LEVEL=48`,
  `RUST_TEST_THREADS=48`). Keep scratch in this worktree's `crustify/tmp/`, and delete
  scratch builds before you end: the disk is 94% full. End only processes of your own
  worktree. A wait loop over `pgrep -f` matches its own command line and never ends: wait on
  a PID.
- If a step cannot be done as the procedure describes, stop at the last tree that builds
  and passes, commit what holds, and report exactly what blocks the rest.
- The process records of your run are kept by the assistant in the base checkout. The
  dashboard's usage cells and the slow list are the assistant's: name any test an end gate
  measured over a minute in your final message.
