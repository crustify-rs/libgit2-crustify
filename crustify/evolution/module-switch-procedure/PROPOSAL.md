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

The second switch, `src/cli/opt.c`'s (`modules/git2/src/cli/opt.c/integration/run-1`), the
first of the `git2` link unit, the CLI, ran items 1 to 4's scripts unchanged and met five
more gaps:

5. **A link unit no C test reaches has its switch gate in a module pass's run dir.** The
   CLI's exports are checked by running the relinked `git2` beside the previous one (the
   `git2`-exports lesson), but the comparison is `cli-smoke.py`, a scratch script recorded in
   `wave-186/batch-7/run-1`'s artifacts, which writes to a fixed `crustify/tmp/cli` and whose
   scratch commits take the clock's time, so that two runs' outputs differ in every line that
   prints a commit's date or id.
6. **Retiring a module's equivalence tests by file loses those that run no C.**
   `porting-workflow` retires them "which need the C", and keeps their behaviour in unit
   tests, but an equivalence file also holds tests that only read the raw bindings'
   constants, which stay with the header: two of `opt.c`'s did, and no unit test kept what
   they established.
7. **A review's reproducer that enables the deleted export feature no longer resolves.**
   Two reproducers under `crustify/reviews/modules/git2/src/cli/opt.c/wave-186/` name
   `ffi-export-src-cli-opt-c` in their manifests; nothing says whether a switch updates them
   or leaves them reproducing at the commit their report names.
8. **`build-variants.sh` takes no job count.** It builds every variant side by side, each
   `cmake --build` and `cargo` at the environment's job count, so a host cap holds only if
   the caller divides it by hand.
9. **The C library's link pulls Rust it never calls.** With `opt.rs`'s gateway compiled,
   `libgit2.so`, which calls only `utf8.c`'s exports, holds 6337 local Rust functions where
   gen1's held 2608, and imports about 75 more glibc functions: the linker takes whole
   archive members of the dev-profile `staticlib`, and does not collect unused sections.
   Its exported API does not change.

The third switch, `deps/llhttp/api.c`'s (`modules/libgit2/deps/llhttp/api.c/integration/run-1`),
ran items 1 to 4's scripts unchanged and met four more gaps:

10. **The switch's Rust side precedes the generation.** It is the first switch that removes
    raw bindings: 23 functions without an export, with their allowlist entries. Each
    variant's `build` runs `rust-staticlib.sh`, which builds the crate of the checkout with
    `--features bindgen` in campaign mode, checking the committed bindings against the
    variant's headers, and compiles the gateway only once the export feature is gone. So the
    switch's Rust side, the feature deleted, the allowlist pruned and the bindings
    regenerated, comes before `build-variants.sh`, not after the C's removal as the
    procedure's order (the C change, then the variants, then the gates) suggests. The
    bindings are regenerated against the previous generation, whose headers the switch does
    not change.
11. **A module whose exports only an online suite reaches has no C gate.** `c-tests.sh` runs
    the offline suites the baseline records, and no offline C test drives libgit2's HTTP
    client (the module pass's finding): `httpparser.c`, `httpclient.c` and `llhttp.c`'s
    generated parser call the 31 exports only in a fetch, clone or push over HTTP, whose
    suites (`online::*`) the baseline disables, since they need the network. libgit2's own CI
    runs part of them offline, against servers on localhost named by `GITTEST_REMOTE_URL`
    (`ci/test.sh`'s `auth_clone_and_push`, `-sonline::clone::push -sonline::push`).
12. **The equivalence tests that call no C are marked already.** Item 6's step needs each
    retiring test's calls checked. A test that calls C carries `#[cfg_attr(miri, ignore =
    "calls C")]` (the Miri gate's marker), so the tests that lack it are the candidates, a
    mechanical check; here all 21 carry it, and none is carried over.
13. **A CMake file may name the removed source in a property.** `deps/llhttp/CMakeLists.txt`
    globs its sources, as the lesson has it, and also sets
    `set_source_files_properties(api.c http.c llhttp.c PROPERTIES COMPILE_FLAGS …)`. CMake
    ignores a property of a source no target lists, so the file was left unchanged and every
    variant configured and built; the dangling name stays until the last of the three goes.

The fourth switch, `deps/llhttp/http.c`'s (`modules/libgit2/deps/llhttp/http.c/integration/run-1`),
llhttp's second, ran items 1 to 4's scripts and item 11's unchanged but for item 16, and met four
more gaps:

14. **The export feature is named outside the gateway's `cfg`.** The switch deletes "the
    module's export feature", which reads as the `Cargo.toml` entry and the gateway's
    `#[cfg(feature = …)]`. http.c's `headers_code`, which only the gateway calls, also carried
    `#[cfg_attr(not(any(test, feature = "ffi-export-deps-llhttp-http-c")), expect(dead_code))]`:
    with the feature gone, the unknown `cfg` and the expectation left unmet fail clippy with
    warnings denied. The module pass foresaw it, so the switch did not fail; nothing in the
    procedure would have. The step is mechanical: every mention of the feature's name in the
    crate (`src/`, `tests/`, `Cargo.toml`, `build.rs`) is removed or rewritten, a `cfg_attr`
    or `cfg(any(…))` that names it as much as the gateway's attribute, so that only the run
    records and the reviews' reproducers (item 7) still name it.
