# The packaging gate's procedure, for a crate whose dependencies are unpublished

Status: `open`.

## Signal

A procedure worked out by hand that a skill should state: `testing-gates`' Regressions
section says that a batch that changes the raw bindings packages the safe crate, "built
alone in bundled mode from the C it packages, with no build variant selected", and that
"each crate the package depends on is patched to its local source", but not how. The
direct form fails, and the steps that pass were found by trial.

## Evidence

`wave-229/batch-0/run-1` (`src/util/utf8.c`, which allowlists three functions), on base
`2216b0cd0`, the first packaging gate since the tree's change of 2026-11-13:

1. `CRUSTIFY_BUILD_VARIANT= cargo package --offline --allow-dirty` stops before verifying:
   `no matching package named glibc-bindings found` (crates.io index). The packaged
   manifest drops the Git source of `glibc-bindings` and the path of `ffibox`, keeping
   their versions, and the workspace's `[patch]` tables, keyed by those sources, do not
   apply to it.
2. `cargo package --offline --allow-dirty --no-verify --exclude-lockfile` packages (697
   files). Extracted under `crustify/tmp/`, the package's manifest needs an empty
   `[workspace]`, since the worktree's own encloses it, and
   `[patch.crates-io] glibc-bindings = { path = "/target/glibc-crustify" }` and
   `ffibox = { path = "/crustify-rs/ffibox" }`.
3. `CRUSTIFY_BUILD_VARIANT= cargo build --offline` there then fails in CMake: PCRE2, then
   zlib, not found. The host has neither: the bundled build finds them only through the
   side campaigns' `plain` installs, `OPENSSL_ROOT_DIR` and `CMAKE_PREFIX_PATH`, the
   latter `:`-separated in the environment where `build.json`'s `-D` form takes `;`.
   With `OPENSSL_ROOT_DIR=<openssl install>` and
   `CMAKE_PREFIX_PATH=<openssl install>:<pcre2 install>:<zlib install>`, the package
   builds alone: cmake builds `libgit2.a` and `libcrustify_git2.a` into `OUT_DIR`.

`modules/git2/src/cli/opt.c/integration/run-1` (the switch of `src/cli/opt.c`, on
`f68ea8660`), the next packaging gate, followed steps 2 and 3 and failed to compile the
package's `lib`: two copies of ffibox, `0.1.1` by path for the crate and `0.1.0` by Git
(`40621a2`) for `glibc-bindings`, whose manifest takes ffibox by Git. The workspace patches
that Git source too (`[patch."https://github.com/crustify-rs/ffibox"]`), and step 2's
`[patch.crates-io]` does not stand for it. `opt.rs`'s gateway, which compiles from this switch
on, is the first code of the crate to pass a `glibc_bindings` `FileRef` through the crate's
own ffibox traits (`FileRef::from_ptr`, E0599 four times), so `utf8.c`'s gate could not have
met it. With the workspace's Git patch copied into the package's manifest as well, and the
lockfile the first attempt wrote removed, so that the patch applies, the graph holds one
ffibox and the package builds alone (696 files). Its `lib` tests: 829 pass and 58 fail, the
58 needing `c/tests/resources`, as at `utf8.c`'s gate. So the script copies every `[patch]`
table of the workspace, the Git sources' as well as crates.io's, and resolves with no
lockfile.

## Destination

The spec: `testing-gates`' Regressions section, with a script beside it, as `authoring`
places a procedure that must run exactly so, documented by its `--help`: it packages
without verifying, patches each unpublished dependency to the source the workspace's
`[patch]` names for it, gives the extracted package a workspace of its own, puts the
out-of-tree libraries' installs of the `plain` variant on the build system's search paths,
and builds it with no variant, in a scratch dir it removes.

## Change

Not made: the gate is not a translator's to change. The section would name the script,
and say that the patches and search paths stand in for what a dependent's host supplies.

## Validation

The run above, step by step: step 1 fails as quoted, steps 2 and 3 pass. A script is
validated by passing on that tree and failing, naming the library, without the search
paths.
