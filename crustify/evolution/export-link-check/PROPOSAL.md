# A port batch runs its module's tests with its exports linked in place of its C

Status: `open`.

## Signal

One procedure repeated by hand, which no gate runs. `testing-gates`' Regressions section
has a `port` batch check and lint the safe crate with its module's export feature on,
"which compiles the module's gateway without linking it", and compare each export's
signature with its raw binding. Nothing runs the gateway before the module's switch: every
gate builds with the feature off, and the `testing` skill puts an export's tests "with the
stage that removes the form", which compile only under the feature. A gateway that
compiles, and has C's signature, but writes the wrong out-parameter, reads past its size or
mishandles a NULL, passes every gate until the switch rebuilds the variants, when the C
tests that remain are the first to call it.

## Evidence

- `wave-229/batch-0/run-1` (`src/util/utf8.c`, the crate's first gateway) ran, "as a
  scratch check, not a gate", its tests with the exports linked in place of the archive's
  `utf8.o` (its dashboard notes).
- `wave-229/batch-1/run-1`, its module pass, did it again, by hand: a private build dir under
  `crustify/tmp/` whose entries link to the variant's, but for a copy of its static archive
  without the module's objects (`ar d libcrustify.a utf8.c.o`), named by
  `CRUSTIFY_BUILD_DIR`; then `cargo-variant.sh <variant> test --workspace --features
  ffi-export-src-util-utf8-c` on `plain` and `asan-ubsan`, which passed but for the known
  nightly failures, `nm` showing no `utf8proc_utf8class` and `next_hfs_char` calling Rust.
  It also relinked the variant's C test executables (`libgit2_tests`, `util_tests`) from
  their `build.ninja` edges, without `src/util/CMakeFiles/util.dir/utf8.c.o` and with the
  crate built as a `staticlib` with the feature on: `-sutf8`, and `-spath -sdiff -sindex
  -scheckout -srefs`, gave the stock binaries' results (825 passed, 22 skipped). A
  `util_tests` input list holds two `utf8.c.o`, the module's and its C test's, so removing
  by file name alone would drop the test: the procedure must name the object's path.
- The module pass added the gateway's own `unit_priv` tests, which compile only under the
  feature, and ran them by hand with it on, under Miri too.
- `wave-186/batch-5/run-1` and `wave-186/batch-6/run-1` (`git2/src/cli/opt.c`) ran the
  module's `equiv` tests the same way, by hand, against a copy of the archive without
  `opt.c.o`; those tests compare the C with the Rust, so with the exports linked they compare
  the exports with the native API, and they are retired with the C at the switch.
- `wave-186/batch-7/run-1`, the module pass, found the gateway with no test of its own and two
  defects that only the export's callers reach. Its gateway tests, written after `main.c`,
  `cmd_hash_object.c` and `common.c`, ran with the feature on against the private build and
  under Miri, where the feature also compiles: Miri reported
  `ub_export/args-slot-handed-to-c-through-a-dead-borrow`, a `char **` the export stored for C
  that a later unique reborrow invalidated, which no gate could reach, since every gate builds
  with the feature off. The module's remaining C is an executable, the CLI, which no C test
  runs (`modules.json`'s `tests` is empty, and no suite under `c/tests` reaches `src/cli`), so
  the switch's C-test gate checks nothing of these exports. The pass relinked the variant's
  `git2` from its `build.ninja` edge, without `src/cli/CMakeFiles/git2_cli.dir/opt.c.o` and
  with the crate built as a `staticlib` with the feature on, and ran it and the stock `git2`
  over 66 command lines in scratch repositories: 62 alike, the 4 others the parser's recorded
  departures from the C. The procedure is recorded for this target as a lesson
  (`crustify/LESSONS.md`, "A `git2` module's exports are checked through the CLI").

- `wave-194/batch-4/run-1` (`deps/llhttp/api.c`, its last `port` batch) ran it by hand again,
  as its task message asked, on `plain` and `asan-ubsan`: private build dirs under
  `crustify/tmp/linkcheck/<variant>` linking to the variant's entries, with a copy of
  `libcrustify.a` less `api.c.o` (211 objects), named by `CRUSTIFY_BUILD_DIR`, and the tests
  run with `--features ffi-export-deps-llhttp-api-c`. Two things the procedure must say came
  out of it. First, not every test binary can link: the module's `equiv` binary calls 19 of
  api.c's functions that no remaining C calls and that therefore have no export
  (`llhttp_get_type`, the lenient setters, `llhttp_reset`, …), so it fails to link
  (`undefined symbol`), by design, since it compares with that C; the check runs the other
  binaries and the doctests, and the undefined symbols of `equiv` are the list of the module's
  unexported functions, every one of which should be api.c's own. Second, a test outside
  `equiv` that calls such a function through its raw binding fails to link too, and would
  fail at the switch: `ub_unsafe`'s llhttp fixtures called `llhttp_settings_init`, which the
  batch replaced with the zeroed table it makes. The rest passed (`lib` 858 with the gateway's
  six tests, `ub_safe` 785, `ub_unsafe` 134, `unit_pub` 156, doctests 92 and 387; under
  ASan+UBSan the same but for the known nightly `compile_fail` snapshots and `CommitBuilder`
  doctest), `nm` showing the exports defined and api.c's other symbols absent.

- `wave-194/batch-7/run-1`, api.c's module pass, found that leaving `equiv` out of the check
  is itself a gap. `equiv` is one binary for every module, and `llhttp.c`'s equivalence
  tests (`tests/equiv/deps/llhttp/llhttp.rs`), which stay, also called 10 of api.c's
  unexported functions, so retiring api.c's own equivalence file at the switch would still
  have left `equiv` unlinked
  (`misc/llhttp-equiv-tests-call-api-c-functions-without-exports`). `rust-lld` lists only
  20 undefined symbols by default, so the full list needs its `--error-limit=0`. The
  pass moved those tests into api.c's file, then ran the check over a scratch tree with that
  file retired, as the switch will: `equiv` linked, 532 passing on `plain`, ASan+UBSan and
  TSan. So the check must link `equiv` too, with only the module's own equivalence files
  retired, and fail on any undefined symbol it references from another module's file.

- `wave-197/batch-0/run-1` (`deps/llhttp/http.c`, its whole `port`) ran the check by hand
  again, as its task message asked, on `plain` and `asan-ubsan`, over a scratch tree with its
  own equivalence file retired: every binary linked, `equiv` included (532), and passed but
  for the known nightly failures under ASan+UBSan. Removing the module's object by name would
  have removed the wrong one: the archive holds two `http.c.o` members,
  `src/libgit2/transports/http.c`'s (the 154th) and `deps/llhttp/http.c`'s (the 168th), and
  `ar d libcrustify.a http.c.o` deletes the first; the batch used `ar dN 2` and checked with
  `nm` that the member left defines `git_smart_subtransport_http`. So the script must take
  the module's objects by their `build.ninja` paths, as the `util_tests` evidence above found
  for a link's inputs, and pick an archive member by position, never by its base name alone.
  It also relinked the variants' `git2` and `libgit2_tests` from their `build.ninja` edges,
  without `deps/llhttp/CMakeFiles/llhttp.dir/http.c.o` and with the crate's static library
  built with the feature on, and ran `crustify/scripts/http-gate.sh` over them on `plain`,
  `asan-ubsan` and `coverage`, the module's only C callers being the HTTP client and
  `llhttp.c`, which no offline C test reaches.

- `wave-226/batch-3/run-1` (`src/util/thread.c`'s thread-local storage) ran the check by
  hand, as a scratch run, not a gate: the whole workspace built with
  `ffi-export-src-util-thread-c` on, on `plain`, links the four `git_tlsdata_*` exports in
  place of the archive's `thread.c.o`, whose only definitions they are, so that the member
  is never pulled and no duplicate symbol is reported. Every binary passed (`lib` 914, the
  gateway's own two tests among them, `equiv` 535, `ub_safe` 808, `ub_unsafe` 148,
  `unit_pub` 266, doctests 111 and 400), with `errors.c`'s and `oid.c`'s keys on the Rust
  registry. No C test binary was relinked.

## Destination

The spec: `testing-gates`' Regressions section, the export checks of a `port` batch, with a
script beside it, as `authoring` places a procedure that must run exactly so, documented by
its `--help`: given a variant and the module's objects, as `build.ninja` names them, it
makes the private build dir with an archive lacking them, runs the workspace's tests with
the module's export feature on against it, and, where the target builds C test
executables, relinks them against the crate's static library in place of the module's
objects and runs the suites tied to the module and to the modules that call its exports.

The script also runs the gateway's own tests under Miri with the export feature on, which
Miri's builds allow before the switch since they link no C, and, where the module's remaining
C callers are executables that no C test runs, names that check as the gap it is: an export
whose only C callers are executables is checked by its gateway tests alone, which `testing`'s
UB tests section would have written after those callers, or by a relink of the executables
compared with the stock ones over the inputs the target documents.

## Change

Not made: a script and a new gate want their own review, and the module's integration,
right after this pass, exercises the same link through the target's build system. The
section would say that a `port` batch, and its module pass, run the check on `plain` and
`asan-ubsan`, and that a module's switch still runs the C tests over the actual build.

## Validation

To be run: the script on `wave-229`'s tree, passing, and failing when the gateway's
`bytes()` reads one byte past `str_len` (which the module pass's tests catch under Miri) or
when `git_utf8_iterate` leaves `*out` unwritten on a refusal.

## Review

Pending.
