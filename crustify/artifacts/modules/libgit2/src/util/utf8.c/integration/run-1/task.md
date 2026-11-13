# modules/libgit2/src/util/utf8.c/integration/run-1: the module's switch from C

- Repository: the module's integration worktree,
  `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/utf8.c/integration`,
  on the module's branch `crustify/modules/libgit2/src/util/utf8.c`, checked out there.
- This run has no worklist and lands on no wave branch: `task.json` beside this file holds
  its run fields only. It is the hand-off the assistant's role allows for an integration:
  the assistant merged the working branch into the module's branch (`43605086f`, gates
  green, no C changed), and asks you, an agent of the module's batch kind, to carry out the
  module's switch from C to Rust on top of it, in this worktree. Commit on the checked-out
  branch; push nothing, anywhere. The assistant fast-forwards the working branch to your tip
  once you report.

## Instructions

This is the campaign's first switch, so it also makes the one-time change every later
switch relies on. Follow `porting-workflow` (One language per module, Builds),
`versioning` (Repositories, Integration), `testing-gates` (the paragraph on a module's
switch), `setup` (Builds), `bindgen` (Build modes) and the conventions' Exports and Export
features sections; where they leave a choice, make it, and say in your final message what
you chose and why. In outline:

1. **The Rust side.** Delete the export feature `ffi-export-src-util-utf8-c`, so that the
   gateway of `src/libgit2/src/util/utf8.rs` compiles from now on, with its tests. Retire the
   module's equivalence tests, which need the C (`tests/equiv/src/util/utf8.rs`); their
   behaviour stays in its unit tests. The raw bindings of `git_utf8_char_length`,
   `git_utf8_iterate` and `git_utf8_valid_buf_length` stay and now reach the exports.
2. **The C side.** Remove `src/util/utf8.c` from `c/`. `src/util/utf8.h` stays while C
   includes it. The module's one tied C test, `tests/util/utf8.c`, uses only the public
   API (`test_utf8__char_length`), so it stays and checks the exports. Your worktree's `c/`
   is a detached worktree of the base checkout's submodule at `0551dfd4a`; the fork is
   `crustify-rs/libgit2-fork`, whose `main` is at that same commit. The submodule
   repository's local `main` is upstream's (`42a89d7cc`), not the fork's: create a local
   branch `fork/main` at `0551dfd4a`, which stands for the fork's `main`, check it out in
   your `c/` worktree, commit the C change on it as `integrate src/util/utf8.c`, with the
   campaign's identity, and record the new submodule commit in the superproject. Point `.gitmodules`' `url` at
   `https://github.com/crustify-rs/libgit2-fork.git`; do not change any git remote, and do
   not push the fork.
3. **The C build, once.** Change the fork's C build, in CMake's own idiom, so that the C that
   remains, `libgit2.so`, the CLI and the C tests (`libgit2_tests`, `util_tests`), links the
   safe crate's Rust static library, built in campaign mode so that it carries no C of its
   own: no process may hold a module both as C and as Rust, nor its Rust twice. Decide how
   the library is built for each variant (its sanitizer or coverage instrumentation
   included) and how the build is told where it is, and record the commands in
   `crustify/artifacts/build.json` (a new `version`) and the scripts under
   `crustify/scripts/` they need. `wave-229/batch-1`'s Notes entry in the dashboard records
   a scratch relink of `util_tests` and `libgit2_tests` against the Rust exports that passed;
   start from what it learned.
4. **The variants, in a new generation.** Rebuild the five variants (`plain`, `coverage`,
   `asan-ubsan`, `tsan`, `bsan`) from the C that remains, with their static archives, under
   `/target/libgit2-crustify/crustify/builds.gen1/<variant>`, and make your worktree's
   `crustify/builds` a symlink to that directory. Do not touch
   `/target/libgit2-crustify/crustify/builds`: three other agents are building and testing
   against it right now, on branches that still hold `utf8.c` as C. The assistant switches
   the base checkout to the new generation after you report. The side campaigns' installs
   are reached relative to a variant's directory, which a sibling of `builds` keeps valid.
5. **The gates.** A module's switch: every test of the workspace in campaign mode over the
   rebuilt variants, plain and under ASan/UBSan and TSan, and Miri per test binary; the
   target's remaining C tests, `ctest -R '^(offline|util)$'` as `build.json`'s `test`
   command has it, over the link with the Rust static library, on each variant whose
   baseline `crustify/artifacts/c-baseline.json` records, compared with that baseline; and
   packaging the safe crate in bundled mode, since `c/` changed (the procedure is in
   `crustify/evolution/package-gate-procedure`). BSan is rebuilt as a variant and not run:
   it is no regression gate since 2026-11-13 (the settings' `sanitizers` answer). Known
   failures that are not yours: under the nightly variants, 9 `compile_fail` snapshots
   blessed on stable mismatch, and the `CommitBuilder` compile-fail doctest (E0624).
6. **Records.** A Notes entry in the dashboard on the switch and its gates; the module's
   coverage is its Rust's from now on. Commit this run dir's `task.json` and `task.md`.
   Subject of your Rust-side commit: `utf8.c: switch from C to Rust
   (modules/libgit2/src/util/utf8.c/integration/run-1)`.

- libc's wrapper is pinned at `1e60130`, a commit of `/target/glibc-crustify` that is not
  pushed: cargo's git cache holds it on this host, so offline resolution works.
- Three other agents run on this host. Cap builds and tests at 32 jobs
  (`CARGO_BUILD_JOBS=32`, `CMAKE_BUILD_PARALLEL_LEVEL=32`, `RUST_TEST_THREADS=32`). Keep
  scratch in this worktree's `crustify/tmp/`, and delete scratch builds before you end. End
  only processes of your own worktree.
- If a step cannot be done as the skills describe, stop at the last tree that builds and
  passes, commit what holds, and report exactly what blocks the rest: do not leave the
  branch on a tree that does not build.
- Every later switch repeats steps 1, 2, 4 and 5. If you wrote a procedure by hand that a
  script should carry, file it as a proposal with the script, as `evolution` describes.
