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
