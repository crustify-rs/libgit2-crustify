# The bindgen skill's build modes, as the bundling migration met them

Status: `open`.

## Signal

Skills that cannot be followed as written (`evolution` §2), met by the campaign's bundling
migration (`crustify/migrations/bundled-sys`, `6f38b8596` … `e11d5b3ae`, 2026-11-13), which laid
the raw bindings out as `bindgen` (crustify `63b92be`) describes:

1. **Shared mode cannot serve a target built with hidden visibility.** Build modes says shared
   mode builds `c/` "the same way" and ships the libraries through an `$ORIGIN` rpath (lines
   111, 124–129). libgit2's CMake adds `-fvisibility=hidden` (`c/cmake/DefaultCFlags.cmake:131`),
   so its shared library lacks the 42 internal items the bindings reach; the migration's build
   script fails shared mode naming them. The paragraph that handles this (lines 143–148) names
   system mode alone.
2. **"The generated modules' doctests are off" (lines 64–65)** has no direct means once the
   bindings are modules of the safe crate: rustdoc has no per-module switch. The migration
   compiles doctests from a `cfg(doctest)` copy of the bindings whose C doc comments are plain
   comments.
3. **Package contents (lines 198–200)**: "nothing else of the repository" is unreachable, since
   cargo always adds `Cargo.lock`, `Cargo.toml.orig` and `.cargo_vcs_info.json`; the migration
   also packages `README.md`.
4. **Bundled mode's prerequisites**: bundled mode builds `c/` against the system's out-of-tree
   libraries, which a host may lack (this one has no PCRE2 or zlib development files); the
   skill says nothing of how a dependent supplies them (`CMAKE_PREFIX_PATH`/`PKG_CONFIG_PATH`
   here).

## Destination

The spec: `bindgen` (Generation, Build modes, Publishing).

## Change

To be drafted: shared mode joins system mode's rule (it cannot serve once the bindings reach
what the shared library hides, and the build fails naming it); the doctests are compiled from a
copy of the bindings with C comments as plain comments; the package holds the layout, `src/`,
the files of `c/` the bundled build reads, and the files cargo itself adds; bundled mode finds
the out-of-tree libraries as the target's build system does, and says so when one is missing.

## Validation

The migration's build script and gates are the rerun: shared mode fails with the 42 items named,
doctests pass, `cargo package` packages and verifies (650 files), bundled builds with the
side campaigns' installs on the search paths.

## Review

Pending.
