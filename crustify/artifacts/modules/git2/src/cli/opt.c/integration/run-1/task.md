# modules/git2/src/cli/opt.c/integration/run-1: the module's switch from C

- Repository: the module's integration worktree,
  `/target/libgit2-crustify/crustify/.worktrees/modules/git2/src/cli/opt.c/integration`, on
  the module's branch `crustify/modules/git2/src/cli/opt.c`, checked out there.
- This run has no worklist and lands on no wave branch: `task.json` beside this file holds
  its run fields only. The assistant merged the working branch into the module's branch
  (`6d823f4d7`, gates green, no C changed), and asks you, an agent of the module's batch
  kind, to carry out the module's switch from C to Rust on top of it, in this worktree.
  Commit on the checked-out branch; push nothing, anywhere. The assistant fast-forwards the
  working branch to your tip once you report.

## Instructions

This is the campaign's second switch, and the first of the `git2` link unit (the CLI).
`src/util/utf8.c`'s switch (`012d1d34c`, `a8d41d84f`) made the one-time change: read its
Notes entry in the dashboard, its lesson in `crustify/LESSONS.md` and
`crustify/evolution/module-switch-procedure`, and follow that procedure with the scripts it
left (`crustify/scripts/build-variants.sh`, `rust-staticlib.sh`, `c-tests.sh`,
`build.json` version 7). Where the CLI's link unit needs what the library's did not, make
the change and say what it was.

1. **The Rust side.** Delete the export feature `ffi-export-src-cli-opt-c`, so that the
   gateway of `src/git2/src/cli/opt.rs` compiles from now on, with its six gateway tests.
   Retire the module's equivalence tests, which need the C (`tests/equiv/src/cli/opt.rs`);
   say which behaviour they established no unit test keeps, if any. The raw bindings of the
   exported functions stay and reach the exports; those of functions with no export go with
   their allowlist entries.
2. **The C side.** Remove `src/cli/opt.c` from `c/`; `src/cli/opt.h` stays while the CLI's C
   includes it. The module has no C tests tied to it. The fork's line is the submodule
   repository's local branch `fork/main` (at `1b37e3d08`, utf8.c's switch): check it out in
   your `c/` worktree, commit the C change on it as `integrate src/cli/opt.c`, with the
   campaign's identity, and record the new submodule commit in the superproject. Change no
   git remote and push nothing.
3. **The variants, in a new generation.** Build the five variants from the C that remains
   under `/target/libgit2-crustify/crustify/builds.gen2/<variant>`, with
   `build-variants.sh`, and make your worktree's `crustify/builds` a symlink to that
   directory (it links to `builds.gen1` now). Do not touch `crustify/builds.gen1`: another
   agent is building and testing against it, on a branch that still holds `opt.c` as C.
4. **The gates.** A module's switch, as utf8.c's ran them: the workspace's tests in campaign
   mode over the rebuilt variants, plain and under ASan/UBSan and TSan, and Miri per test
   binary; the target's remaining C tests through `c-tests.sh`, compared with
   `c-baseline.json`; packaging the safe crate in bundled mode. The C tests reach none of
   the CLI: the check of this switch's exports is the relinked-CLI comparison the module
   pass recorded in its run dir
   (`crustify/artifacts/modules/git2/src/cli/opt.c/wave-186/batch-7/run-1/`, 66 command
   lines against the stock `git2`, 62 matching and 4 recorded departures). Run it against
   the new generation's `git2`, which now links the exports for real, and the previous
   generation's, and report the comparison. BSan is rebuilt as a variant and not run. Known
   failures that are not yours: under the nightly variants, 9 `compile_fail` snapshots
   blessed on stable mismatch, and the `CommitBuilder` compile-fail doctest (E0624).
5. **Records.** A Notes entry in the dashboard on the switch and its gates. Commit this run
   dir's `task.json` and `task.md`. Subject of your Rust-side commit: `opt.c: switch from C
   to Rust (modules/git2/src/cli/opt.c/integration/run-1)`.

- libc's wrapper is pinned at `24da93f`, a commit of `/target/glibc-crustify` that is not
  pushed: cargo's git cache holds it on this host, so offline resolution works.
- One other agent runs on this host. Cap builds and tests at 48 jobs
  (`CARGO_BUILD_JOBS=48`, `CMAKE_BUILD_PARALLEL_LEVEL=48`, `RUST_TEST_THREADS=48`). Keep
  scratch in this worktree's `crustify/tmp/`, and delete scratch builds before you end. End
  only processes of your own worktree.
- If a step cannot be done as the procedure describes, stop at the last tree that builds
  and passes, commit what holds, and report exactly what blocks the rest: do not leave the
  branch on a tree that does not build.
- Add what this switch shows the procedure lacks to
  `crustify/evolution/module-switch-procedure`, as evidence.
