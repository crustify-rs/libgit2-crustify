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

The `git2` link unit's scaffold (`f9a02dc65`, `44e95b76d`, 2026-11-13), the CLI executable, met
four more:

5. **"An empty allowlist generates nothing" (Generation)**: bindgen still writes its one-line
   banner, and the update path deleted an empty `static_fns.c` that Layout lists as committed;
   the build script now writes it empty.
6. **An executable link unit in bundled mode**: no target of the build system builds an
   executable's objects without linking it; the build script runs the compile commands the
   build system records for the CLI's TUs (`compile_commands.json`) and archives them, which is
   not "built by the target's own build system" as Build modes says.
7. **An executable link unit in shared and system modes**: neither mode says what to do with a
   link unit no library holds; the build script refuses its feature, naming the modes that serve it.
8. **A type `external.txt` re-exports must be bound in the owning link unit**: the CLI's headers
   name `git_str`, which `sys::libgit2` did not bind, so the scaffold added it to libgit2's
   allowlist; Extending says a batch extends its own link unit's input and nothing else, which
   leaves this cross-unit case unsaid.

## Destination

The spec: `bindgen` (Generation, Build modes, Publishing).

## Change

crustify `81ece02` and `437159f` (bindgen; setup's Builds): shared mode, like system
mode, cannot serve once the bindings reach what a shared library hides, and fails naming it; an
executable link unit's objects are compiled and archived by bundled mode, taken from the
variant's one archive by campaign mode, and refused by shared and system modes, with the
consequence of its `main` and how its unit is ported meanwhile; doctests come from an `OUT_DIR`
copy of the bindings with plain comments, which `src/sys.rs` includes under `cfg(doctest)`; an
empty allowlist leaves the banner and an empty `static_fns.c`; a type another link unit owns
joins `external.txt`, the owner's allowlist and a re-export in `src/sys.rs`; bundled mode finds
the out-of-tree libraries through the build system's own search paths; the package holds the
C's license files, the README and what cargo adds. Setup's Builds archives every link unit's
objects, a library's or an executable's, once each.

## Validation

The migration's build script and gates are the rerun: shared mode fails with the 42 items named,
doctests pass, `cargo package` packages and verifies (650 files), bundled builds with the
side campaigns' installs on the search paths.

## Review

review-1 (`crustify/artifacts/evolution/bindgen-modes-after-bundling/review-1`), of crustify `ce6445d`: **rejected** on five fixes: the doctest copy's place
(OUT_DIR, from whichever bindings the build uses, included by `src/sys.rs`), the empty
allowlist's claim (both generated files, not every Layout file), the executable's `main` stated by
its consequence and handling, the cross-unit exception moved out of the numbered steps with the
`src/sys.rs` re-export, and the C's license files in the package. All eight items had evidence;
five reran as stated; nothing weakens a check. Fixed in `437159f`, on `81ece02` (`ce6445d`
rebased onto `7e35c25`). review-2 pending.