15. **An archive's member names are not unique.** `static-archive.sh` archives the objects
    `libgit2.so` links by their file names, and `libcrustify.a` held two `http.c.o`, llhttp's
    and the HTTP transport's (`src/libgit2/transports/http.c`). The count the earlier switches
    compared (210 to 209) and the diff of `ar t` (one `http.c.o` fewer) do not say which left:
    `nm -A` over the archive does, the member left defining `git_smart_subtransport_http` and
    no `llhttp_*` symbol. The module pass's link check had to remove the member by position
    (`ar dN 2`). The archive's check would be unambiguous by object path, as `build.ninja`
    names them (`deps/llhttp/CMakeFiles/llhttp.dir/http.c.o`), or by the symbols the removed
    TU defined, which the switch checks anyway.
16. **The HTTP gate did not see keep-alive.** `http-gate.sh`'s `results.txt` tallied the
    requests each remote answered by method, framing and status, which do not change when the
    client stops reusing its connections: `llhttp_should_keep_alive`, the export of this
    switch that libgit2's client calls, could have returned `0` always, and every line would
    have matched. The run made `http-remote.py` log each request's place on its connection
    (`conn=<n>`) and `http-gate.sh` tally the connections the client opened to each remote by
    framing (`connections <side> <framing> count=<n>`), lines added after the request lines,
    which are unchanged.
17. **The variants' smoke test is gone.** The earlier switches smoke-tested each variant with
    `util_tests -sutf8`, which has found no suite since `git_utf8_char_length`'s
    `port/unexport` (wave 232) carried its C test over, gen4 included. Nothing names the smoke
    test, so each switch picks one; a suite of a module that stays C the longest, or
    `build.json`'s own, would keep it from leaving with a module.

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

Where `src/cli/opt.c`'s switch went (its commits on the module's branch, the fork's
`f57e1a616`, and its dashboard Notes entry, `git2/src/cli/opt.c` integration):

- Items 1 to 4: `build-variants.sh` built `crustify/builds.gen2` from `f57e1a616`, five
  variants in about 15 minutes, each archive holding 211 objects where gen1's held 212,
  `opt.c.o` gone, and each `git2` defining the five `cli_opt_*` exports as global Rust
  symbols (gen1's were the C's local ones). The CLI's link unit needed no change of its own:
  the first switch's `SelectRust.cmake` already links the library into the CLI, which takes
  libgit2's objects statically, and the crate's default features include `git2`, so
  `rust-staticlib.sh`'s library already carries the CLI's exports once its feature is gone.
  `c-tests.sh` ran unchanged.
- Item 5: the switch ran `cli-smoke.py` from the module pass's run dir, the stock `git2` being
  gen1's and the relinked one gen2's, on plain, ASan/UBSan and TSan: 62 of 66 the same on
  each, the 4 others the module's recorded departures, and no sanitizer report. Gen2's
  outcomes equal the module pass's relinked ones on all 66, but for three (`cat-file -p HEAD`,
  `cat-file commit HEAD`, `blame f1`) that differ only in the scratch commit's date and id.
  A tracked script beside `c-tests.sh`, taking its output dir and the command lines' file,
  with `GIT_AUTHOR_DATE` and `GIT_COMMITTER_DATE` fixed, would make each run's record
  comparable with the last.
- Item 6: `usage_flags_match_the_c_constants` and `parse_flags_match_the_c_constants`, which
  compared `Usage`'s and `ParseFlags`' bits with `opt.h`'s `CLI_OPT_USAGE_*` and
  `CLI_OPT_PARSE_*` through the raw bindings, moved to `tests/unit_pub/src/cli/opt.rs`; the
  CLI's spec tables, which stay C, set those bits. The switch's step would read: before an
  equivalence file is retired, each of its tests that calls no C is carried over to the
  module's unit tests.
- Item 7: the two reproducers were left as they are, reproducing at the commit their reports
  name.
- Item 8: the switch split the host's cap of 48 by hand, `CARGO_BUILD_JOBS=10` and
  `CMAKE_BUILD_PARALLEL_LEVEL=10` for the five variants.
