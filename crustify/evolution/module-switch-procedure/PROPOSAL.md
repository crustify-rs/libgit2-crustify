# A module's switch: variant generations, the C's Rust library, and the C test gate

Status: `open`.

## Signal

Procedures worked out by hand that the skills should state, met at the campaign's first
switch from C (`modules/libgit2/src/util/utf8.c/integration/run-1`), which every later switch
repeats, and two gaps where following the skills as written fails:

1. **The variants cannot be rebuilt in place.** `porting-workflow`'s Builds section rebuilds
   the shared variants "from the C that remains once the integration lands", and
   `testing-builds` has them prebuilt and immutable. Each variant's `configure` takes the C
   from the base checkout (`cmake -S ../../../c`), whose `c/` the batches still running hold
   with the module as C: three agents were building and testing against
   `crustify/builds` while this switch ran. A build configured from the integration
   worktree's `c/` instead is bound to a tree that is purged once the module integrates,
   and its build files then name sources that no longer exist.
2. **Campaign mode cannot build the Rust library the C links.** The C consumers link "the
   safe crate's Rust static library, built in campaign mode so that it carries no C of its
   own", but campaign mode (`bindgen`, Build modes) requires the variant's artifacts and
   static archive, which exist only once the C has linked, and the C's link needs the Rust
   library first. Once the C links it, the link edges campaign mode reads list it among
   their libraries, so the Rust tests would link the Rust twice, once from their rlib and once
   from the library.
3. **BorrowSanitizer has no `rustc` subcommand**, and `cargo bsan build` cannot override a
   crate's type, so the instrumented `staticlib` the `bsan` variant's C links has no direct
   command; its driver also bundles BSan's C runtime into the library, which the C's own link
   already links.
4. **The C test gate was scratch.** The baseline's per-suite runner,
   `crustify/tmp/scripts/c-suites.sh` in the base checkout, is untracked and writes its
   results into the variant's dir; a switch's gate compares with that baseline on every
   variant, at every switch.

## Evidence

The run's commits `012d1d34c` (the switch), and the fork's `1b37e3d08`; its dashboard Notes
entry, `libgit2/src/util/utf8.c` integration. Where the run went:

- Item 1: the variants were rebuilt as `crustify/builds.gen1`, a directory beside
  `crustify/builds`, which the integration worktree's `crustify/builds` links to, each
  configured from `crustify/builds.gen1/c`, a detached worktree of the fork at the commit the
  generation was built from (`crustify/scripts/build-variants.sh`). The side campaigns'
  installs, reached as `$PWD/../../../..`, resolve the same from a sibling of `builds`.
- Item 2: `build.rs`'s campaign mode takes `CRUSTIFY_STATICLIB`, under which it reads the
  variant's configured build (`build.ninja`, `CMakeCache.txt` and `libgit2.pc`, all written
  by `configure`) and links none of it, and every other campaign build leaves out of the link
  the library the build's cache names (`LIBGIT2_RUST_LIBRARY`), matched by canonical path,
  since CMake names it relative to the build dir and the cache by its physical path.
  `crustify/scripts/rust-staticlib.sh` runs as the first step of each variant's `build`,
  between `configure` and `cmake --build`. The library carries only the Rust, its three
  exports and the crate's own `__extern` wrappers (`nm`): campaign mode links the variant's
  archive unbundled (`static:-bundle`), so a `staticlib` leaves it out.
- Item 3: a stand-in for the cargo that cargo-bsan runs (`CARGO`) turns its `build` into
  `rustc --crate-type staticlib`, every other call passing through, and the bundled runtime's
  90 members are removed from the copy written; the `bsan` variant's `libgit2.so` then holds
  no `__bsan_*` definition, and its `util_tests -sutf8` passes.
- Item 4: `crustify/scripts/c-tests.sh` runs `build.json`'s `test` command over a copy of the
  variant's `CTestTestfile.cmake` tree, and every suite alone as the baseline did, into an
  output dir of its own, and compares the tally with `c-baseline.json`'s record.

## Destination

The spec, where `authoring` places each part, with the scripts beside the skills that name
them, generalized from libgit2's names (`libgit2_tests`, `util_tests`, the
`LIBGIT2_RUST_LIBRARY` variable, the `offline` and `util` targets) into arguments:

- `setup`, Builds, and `testing-builds`, Builds: the variants are built in generations, each
  from a worktree of the fork of its own, and a checkout's `crustify/builds` links to one; a
  switch builds the next generation, and the assistant moves the base checkout to it once
  the switch lands, so that the batches in flight keep theirs. `build-variants.sh` beside
  `setup`.
- `bindgen`, Build modes: the `staticlib` form of campaign mode, and that campaign mode leaves
  the library the C links out of its own links.
- `testing-builds`, Builds: each variant's C consumers link a library built with its
  instrumentation, by `rust-staticlib.sh`, including BSan's form above.
- `testing-gates`, Regressions: a switch's C tests run by `c-tests.sh` per variant the
  baseline records, compared with it.
- `porting-workflow`, Builds: the order a switch's steps take, the C build's `configure`
  before the Rust library and its link after it.

## Change

Made in the campaign only: the scripts under `crustify/scripts/`, `build.rs`'s campaign
mode and `build.json` version 7. The skills are not changed by this run: the change spans
`setup`, `bindgen`, `testing-builds` and `testing-gates`, which are not the translator's to
evolve, and wants its own review.

## Validation

This run: `build-variants.sh` built the five variants of `crustify/builds.gen1` from the
fork's `1b37e3d08`, each smoke-tested (`util_tests -sutf8`); the workspace's gates ran over
them and `c-tests.sh` compared four variants with the baseline, as the dashboard entry
records. A later switch validates the generalized scripts by running them.

## Review

Pending.
