# Campaign mode builds the Rust library the remaining C links, once per variant

Status: `open`. Split from `module-switch-procedure` on 2026-11-14. No change is made yet.

Goal: Interoperability. Its measures are the remaining C's tests passing over the Rust link,
and the build modes and variants the crate builds and tests in.

## Signal

"Skills that contradict each other, or an agent prompt that leads its agent wrong": a defect
in the spec. `porting-workflow` asks for a library "built in campaign mode", and `bindgen`'s
campaign mode cannot build it as written (item 2). `testing-builds` asks for a library with
each variant's instrumentation, and BorrowSanitizer has no command for one (item 3).

## Evidence

Met at the campaign's first switch from C,
`modules/libgit2/src/util/utf8.c/integration/run-1`. The run's commits are `012d1d34c` (the
switch) and the fork's `1b37e3d08`; its dashboard Notes entry is `libgit2/src/util/utf8.c`
integration.

**Campaign mode cannot build the Rust library the C links (item 2).** The C consumers link
"the safe crate's Rust static library, built in campaign mode so that it carries no C of its
own", but campaign mode (`bindgen`, Build modes) requires the variant's artifacts and static
archive, which exist only once the C has linked, and the C's link needs the Rust library
first. Once the C links it, the link edges campaign mode reads list it among their
libraries, so the Rust tests would link the Rust twice, once from their rlib and once from
the library.

- Where the run went: `build.rs`'s campaign mode takes `CRUSTIFY_STATICLIB`, under which it
  reads the variant's configured build (`build.ninja`, `CMakeCache.txt` and `libgit2.pc`, all
  written by `configure`) and links none of it, and every other campaign build leaves out of
  the link the library the build's cache names (`LIBGIT2_RUST_LIBRARY`), matched by canonical
  path, since CMake names it relative to the build dir and the cache by its physical path.
  `crustify/scripts/rust-staticlib.sh` runs as the first step of each variant's `build`,
  between `configure` and `cmake --build`. The library carries only the Rust, its three
  exports and the crate's own `__extern` wrappers (`nm`): campaign mode links the variant's
  archive unbundled (`static:-bundle`), so a `staticlib` leaves it out.
- `src/cli/opt.c`'s switch (`modules/git2/src/cli/opt.c/integration/run-1`, the fork's
  `f57e1a616`), the first of the `git2` link unit, the CLI: each `git2` of
  `crustify/builds.gen2` defines the five `cli_opt_*` exports as global Rust symbols (gen1's
  were the C's local ones). The CLI's link unit needed no change of its own: the first
  switch's `SelectRust.cmake` already links the library into the CLI, which takes libgit2's
  objects statically, and the crate's default features include `git2`, so
  `rust-staticlib.sh`'s library already carries the CLI's exports once its feature is gone.
- `deps/llhttp/api.c`'s switch (`modules/libgit2/deps/llhttp/api.c/integration/run-1`, the
  fork's `c54c0c180`): in every variant of `crustify/builds.gen3`, `libgit2.so`, `git2` and
  `libgit2_tests` define the 31 exports (`addr2line` puts `llhttp_execute` at `api.rs:1634`)
  and none of api.c's 23 unexported functions.
- `deps/llhttp/http.c`'s switch (`modules/libgit2/deps/llhttp/http.c/integration/run-1`, the
  fork's `230c9dbdb`): in every variant of `crustify/builds.gen5`, `libgit2.so`, `git2` and
  `libgit2_tests` define the four exports (`addr2line` puts `llhttp_should_keep_alive` at
  `http.rs:545`) and no `llhttp_message_needs_eof`.
- From item 10: `rust-staticlib.sh` builds the crate of the checkout with `--features
  bindgen` in campaign mode, checking the committed bindings against the variant's headers,
  and compiles the gateway only once the export feature is gone.

**BorrowSanitizer has no `rustc` subcommand (item 3)**, and `cargo bsan build` cannot
override a crate's type, so the instrumented `staticlib` the `bsan` variant's C links has no
direct command; its driver also bundles BSan's C runtime into the library, which the C's own
link already links.

- Where the run went: a stand-in for the cargo that cargo-bsan runs (`CARGO`) turns its
  `build` into `rustc --crate-type staticlib`, every other call passing through, and the
  bundled runtime's 90 members are removed from the copy written; the `bsan` variant's
  `libgit2.so` then holds no `__bsan_*` definition, and its `util_tests -sutf8` passes.

## Destination

From the original's Destination list:

- `bindgen`, Build modes: the `staticlib` form of campaign mode, and that campaign mode
  leaves the library the C links out of its own links.
- `testing-builds`, Builds: each variant's C consumers link a library built with its
  instrumentation, by `rust-staticlib.sh`, including BSan's form above.

Protected: `testing-builds` (`skills/verification/testing/builds/SKILL.md`, under
`skills/verification/**`). Not protected: `bindgen`.

The original places each script "beside the skills that name them". It does not say whether
`rust-staticlib.sh` sits beside `bindgen` or beside `testing-builds`. Beside `testing-builds`
it is on a protected path.

The step order (`configure`, then the Rust library, then the C's link) is
`porting-workflow`'s, and the proposal `switch-step-order` takes it.

## Change

Not made. In the campaign only, `crustify/scripts/rust-staticlib.sh`, `build.rs`'s campaign
mode and `build.json` version 7 exist.

The change would be:

1. `bindgen`, Build modes, describes a second form of campaign mode, selected by an
   environment variable. In this form the build script reads the variant's configured build
   and links none of its C. The crate is then built as a static library that holds only the
   Rust.
2. `bindgen`, Build modes, states that when a variant's C links the Rust library, every
   other campaign build leaves that library out of its own link. The library is matched by
   canonical path. The reason is that a Rust test would otherwise link the Rust twice.
3. `testing-builds`, Builds, states that each variant's `build` command builds the library
   with that variant's instrumentation, as its first step after `configure`. Under
   BorrowSanitizer the library is built through a stand-in for the cargo its driver runs,
   and the runtime the driver bundles is removed from the library, because the C's own link
   already links it.
4. `rust-staticlib.sh` moves beside the skill that names it. These libgit2-specific names
   become arguments or are read from `build.json`: the CMake cache variable that names the
   library (`LIBGIT2_RUST_LIBRARY`), the crate's package name (`libgit2-crustify`) and the
   library file cargo writes (`liblibgit2_crustify.a`), and the side campaign whose install
   the BSan build adds to `PKG_CONFIG_PATH` (`pcre2-bindings`). The host triple
   (`x86_64-unknown-linux-gnu`) and the list of variants are also written into the script
   today.

## Validation

Copied from the original:

- First switch: `build-variants.sh` built the five variants of `crustify/builds.gen1` from
  the fork's `1b37e3d08`, each smoke-tested (`util_tests -sutf8`); the workspace's gates ran
  over them and `c-tests.sh` compared four variants with the baseline, as the dashboard entry
  records. A later switch validates the generalized scripts by running them.
- `src/cli/opt.c`'s switch ran `build-variants.sh`, `rust-staticlib.sh` and `c-tests.sh`
  unchanged, for a second link unit, and its gates over `crustify/builds.gen2` are its
  dashboard entry's.
- `deps/llhttp/api.c`'s switch ran the three scripts unchanged again, for a third module, the
  first whose exports libgit2 itself calls only over the network. Its gates over
  `crustify/builds.gen3` are its dashboard entry's.
- `deps/llhttp/http.c`'s switch ran the scripts again, for a fourth module.

## Review

None yet.
