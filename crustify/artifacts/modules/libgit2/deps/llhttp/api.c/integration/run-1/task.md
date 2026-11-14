# modules/libgit2/deps/llhttp/api.c/integration/run-1: the module's switch from C

- Repository: the module's integration worktree,
  `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/deps/llhttp/api.c/integration`, on the module's branch `crustify/modules/libgit2/deps/llhttp/api.c`, checked out there.
- This run has no worklist and lands on no wave branch: `task.json` beside this file holds
  its run fields only. The assistant merged the working branch into the module's branch
  (`a4335ab7a`, gates green, no C of the module's changed), and asks you, an agent of the
  module's batch kind, to carry out the module's switch from C to Rust on top of it, in this
  worktree. Commit on the checked-out branch; push nothing, anywhere. The assistant
  fast-forwards the working branch to your tip once you report.

## Instructions

This is the campaign's third switch. `src/util/utf8.c`'s (`012d1d34c`) made the one-time
change and `src/cli/opt.c`'s (`f68ea8660`) followed its procedure: read both Notes entries
in the dashboard, the lessons in `crustify/LESSONS.md` and
`crustify/evolution/module-switch-procedure` with the evidence opt.c's switch added, and
follow it with the scripts under `crustify/scripts/` (`build-variants.sh`,
`rust-staticlib.sh`, `c-tests.sh`, `build.json` version 7).

1. **The Rust side.** Delete the export feature `ffi-export-deps-llhttp-api-c`, so that the
   gateway of `src/libgit2/deps/llhttp/api.rs` compiles from now on, with its gateway tests.
   Retire the module's equivalence tests, `tests/equiv/deps/llhttp/api.rs` and its `mod`
   line: the module pass (`245185caa`) moved there every test that calls an `api.c`
   function with no export, and mapped each of the 21 to the unit tests that keep its
   behaviour (its `misc` report under
   `crustify/reviews/modules/libgit2/deps/llhttp/api.c/wave-194/batch-7/`); carry over any
   that call no C, as opt.c's switch did. The raw bindings of exported functions stay and
   reach the exports; those of functions with no export go with their allowlist entries.
   The 23 dispatchers have no header declaration and no raw binding.
2. **The C side.** Remove `deps/llhttp/api.c` from `c/`; `deps/llhttp/llhttp.h` stays, the
   header `llhttp.c`, `http.c` and libgit2's HTTP client include. The module has no C tests
   tied to it. The fork's line is the submodule repository's local branch `fork/main` (at
   `f57e1a616`, opt.c's switch): check it out in your `c/` worktree, commit the C change on
   it as `integrate deps/llhttp/api.c`, with the campaign's identity, and record the new
   submodule commit in the superproject. Change no git remote and push nothing.
3. **The variants, in a new generation.** Build the five variants from the C that remains
   under `/target/libgit2-crustify/crustify/builds.gen3/<variant>`, with
   `build-variants.sh`, and make your worktree's `crustify/builds` a symlink to that
   directory (it links to `builds.gen2` now, which you do not touch).
4. **The gates.** A module's switch, as the two before ran them: the workspace's tests in
   campaign mode over the rebuilt variants, plain and under ASan/UBSan and TSan, and Miri
   per test binary; the target's remaining C tests through `c-tests.sh`, compared with
   `c-baseline.json`; packaging the safe crate in bundled mode (the manifest also needs the
   workspace's Git-source patch for ffibox, as opt.c's switch found). No offline C test
   drives the HTTP client, and the online suites are disabled in the baseline: say what, if
   anything on this host, exercises the switched exports through libgit2's own C
   (`httpparser.c`, `httpclient.c` and `llhttp.c`'s generated parser calling the 23
   dispatchers), and run it against the new generation and the previous one if something
   does, such as a clone or fetch over HTTP from a server on localhost with the variant's
   `git2`. BSan is rebuilt as a variant and not run. Known failures that are not yours:
   under the nightly variants, 9 `compile_fail` snapshots blessed on stable mismatch, and
   the `CommitBuilder` compile-fail doctest (E0624).
5. **Records.** A Notes entry in the dashboard on the switch and its gates. Commit this run
   dir's `task.json` and `task.md`. Subject of your Rust-side commit: `api.c: switch from C
   to Rust (modules/libgit2/deps/llhttp/api.c/integration/run-1)`.

- libc's wrapper is pinned at `24da93f`, a commit of `/target/glibc-crustify` that is not
  pushed: cargo's git cache holds it on this host, so offline resolution works.
- No other agent of the campaign runs now. Cap builds and tests at 64 jobs
  (`CARGO_BUILD_JOBS=64`, `CMAKE_BUILD_PARALLEL_LEVEL=64`, `RUST_TEST_THREADS=64`); another
  campaign shares the host. Keep scratch in this worktree's `crustify/tmp/`, and delete
  scratch builds before you end. End only processes of your own worktree.
- A wait loop over `pgrep -f` matches its own command line and never ends: wait on a PID.
- If a step cannot be done as the procedure describes, stop at the last tree that builds
  and passes, commit what holds, and report exactly what blocks the rest.
- Add what this switch shows the procedure lacks to
  `crustify/evolution/module-switch-procedure`, as evidence.
