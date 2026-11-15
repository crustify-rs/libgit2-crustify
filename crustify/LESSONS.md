# Lessons

Rules that hold for this campaign's target alone, each with the evidence it rests on and
its scope, as the `evolution` skill's Lessons section describes. Every run reads them
before its first step.

## Before `git_libgit2_init`, probe libgit2's allocator only through a release

**Rule.** Before `git_libgit2_init`, `git__allocator` is the failing family
(`src/util/allocators/failalloc.c`), and a test may reach it only through a release:
`git_odb_backend_data_free`, which is `git__free`, sets no error. Any allocation then
fails, and `git__malloc`, `git__realloc` and every error setter report through
`git_error_set_oom` or `set_error`, whose thread-state lookup reads TLS key 0 before
`git_error_global_init` has created libgit2's key: it may return another component's
slot and write `&oom_error` into it. The same holds for an allocator installed with
`set_allocator` before initialisation that fails a request.

**Evidence.** `git_odb_backend_data_alloc`'s finding in the ownership store (ASan
unknown-crash in `git_error_set_oom`, `errors.c:170`); `src/util/errors.c`'s
`threadstate_get`; the equivalence test of `wave-176/batch-0`
(`tests/equiv/src/util/allocators/failalloc.rs`), which compares the C release with the
Rust family's and leaves `malloc` and `realloc` uncompared for this reason. The family's
routines called directly, as `git_failalloc_malloc` through its raw binding, set no error
and are outside the rule: `wave-176/batch-1` compares them so in the test process.

**Scope.** The tests of `src/util/allocators/*.c`, `src/util/alloc.c` and
`src/util/errors.c`, and any test that runs libgit2 before its initialisation.

## libgit2's allocator family is reached through `alloc.rs`'s strategies

- **Rule.** Rust allocates, copies and releases memory in libgit2's allocator family
  (`git__malloc`, `git__calloc`, `git__mallocarray`, `git__strdup`, `git__strndup`,
  `git__substrdup`, `git__free`) through the strategies of `src/libgit2/src/util/alloc.rs`:
  `Free` / `AllocBox`, `ArrayFree` / `AllocVec` and `StrFree` / `AllocStr`, which call
  those primitives through their raw bindings, the two `static inline` ones through the
  wrappers in `sys/libgit2/static_fns.c`. New owners of memory libgit2 frees with
  `git__free` use those policies, not new ones over `git_odb_backend_data_alloc` /
  `git_odb_backend_data_free`, whose bodies are `git__malloc` / `git__free` but which are
  odb.c's exports and loop back through the strategies once odb.c is ported.
- **Evidence.** wave 12 batch 1 (`src/util/alloc.c`) reached the family through odb.c's
  exports while the primitives were hidden in `libgit2.so`; the static campaign link
  (`d21c78c70`) made them link, and wave 12 batch 2 rebound the strategies over them. The
  `OdbData*` policies of `src/libgit2/src/libgit2/odb.rs` (`OdbDataBox`, `OdbDataVec`,
  `OdbDataStr`, `OdbDataStrVec`) and the raw `git_odb_backend_data_alloc` calls of
  `oidarray.rs` predate them.
- **Scope.** Every module's owners of strings, singletons and arrays libgit2 frees with
  `git__free`. The existing `OdbData*` uses are odb.c's batches' to revisit: where they
  hold memory C frees with `git__free` rather than with `git_odb_backend_data_free`, they
  take these strategies.

## libgit2's allocator storage is 8-aligned, not `malloc`-aligned

- **Rule.** A policy over libgit2's allocator family (`git__malloc` and its siblings, and the
  exports whose bodies are theirs, such as `git_odb_backend_data_alloc`) vouches for the
  wider of `usize`'s and `u64`'s alignment, never `max_align_t`'s. libgit2 configured with
  `DEBUG_STRICT_ALLOC`, as its own CI is, runs on `debugalloc`, which returns `malloc`'s block
  offset by a `size_t`. A type aligned to 16 is refused at compile time, not allocated.
- **Evidence.** wave 12 batch 3's `ub_safe/strict-alloc-misaligns-wide-runs`: with
  `MAX_ALIGN = align_of::<max_align_t>()`, `AllocVec::<u128>::from_slice` formed a misaligned
  slice from safe code on a private `-DDEBUG_STRICT_ALLOC=ON` build. `odb.rs`'s `DATA_ALIGN`
  already followed this rule.
- **Scope.** Every lifecycle policy, owner or alignment claim over storage libgit2's
  allocator makes, whether it comes from Rust or from C.

## The tree's inline `mod unit_tests` modules are no precedent for test placement

