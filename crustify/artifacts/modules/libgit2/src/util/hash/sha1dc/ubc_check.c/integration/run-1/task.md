# modules/libgit2/src/util/hash/sha1dc/ubc_check.c/integration/run-1: the module's switch from C

- Repository: the module's integration worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/hash/sha1dc/ubc_check.c/integration`, on the module's branch
  `crustify/modules/libgit2/src/util/hash/sha1dc/ubc_check.c`, checked out there.
- This run has no worklist and lands on no wave branch: `task.json` beside this file holds
  its run fields only. The assistant merged the working branch into the module's branch
  (`494db0118`, format, clippy and the plain workspace's 3226 tests green, no C of the
  module's changed), and asks you, an agent of the module's batch kind, to carry out the
  module's switch from C to Rust on top of it, in this worktree. Commit on the checked-out
  branch; push nothing, anywhere. The assistant fast-forwards the working branch to your
  tip once you report.

## Instructions

This is the campaign's sixth switch. `src/util/integer.h`'s, an hour ago
(`crustify/artifacts/modules/libgit2/src/util/integer.h/integration/run-1/task.md`, its
Notes entry in the dashboard, `b95941fa6`), and `deps/llhttp/http.c`'s, the last that
removed a file, are the precedents. The module pass's final message lists what this switch
deletes and what its gates should run: it is in the dashboard's Notes entry of
`wave-221/batch-3` and in `crustify/evolution/export-link-check`. Read
`crustify/LESSONS.md` and the scripts under `crustify/scripts/` (`build-variants.sh`,
`rust-staticlib.sh`, `c-tests.sh`, `build.json` version 7) first.

The order of the parts, for every switch (a review of
`crustify/evolution/switch-header-change-before-bindings` asked for one order): the C side
committed on the fork first; then the Rust side in one commit, its bindings regenerated from
your checkout's `c/` at that commit against the builds in use, and the submodule recorded
at it; then the builds; then the gates.

1. **The C side.** Remove `src/util/hash/sha1dc/ubc_check.c` from `c/`; `ubc_check.h`
   stays for `sha1.c`, and the directory's glob needs no CMake change. No C test is tied to
   the module. The fork's line is the submodule repository's local branch `fork/main` (at
   `d9a1d699b`, integer.h's switch): check it out in your `c/` worktree, commit the change
   on it as `integrate src/util/hash/sha1dc/ubc_check.c`, with the campaign's identity.
   Change no git remote and push nothing.
2. **The Rust side, one commit.** Delete the export feature
   `ffi-export-src-util-hash-sha1dc-ubc-check-c`, every mention of it: in `Cargo.toml`
   the feature line, its comment and the comment listing the switched modules; in
   `src/libgit2/src/util/hash/sha1dc/ubc_check.rs` the `cfg(not(feature))` `SHA1_DVS`
   static with its doc, anchor and alias, the `cfg` on the re-export, which keeps the alias
   and description, the `cfg` on `mod ffi_export`, the module doc's "compiled only under
   the export feature" paragraph and `DvInfo`'s "the table `ubc_check.c` defines". Retire
   the module's equivalence tests, `tests/equiv/src/util/hash/sha1dc/ubc_check.rs` with the
   `mod` files `hash.rs` and `sha1dc.rs` and `mod hash;` in `tests/equiv/src/util.rs`:
   all four need the C, and the module pass's two `unit_pub` tests keep what they checked.
   The allowlist's `ubc_check`, `sha1_dvs` and `dv_info_t` stay and reach the exports.
   Record the submodule at the fork's new commit.
3. **The variants, in a new generation.** Build the five variants from the C that remains
   under `/target/libgit2-crustify/crustify/builds.gen7/<variant>`, with
   `build-variants.sh`, and make your worktree's `crustify/builds` a symlink to that
   directory (it reaches `builds.gen6` now, which you do not touch). Check with `nm` that
   `ubc_check` and `sha1_dvs` come from the Rust in every variant.
4. **The gates.** A module's switch, as `testing-gates` has it: the workspace's tests in
   campaign mode over the rebuilt variants, plain and under ASan/UBSan and TSan, Miri per
   test binary and BSan, less the slow list (`crustify/scripts/slow-list.txt`), through
   `end-gates`; the target's remaining C tests through `c-tests.sh` on every variant,
   compared with `crustify/artifacts/c-baseline.json`, `util_tests -ssha1` above all,
   whose `detect_collision_attack` is the one case in which a recompression finds a
   collision; and packaging the safe crate in bundled mode, since `c/` changes. Every end
   gate passed before the switch: a failure is the switch's.
5. **One measurement, for the user.** The module pass measured `ubc_check` at about 3.3
   times C's `-O2` cost per call in the Rust static library the remaining C links, which
   `rust-staticlib.sh` builds on the dev profile. Measure what the switch costs a C
   consumer on `plain`: time hashing a large buffer through libgit2's SHA-1 (for example
   `git_odb_hash` over 256 MB, or `util_tests -ssha1` repeated) on gen6 and on gen7, and
   say what profile and flags the library was built with. Change no build setting.
6. **Records.** A Notes entry in the dashboard on the switch and its gates. Commit this run
   dir's `task.json` and `task.md`. Subject of your Rust-side commit: `ubc_check.c: switch
   from C to Rust (modules/libgit2/src/util/hash/sha1dc/ubc_check.c/integration/run-1)`.

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