- Item 9: `nm` of each generation's plain `libgit2.so.1.9.0`: 5878 and 9607 local and global
  text symbols, 2608 and 6337 of them Rust's (`_R`), `cli_opt_parse` in neither; the dynamic
  symbol tables differ only in undefined glibc imports. `-Wl,--gc-sections` on the link that
  takes the Rust library, or a library built with one codegen unit per export, would keep it
  to what the C calls; not tried.

Where `deps/llhttp/api.c`'s switch went (its commits on the module's branch, the fork's
`c54c0c180`, and its dashboard Notes entry, `libgit2/deps/llhttp/api.c` integration):

- Items 1 to 4: `build-variants.sh` built `crustify/builds.gen3` from `c54c0c180`, five
  variants in about 7 minutes at 12 jobs each (item 8's split again, for the 64 the run was
  capped at), each archive holding 210 objects where gen2's held 211, `api.c.o` gone. In
  every variant, `libgit2.so`, `git2` and `libgit2_tests` define the 31 exports (`addr2line`
  puts `llhttp_execute` at `api.rs:1634`) and none of api.c's 23 unexported functions; each
  passed its smoke test (`util_tests -sutf8`). `libgit2.so` defines the same 907 dynamic
  symbols as gen2's, and holds 6747 local Rust functions where gen2's held 6337 (item 9).
  `c-tests.sh` ran unchanged.
- Item 10: the run deleted the feature, pruned `sys/libgit2/allowlist.txt` of the 23
  functions and regenerated `bindings.rs` (`CRUSTIFY_BINDINGS_UPDATE=1 cargo-variant.sh plain
  build --features bindgen --lib`, against gen2), committed that, and only then committed the
  C change and ran `build-variants.sh`, whose `rust-staticlib.sh` then checked the committed
  bindings against gen3's headers on every variant.
- Item 11: the run added `crustify/scripts/http-remote.py`, `git http-backend` behind a
  persistent HTTP/1.1 front that frames each response by its path (`Content-Length`,
  chunked in 1, 7, 4096 and 65536-byte chunks, chunked with chunk extensions, or
  close-delimited), with an optional Basic challenge, and `crustify/scripts/http-gate.sh`,
  which runs a variant's `git2 clone` of two repositories through each framing, three
  clones that fail, and `libgit2_tests -sonline::push -sonline::clone::credentials` with
  `GITTEST_REMOTE_URL` at the guarded remote, into a `results.txt` free of times, ports and
  paths. Gen2's and gen3's results are compared with `diff` (the dashboard entry).
- Item 12: `grep -c 'ignore = "calls C"'` over `tests/equiv/deps/llhttp/api.rs` gives 21,
  its `#[test]` count; the module pass's map (`wave-194/batch-7`'s `misc` report) names the
  unit tests that keep each.
- Item 13: `deps/llhttp/CMakeLists.txt` unchanged; gen3's five `configure`s and builds
  passed with it.
- Item 7 again: `wave-194/batch-7/equiv/execute-export-moves-an-empty-runs-error-pos`'s
  reproducer names `ffi-export-deps-llhttp-api-c`, and is left reproducing at the commit its
  report names.

Where `deps/llhttp/http.c`'s switch went (its commits on the module's branch, the fork's
`230c9dbdb`, and its dashboard Notes entry, `libgit2/deps/llhttp/http.c` integration):

- Items 1 to 4: `build-variants.sh` built `crustify/builds.gen5` from `230c9dbdb`, five
  variants in about 6 minutes at 12 jobs each (item 8's split, for the run's cap of 64), each
  archive holding 209 objects where gen4's held 210. In every variant, `libgit2.so`, `git2` and
  `libgit2_tests` define the four exports (`addr2line` puts `llhttp_should_keep_alive` at
  `http.rs:545`) and no `llhttp_message_needs_eof`; each passed `util_tests -sstr` and
  `git2 --version` (item 17). `c-tests.sh` ran unchanged.
- Item 10 again: the run deleted the feature, pruned the allowlist of
  `llhttp_message_needs_eof` and regenerated `bindings.rs` against gen4, committed that with the
  submodule at the fork's new commit, and only then ran `build-variants.sh`.
- Item 11 again: `http-gate.sh` over gen4's C and gen5's Rust, on plain, ASan+UBSan and TSan,
  gives the same `results.txt`, and on gen5's coverage variant the run calls all four exports.
- Item 12 again: all six of `tests/equiv/deps/llhttp/http.rs`'s tests carry the `calls C`
  marker; the module pass's `equiv-retirement.md` maps each to the unit tests that keep it.
- Item 13 again: `deps/llhttp/CMakeLists.txt`'s property names `api.c` and `http.c`, both
  gone, and `llhttp.c`; unchanged, and gen5's five `configure`s and builds passed with it.
- Item 14: `grep -rn ffi-export-deps-llhttp-http-c` over `src`, `tests`, `Cargo.toml` and
  `build.rs` found the `Cargo.toml` entry, the gateway's `cfg`, `headers_code`'s `cfg_attr` and
  the module's doc comment; after the switch it finds none there.