- **Rule.** A new unit test of a module's public interface goes in `tests/unit_pub`, at the
  tested TU's path (`tests/unit_pub/src/util/...`), and one of an internal routine in an
  inline `#[cfg(test)] mod unit_priv` beside it, as the conventions' Testing section places
  them, whatever the module's file or its neighbours hold. The 34 inline `mod unit_tests`
  modules under `src/` (`alloc.rs` and `errors.rs` among them) predate that placement and
  mix both kinds; a batch that adds tests to one of those files puts its new tests where the
  conventions place them, and leaves the old ones to the batches that port their modules.
  Until `tests/unit_pub` reaches the working branch, a wave that needs it creates it with
  the same `main.rs`, `src.rs`, directory files and `[[test]] unit_pub` entry as waves 169
  and 176, so that the branches merge with only the one-line `mod` additions conflicting.
- **Evidence.** wave 182 batch 0 (`cff595a09`) put `win32_leakcheck.rs`'s three
  public-interface tests in an inline `mod unit_tests`, following `alloc.rs` and
  `errors.rs`; its module pass (`wave-182/batch-1`) moved them to `tests/unit_pub` and
  renamed the module `unit_priv` (a `conventions` finding).
- **Scope.** Every batch that writes unit tests, until the legacy `unit_tests` modules are
  gone.

## Test allocators start from libgit2's standard allocator

- **Rule.** A test that installs an allocator which refuses, counts or delays some requests
  builds it as libgit2's own out-of-memory tests do (`tests/util/str/oom.c`): it copies
  `stdalloc::allocator()` (`src/libgit2/src/util/allocators/stdalloc.rs`), replaces the
  members it changes, and forwards the rest of each request to the standard member it
  replaced, read once from that value. It does not declare the C library's `malloc`,
  `realloc` and `free` in an `extern` block of its own, nor reimplement an `Alloc` over them
  only to stand in for the standard family.
- **Evidence.** wave 181 batch 0 bound `git_stdalloc_init_allocator` once the static
  campaign link (`d21c78c70`) made it link; it had been hidden from `libgit2.so`. Before it,
  about thirty test files (the `Refusing` and `Countdown` allocators of `buf.rs`,
  `config.rs`, `index.rs`, `refs.rs`, `settings.rs` and others, and their `equiv` and
  `ub_unsafe` counterparts) each declared libc's `malloc` family to build one. The module
  pass (wave 181 batch 1) built the out-of-memory equivalence test this way, as batch 0
  did, and restores the default with `set_allocator(None)` as `test_str_oom__cleanup` does.
- **Scope.** Every test allocator written from wave 181 on. The existing ones stay until
  their modules' batches revisit them; they are not wrong, since the standard family is the
  C library's heap, but they repeat a binding by hand.

## reftable's error type is `basics::Error`, which `error.c`'s port takes over

