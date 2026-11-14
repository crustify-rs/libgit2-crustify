# modules/libgit2/deps/llhttp/http.c/integration/run-1: the module's switch from C

- Repository: the module's integration worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/deps/llhttp/http.c/integration`, on the module's branch
  `crustify/modules/libgit2/deps/llhttp/http.c`, checked out there.
- This run has no worklist and lands on no wave branch: `task.json` beside this file holds
  its run fields only. The assistant merged the working branch into the module's branch
  (`b478971b2`, gates green, no C of the module's changed), and asks you, an agent of the
  module's batch kind, to carry out the module's switch from C to Rust on top of it, in this
  worktree. Commit on the checked-out branch; push nothing, anywhere. The assistant
  fast-forwards the working branch to your tip once you report.

## Instructions

This is the campaign's fourth switch, and the second of llhttp: `deps/llhttp/api.c`'s
(`21c5c5f47`) is the closest precedent. Read the three switches' Notes entries in the
dashboard, the lessons in `crustify/LESSONS.md` and
`crustify/evolution/module-switch-procedure` with its thirteen items of evidence, and
follow it with the scripts under `crustify/scripts/` (`build-variants.sh`,
`rust-staticlib.sh`, `c-tests.sh`, `http-gate.sh`, `http-remote.py`, `build.json`
version 7).

1. **The Rust side.** Delete the export feature `ffi-export-deps-llhttp-http-c`, so that the
   gateway of `src/libgit2/deps/llhttp/http.rs` compiles from now on, with its gateway
   tests. The module pass (`f1ffed455`) warns that `headers_code`'s
   `cfg_attr(not(any(test, feature = "ffi-export-deps-llhttp-http-c")), expect(dead_code))`
   names the feature: remove it with the feature's other `cfg`s, or clippy with warnings
   denied fails. Retire the module's equivalence tests, `tests/equiv/deps/llhttp/http.rs`
   and its `mod` line; the module pass's `equiv-retirement.md`, in its run dir, maps each of
   the six to the unit tests that keep it. The raw binding of `llhttp_should_keep_alive`
   stays and reaches the export; those of functions with no export
   (`llhttp_message_needs_eof`) go with their allowlist entries. The three hooks have no
   header declaration and no raw binding.
2. **The C side.** Remove `deps/llhttp/http.c` from `c/`; `deps/llhttp/llhttp.h` stays. The
   module has no C tests tied to it. The fork's line is the submodule repository's local
   branch `fork/main` (at `9859ed5e7`, `git_utf8_char_length`'s unexport): check it out in your
   `c/` worktree, commit the C change on it as `integrate deps/llhttp/http.c`, with the
   campaign's identity, and record the new submodule commit in the superproject. Change no
   git remote and push nothing.
3. **The variants, in a new generation.** Build the five variants from the C that remains
   under `/target/libgit2-crustify/crustify/builds.gen5/<variant>`, with
   `build-variants.sh`, and make your worktree's `crustify/builds` a symlink to that
   directory (it links to `builds.gen4` now, which you do not touch). The Rust side lands
   before the generation is built, the variants' build checking the bindings. The archive
   held two members named `http.c.o`, llhttp's and the HTTP transport's
   (`src/libgit2/transports/http.c`): check with `nm` that the one that left is llhttp's.
4. **The gates.** A module's switch, as the three before ran them: the workspace's tests in
   campaign mode over the rebuilt variants, plain and under ASan/UBSan and TSan, and Miri
   per test binary; the target's remaining C tests through `c-tests.sh`, compared with
   `c-baseline.json` (lowered by one case at wave 232: plain 3442 of 3497); packaging the
   safe crate in bundled mode; and `http-gate.sh`, the localhost HTTP check api.c's switch
   added, on the new generation and the previous one, since `llhttp.c`'s generated parser
   calls the three hooks on every response. BSan is rebuilt as a variant and not run. Known
   failures that are not yours: under the nightly variants, 9 `compile_fail` snapshots
   blessed on stable mismatch, and the `CommitBuilder` compile-fail doctest (E0624).
5. **Records.** A Notes entry in the dashboard on the switch and its gates. Commit this run
   dir's `task.json` and `task.md`. Subject of your Rust-side commit: `http.c: switch from
   C to Rust (modules/libgit2/deps/llhttp/http.c/integration/run-1)`.

- libc's wrapper is pinned at `24da93f`, a commit of `/target/glibc-crustify` that is not
  pushed: cargo's git cache holds it on this host, so offline resolution works.
- No other agent of the campaign runs now; another campaign shares the host. Cap builds
  and tests at 64 jobs (`CARGO_BUILD_JOBS=64`, `CMAKE_BUILD_PARALLEL_LEVEL=64`,
  `RUST_TEST_THREADS=64`). Keep scratch in this worktree's `crustify/tmp/`, and delete
  scratch builds before you end. End only processes of your own worktree. A wait loop over
  `pgrep -f` matches its own command line and never ends: wait on a PID.
- If a step cannot be done as the procedure describes, stop at the last tree that builds
  and passes, commit what holds, and report exactly what blocks the rest.
- Add what this switch shows the procedure lacks to
  `crustify/evolution/module-switch-procedure`, as evidence.