- Item 15: `ar t` of gen4's and gen5's plain `libcrustify.a` differ in one line, an `http.c.o`;
  `nm -A` of gen5's shows its one `http.c.o` defining `git_smart_subtransport_http`, where
  gen4's second also defined the three hooks and both predicates.
- Item 16: four runs over gen4 (plain twice, ASan+UBSan, TSan) gave the same connection
  tally, 9 connections to the open remote for its 15 requests and 73 to the guarded one for
  its 610, and the same request lines as api.c's record; gen5's equal them. A mutant of
  `llhttp_should_keep_alive` was not built, so that the tally catches one rests on the client
  reading the export once per response (`httpclient.c`'s `on_headers_complete`).

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
- `porting-workflow`, One language per module (item 6): an equivalence test that calls no C
  is carried over to the module's unit tests at the switch, not retired with the file.
- `testing-gates`, Regressions (item 5): a link unit that no C test reaches is checked at its
  switch by its relinked executables beside the previous generation's, by a tracked script
  beside `c-tests.sh`, and the result compared with the module pass's record.
- `reviewing` (item 7): whether a switch updates the reproducers that name its export
  feature.
- `build-variants.sh` (item 8): a job count, divided among the variants it builds.
- `bindgen`, Build modes, or the fork's `SelectRust.cmake` (item 9): the C library's link
  collects the Rust it does not call.
- `porting-workflow`, Builds (item 10): the switch's Rust side, the bindings regenerated,
  lands before the generation is built, since every variant's build checks them.
- `testing-gates`, Regressions (item 11): a switch whose exports only the baseline's disabled
  online suites reach runs those the target's own CI runs against local servers, against a
  local remote, by the scripts beside `c-tests.sh`, on the new generation and the previous
  one, compared; `http-remote.py` and `http-gate.sh` generalized from libgit2's suites and
  variables into arguments.
- `porting-workflow`, One language per module (item 12): the carry-over candidates are the
  retiring tests without the Miri `calls C` marker.
- The `libgit2` CMake lesson (item 13): a property naming the removed file is left.
- `porting-workflow`, Builds (item 14): deleting the export feature removes every mention of
  its name in the crate, not only the gateway's `cfg`.
- `testing-gates`, Regressions (item 15): a switch checks its archive by the symbols the
  removed TU defined, or by object path, not by member names.
- `http-gate.sh` (item 16): connections are tallied beside requests, so that a keep-alive
  departure shows.
- `setup`, Builds, or `build.json` (item 17): the variants' smoke test is named once, from
  C that stays.

## Change

Made in the campaign only: the scripts under `crustify/scripts/`, `build.rs`'s campaign
mode and `build.json` version 7, and, from `deps/llhttp/api.c`'s switch, `http-remote.py`
and `http-gate.sh` beside `c-tests.sh`. The skills are not changed by this run: the change spans
`setup`, `bindgen`, `testing-builds` and `testing-gates`, which are not the translator's to
evolve, and wants its own review.

## Validation

This run: `build-variants.sh` built the five variants of `crustify/builds.gen1` from the
fork's `1b37e3d08`, each smoke-tested (`util_tests -sutf8`); the workspace's gates ran over
them and `c-tests.sh` compared four variants with the baseline, as the dashboard entry
records. A later switch validates the generalized scripts by running them.

`src/cli/opt.c`'s switch ran `build-variants.sh`, `rust-staticlib.sh` and `c-tests.sh`
unchanged, for a second link unit, and its gates over `crustify/builds.gen2` are its
dashboard entry's. Items 5 to 9 are its evidence, and are not made: `porting-workflow` is the
translator's to evolve, but item 6 wants review with the rest of this proposal.

`deps/llhttp/api.c`'s switch ran the three scripts unchanged again, for a third module, the
first whose exports libgit2 itself calls only over the network. Its gates over
`crustify/builds.gen3` are its dashboard entry's. Item 11's two scripts are made, in the
campaign only (`crustify/scripts/http-remote.py` and `http-gate.sh`), and validated by that
run: gen2's C and gen3's Rust give the same `results.txt` on plain, ASan+UBSan and TSan, and on
gen3's coverage variant the run calls 24 of the 31 exports. A mutant of an export was not
built, so whether the comparison catches a departure rests on the coverage alone. Items 10,
12 and 13 are evidence, and are not made: they want review with the rest of this proposal.

`deps/llhttp/http.c`'s switch ran the scripts again, for a fourth module, and changed one:
item 16's connection tally, in `http-remote.py`'s log and `http-gate.sh`'s `results.txt`, in
the campaign only, validated by four identical runs over gen4 and gen5's equal ones. Items 14,
15 and 17 are evidence, and are not made.

## Review

Pending.