- **Rule.** Rust reports reftable's error codes as `deps::reftable::basics::Error`, and no
  other reftable module defines an error type of its own. The batch that ports
  `enum reftable_error` (`deps/reftable/error.c`, wave 217 under planner run 12's numbering)
  makes that type the enum's port, not a second one: it moves `Error` to
  `src/libgit2/deps/reftable/error.rs` with a variant per constant of
  `deps/reftable/reftable-error.h`, each anchored to its constant, keeps `Other(c_int)` for
  codes outside the enum and `from_code` / `code`, gives `Display` `reftable_error_str`'s text,
  and leaves `pub use super::error::Error;` in `basics.rs`, so that `basics.rs`'s callers and
  tests do not change.
- **Evidence.** `deps/reftable/basics.c` returns `REFTABLE_OUT_OF_MEMORY_ERROR` and
  `REFTABLE_FORMAT_ERROR`, yet was scheduled (wave 169, foundational, now integrated) with no
  dependency on `enum reftable_error`, since wavefront records no use of an enum constant
  (`crustify/evolution/wavefront-enum-constant-uses`). Wave-169/batch-2 therefore defined
  `basics::Error`, unanchored, with three of `reftable_error_str`'s strings
  (`wave-169/batch-4`'s `conventions/reftable-error-type-homed-in-basics`;
  `crustify/evolution/reftable-error-wrapped-ahead-of-basics`).
- **Scope.** The batch of wave 217 that ports `enm::reftable_error` and
  `fn::reftable_error_str`, and every reftable module's batch until then.

## A libgit2 module's switch removes its file from `c/` and changes no CMake file

- **Rule.** libgit2's CMake globs each directory's sources (`file(GLOB …)` in
  `src/util/CMakeLists.txt`, `src/libgit2/CMakeLists.txt`, `src/cli/CMakeLists.txt` and the
  tests'), so a module's switch removes its TU, and the tied C tests that reach its internals,
  from `c/` on the fork's `fork/main`, and edits no `CMakeLists.txt`: the C that remains
  already links the Rust's static library wherever it links (`LIBGIT2_RUST_LIBRARY`,
  `c/cmake/SelectRust.cmake`). A glob is read at `configure` only, so the variants are
  configured afresh, as a new generation (`crustify/scripts/build-variants.sh`), never
  rebuilt in place: a build that only reran `cmake --build` would still list the removed file
  as an input, and fail on it.
- **Evidence.** `src/util/utf8.c`'s switch (`modules/libgit2/src/util/utf8.c/integration/run-1`,
  the fork's `1b37e3d08`): removing the file was the whole C change of the module, beside the
  one-time CMake change; `crustify/builds.gen1`'s archives hold 212 objects where the previous
  generation's held 213, `utf8.c.o` gone.
- **Scope.** Every later module's switch, and every rebuild of the variants after one.

## A `git2` module's exports are checked through the CLI

- **Rule.** No C test reaches the `git2` link unit, the CLI under `src/cli/`: its modules'
  `tests` are empty, and no suite under `c/tests` builds or runs `src/cli`. Its exports are
  therefore checked, before the module's switch, by the gateway's own tests, written after the
  CLI's C that calls them and run with the export feature on against a private build whose
  archive lacks the module's object, and under Miri; and by the CLI itself: the variant's
  `git2` relinked from its `build.ninja` edge (`build git2: C_EXECUTABLE_LINKER__git2_cli_…`),
  without the module's `src/cli/CMakeFiles/git2_cli.dir/<tu>.o` and with the crate built as a
  `staticlib` with the feature on (`CRUSTIFY_STATICLIB=1`, `CRUSTIFY_BUILD_DIR` the private
  build, a `CARGO_TARGET_DIR` of its own), then run beside the stock `git2` over the commands'
  usual and failing command lines in scratch repositories, every difference in exit status,
  output or files written either a departure the module records or a finding. After the
  switch, the relinked CLI is the variants' own, and the same command lines check it.
- **Evidence.** `wave-186/batch-7/run-1`, `src/cli/opt.c`'s module pass: its gateway tests
  under Miri found `ub_export/args-slot-handed-to-c-through-a-dead-borrow`, and its relinked
  `git2`, over the 66 command lines of its run dir's `cli-smoke.py`, matched the stock one on 62,
  the 4 others the parser's recorded departures (the bare `-`, and the literal `--`'s
  reordering).
- **Scope.** Every module of the `git2` link unit, at its module pass and its switch, until the
  CLI has tests of its own.

## cmake-property-naming-a-removed-source

Status: held

**Rule.** When a module's switch removes a C file that a `CMakeLists.txt` names only in a
source-file property, the `CMakeLists.txt` is left unchanged. CMake ignores a property of a
source no target lists. This extends the lesson "A libgit2 module's switch removes its file
from `c/` and changes no CMake file": the switch still edits no CMake file. The dangling
names stay while the directory keeps a C source. The switch that removes its last one is
outside this rule: the directory's library target is then left with no source to compile,
and CMake fails to generate, so that switch edits CMake.

**Evidence.** Item 13 of `module-switch-procedure`. `deps/llhttp/CMakeLists.txt` globs its
sources, as the lesson has it, and also sets
`set_source_files_properties(api.c http.c llhttp.c PROPERTIES COMPILE_FLAGS …)`. At
`deps/llhttp/api.c`'s switch (`modules/libgit2/deps/llhttp/api.c/integration/run-1`) the
file was left unchanged, and gen3's five `configure`s and builds passed with it. At
`deps/llhttp/http.c`'s switch (`modules/libgit2/deps/llhttp/http.c/integration/run-1`) the
property names `api.c` and `http.c`, both gone, and `llhttp.c`; the file is unchanged, and
gen5's five `configure`s and builds passed with it, each variant's `build.ninja` still
compiling `llhttp.c` with the property's flags. A copy of the fork at `230c9dbdb` with
`llhttp.c` removed and the file unchanged fails `cmake` (`CMake can not determine linker
language for target: llhttp`), where the copy with it configures (`fixes/gates-green/batch-0`,
`misc/lesson-cmake-property-takes-the-last-source`).

Scope: the switches of `deps/llhttp/api.c` and `http.c`, and of any later module whose `CMakeLists.txt` names its file in `set_source_files_properties` and whose directory keeps a C source after it; not `deps/llhttp/llhttp.c`'s.

## http-exports-gated-through-a-local-remote

Status: held

