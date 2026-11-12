# fixes/static-campaign-link/run-1

- Repository: your worktree, `/target/libgit2-crustify/crustify/.worktrees/fixes/static-campaign-link`,
  checked out on `crustify/fixes/static-campaign-link`, forked from the working branch
  (`crustify/devel` at `5d0e779f8`).
- Run dir: `crustify/artifacts/fixes/static-campaign-link/run-1/`, holding this file and
  `task.json`.
- Land by committing on `crustify/fixes/static-campaign-link` in your worktree. No wave
  branch or landing lock is involved: no other batch is running, and the assistant
  fast-forwards the working branch to your branch once you report.

## Departures from your role, named so you keep them

This is not a scheduled batch. It has no wave and no items (`task.json`'s `wave`, `batch`
and `module` are null, `items` is empty). It changes the raw bindings' build, not a
module's Rust. The user decided it. Your translation steps (analysis, binding items,
`Replaces` anchors, coverage of a worklist) do not apply. Your gates do, scaled to a
change that every build of the workspace goes through, as described below.

## The problem

libgit2 compiles with `-fvisibility=hidden` (`c/cmake/DefaultCFlags.cmake:131`), so
`libgit2.so` exports only the `GIT_EXTERN` API. Campaign mode links each variant's
`libgit2.so` dynamically (`libgit2-sys/build.rs`, `campaign_library`). So no internal
function links from Rust. In a port campaign, internal functions are most of what the
batches bind. Waves 169, 176 and 12 all hit this. Their proposals are on their wave
branches and are context only:

- `refs/heads/crustify/waves/wave-169:crustify/evolution/internal-symbols-unlinkable/PROPOSAL.md`
  has a probe: the hidden symbols link from the variant's objects.
- `refs/heads/crustify/waves/wave-176:crustify/evolution/unexported-lifecycle-primitives/PROPOSAL.md`.
- `refs/heads/crustify/waves/wave-12:crustify/evolution/hidden-lifecycle-primitives/PROPOSAL.md`.

The user chose to link each variant's C statically in campaign mode. Crustify `20500ce`
writes the rule:

- `setup`, Builds: each variant archives its library's objects by the last step of its
  `build` command, and records the archive as the variant's `static`.
- `bindgen`, Build modes: campaign mode links the `static` archive, with the libraries the
  shared library links. The remaining C resolves the exports within the same link. Only the
  out-of-tree installs take an rpath.
- `templates/build.json`.

Read those sections first. The target's own configuration stays as it is: no visibility
flag changes, no `GIT_EXTERN` edits, no C change at all. The C tests and the CLI keep
linking `libgit2.so`.

## What to do

1. **Archive step.** Write a script under `crustify/scripts/` that archives, in a variant's
   build dir, exactly the objects that `libgit2.so` links into one static archive
   (`libgit2.a`, or another name that cannot collide with a CMake output). That includes
   the `deps/` object libraries libgit2 bundles (reftable, llhttp, ntlmclient, xdiff and
   the others). It excludes the clar tests, the CLI and anything else. Take the object list
   from the build system, for example the `libgit2.so` link edge in `build.ninja` or
   `ninja -t inputs`, not from a hand-kept list. Append the script as the last step of
   every variant's `build` command in `crustify/artifacts/build.json`. Record each
   variant's `static`, raise `version`, and update its `_comment`. Run the script on the
   five existing variants (`crustify/builds/{plain,coverage,asan-ubsan,tsan,bsan}`, shared
   with the base checkout through a symlink). Their objects are current, so nothing needs
   recompiling. Do not reconfigure or rebuild a variant.
2. **`libgit2-sys/build.rs`, campaign mode.**
   - When the variant records `static`, link that archive statically, together with the
     system and out-of-tree libraries that `libgit2.so` itself links. Take those from the
     build, for example `libgit2.pc`'s `Libs.private` or the link edge, not hard-coded.
   - Drop the dylib link and the rpath to the variant dir in that case. Keep the rpath to
     the out-of-tree installs.
   - Mind link order: the archive's C references the Rust exports a ported module will
     define, and the Rust references the C. Group or order as needed, and say what you
     chose.
   - The same applies to `CRUSTIFY_BUILD_DIR` private builds: a private build carries the
     archive the same script makes.
   - Keep the shared-library path for a variant that records no `static`, as the spec
     says.
   - The wrappers' installs are found today at `<root>/../<wrapper>`. That does not resolve
     from a worktree like yours, where the build silently skips them. A static link needs
     them at link time, so find them robustly, for example from the variant's
     `CMakeCache.txt` or the repository's common dir. Check that a worktree build links the
     side campaigns' OpenSSL, PCRE2 and zlib, not the system's.
   - Update the module doc comment.
3. **Everything that assumed the shared library.** Adapt it:
   - the root `build.rs` (`DEP_GIT2_BUILD_DIR`, `DEP_GIT2_DEP_LIB_DIRS`, and the TSan
     `--export-dynamic-symbol=__tsan_*` for `libgit2.so`'s libcrypto atomics);
   - `crustify/scripts/cargo-variant.sh`;
   - `crustify/scripts/coverage/`: C coverage is now in the test binary's own objects, not
     in `libgit2.so`;
   - trybuild and probe crates;
   - `LD_LIBRARY_PATH` uses;
   - anything else `git grep` finds.

   Leave the Rust that today works around hidden symbols as it is: for example
   `alloc.rs`'s strategies over `git_odb_backend_data_alloc`, `failalloc.rs`'s reproduced
   bodies, the `tests/equiv` rebuilds of `git_strarray_copy` and `git_oidarray__reverse`,
   and `errors.rs`'s copy of `git_error_save`. Instead, list in your report each one that
   could now bind its C directly. Later batches take that list.
4. **Proof that it lifts the block.** Add a minimal test that calls one hidden function
   through a raw binding: allowlist `reftable_buf_init`/`reftable_buf_release`, or
   `git_failalloc_malloc`, with `wrapper.h` gaining the internal header, as `bindgen`'s
   Extending section describes. Regenerate the bindings and run the test plain, under
   ASan/UBSan and under TSan.
5. **Gates.** The change touches every build, so:
   - check, format (`cargo +nightly-2026-10-08 fmt --all --check`) and clippy the
     workspace, warnings denied;
   - run the workspace's whole test suite on `plain` and on `asan-ubsan`;
   - build and run a smoke subset on `tsan`, on `bsan` (`cargo bsan`) and on `coverage`,
     and show that a coverage run still reports C line coverage for a libgit2 file;
   - package the safe crate in bundled mode, as `testing-gates` describes;
   - compare failures with the base: the stale trybuild snapshot
     `checkout_notify_callback_not_send.stderr` already fails on the base (toolchain 1.99
     wording), so re-bless it if your toolchain is the gate's;
   - sanitizer runtimes are the likeliest trouble: statically linked clang-instrumented
     objects in a rustc-instrumented binary, and the coverage variant's
     `/usr/lib/llvm-23` profile runtime against rustc's. If a variant cannot link its C
     statically, keep it on the shared path (no `static` recorded), and report why.
6. **Records.** Fill this run's records as your role describes, without wave cells. File
   any evolution proposal the run's evidence supports, under the settings' `evolve`
   answer. Spec changes you judge needed beyond `20500ce` go in a proposal, since that
   change is the assistant's.

## Host

Nothing else runs. Take the host's jobs as the `concurrency` skill describes, and end
only processes of your own worktree.