**Rule.** No offline C test drives libgit2's HTTP client: `httpparser.c`, `httpclient.c` and
`llhttp.c`'s generated parser call llhttp's exports only in a fetch, clone or push over
HTTP, whose suites (`online::*`) the baseline disables, since they need the network. The
switch of a module whose exports only the HTTP client reaches therefore runs
`crustify/scripts/http-gate.sh` on the previous generation and on the new one, on plain,
ASan+UBSan and TSan, and compares the two `results.txt` with `diff`. On the new generation's
coverage variant, the run also records which of the module's exports the gate calls.
`http-gate.sh` starts two remotes from `crustify/scripts/http-remote.py`, one open and one
asking for Basic credentials. It runs a variant's `git2 clone` of two repositories through
each framing libgit2 reads to its end (`Content-Length`, chunked, chunked with extensions),
four clones that fail (close-delimited, malformed, a missing repository, the guarded remote
without credentials), and `libgit2_tests -sonline::push -sonline::clone::credentials` with
`GITTEST_REMOTE_URL` at the guarded remote, through each of those three framings.

**Evidence.** Item 11 of `module-switch-procedure`, met at `deps/llhttp/api.c`'s switch
(`modules/libgit2/deps/llhttp/api.c/integration/run-1`, the fork's `c54c0c180`). That no
offline C test drives the client is the module pass's finding; the 31 exports are api.c's.
libgit2's own CI runs part of the online suites offline, against servers on localhost named
by `GITTEST_REMOTE_URL` (`ci/test.sh`'s `auth_clone_and_push`, `-sonline::clone::push
-sonline::push`). The run added `http-remote.py`, `git http-backend` behind a persistent
HTTP/1.1 front that frames each response by its path (`Content-Length`, chunked in 1, 7,
4096 and 65536-byte chunks, chunked with chunk extensions, or close-delimited), with an
optional Basic challenge, and `http-gate.sh`, whose `results.txt` is free of times, ports
and paths. Gen2's C and gen3's Rust give the same `results.txt` on plain, ASan+UBSan and
TSan, and on gen3's coverage variant the run calls 24 of the 31 exports. A mutant of an
export was not built, so whether the comparison catches a departure rests on the coverage
alone. At `deps/llhttp/http.c`'s switch
(`modules/libgit2/deps/llhttp/http.c/integration/run-1`), `http-gate.sh` over gen4's C and
gen5's Rust, on plain, ASan+UBSan and TSan, gives the same `results.txt`, and on gen5's
coverage variant the run calls all four exports. Both switches' `http-gate.sh` (`0c1bd7a0d`,
`46055d7c3`) run four failing clones, and http.c's recorded `http-gate-results.txt` lists
them (`fixes/gates-green/batch-0`, `misc/lesson-http-gate-miscounts-failing-clones`). The rule
without libgit2's names is the proposal `switch-gate-beyond-c-tests`.

Scope: the switch of every module whose exports libgit2 calls only through its HTTP client: `deps/llhttp/*.c`, `src/libgit2/transports/httpparser.c` and `httpclient.c`.

## http-gate-tallies-connections

Status: held

**Rule.** `http-gate.sh`'s comparison includes the connections the client opened to each
remote, by framing (`connections <side> <framing> count=<n>`), beside the requests each
remote answered. The request lines, by method, framing and status, do not change when the
client stops reusing its connections, so they alone do not check
`llhttp_should_keep_alive`. A switch whose exports change what the gate's lines cannot show
adds a line that shows it, after the existing lines, which stay unchanged.

**Evidence.** Item 16 of `module-switch-procedure`, met at `deps/llhttp/http.c`'s switch
(`modules/libgit2/deps/llhttp/http.c/integration/run-1`, the fork's `230c9dbdb`).
`http-gate.sh`'s `results.txt` tallied the requests each remote answered by method, framing
and status: `llhttp_should_keep_alive`, the export of this switch that libgit2's client
calls, could have returned `0` always, and every line would have matched. The run made
`http-remote.py` log each request's place on its connection (`conn=<n>`) and `http-gate.sh`
tally the connections the client opened to each remote by framing, lines added after the
request lines, which are unchanged. Four runs over gen4 (plain twice, ASan+UBSan, TSan) gave
the same connection tally, 9 connections to the open remote for its 15 requests and 73 to
the guarded one for its 610, and the same request lines as api.c's record; gen5's equal
them. A mutant of `llhttp_should_keep_alive` was not built, so that the tally catches one
rests on the client reading the export once per response (`httpclient.c`'s
`on_headers_complete`). The change is in `http-remote.py`'s log and `http-gate.sh`'s
`results.txt`, in the campaign only, validated by the four identical runs over gen4 and
gen5's equal ones.

Scope: every run of `crustify/scripts/http-gate.sh`, and every change to it or to `crustify/scripts/http-remote.py`.
