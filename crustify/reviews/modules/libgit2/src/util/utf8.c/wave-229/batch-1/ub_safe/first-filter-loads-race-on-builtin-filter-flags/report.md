# Two threads' first filter loads race on a built-in filter's `initialized` flag (TSan)

- **finding**: `ub_safe/first-filter-loads-race-on-builtin-filter-flags`
- **batch**: `wave-229/batch-1/run-1`, the module pass of `libgit2/src/util/utf8.c`. The
  module's end gates found it: TSan over the workspace, on the wave's tip `680ac73bf`, failed
  `lib` with this race (`libgit2::src::libgit2::checkout::unit_tests` staging files while a
  `blob` unit test initialised the filters). It is outside the module's workset, a
  true positive of its gate, filed and fixed here as `reviewing`'s Gates section has a
  gate's true positives handled.
- **items**: the wrappers of every libgit2 routine that loads a filter list internally,
  `fn::git_repository_hashfile` (`RepositoryRef::hash_file`), `fn::git_index_add_bypath`
  (`IndexMut::add_path`), `git_index_add_all` and `git_index_update_all`, checkout and the
  workdir diffs; and `src/libgit2/src/libgit2/filter.rs`'s `initialise_builtin_filters`,
  which closed the race for that module's constructors alone.
- **affected commit**: `680ac73bf` (the wave's tip); the race predates it: `filter.rs`'s
  module docs named it since the tree was homed per translation unit (`324180e8f`).
- **class**: UB reachable from the safe API: a data race in libgit2's C
  (`src/libgit2/filter.c`), between two threads that share no libgit2 object.

## Finding

libgit2 initialises a registered filter the first time a list selects it:
`git_filter_list__load` reads `fdef->initialized` (`filter.c:569`) and `filter_initialize`
writes it (`filter.c:344`), both under the filter registry's *read* lock, which orders
neither against another reader. Two threads whose first loads overlap therefore race on the
flag, whatever repositories they use, and libgit2's threading guide
(`docs/threading.md`) allows two threads to use two repositories at once.

The crate closed this only among `filter.rs`'s own constructors (`FilterList::new`,
`load`, `load_ext`) and the blob writers, each of which calls `initialise_builtin_filters`,
a `Once` that pushes `crlf` and `ident` onto a scratch list before its first load. libgit2's
routines that load a list inside C do not go through it, as `filter.rs`'s module docs said
("libgit2's defect, which only initialising the filters when the runtime is pinned would
close for the whole crate"). Two threads, each with a repository of its own whose
`.gitattributes` selects `crlf`, hashing a file at once through `RepositoryRef::hash_file`,
race, with no `unsafe`.

## Evidence

`src/main.rs` here (`#![forbid(unsafe_code)]`) runs that scenario, the threads meeting at a
`Barrier` before they hash, so that their loads are the process's first. From the
repository root:

```text
CARGO_TARGET_DIR=crustify/tmp/target-repro-tsan crustify/scripts/cargo-variant.sh tsan \
  run -q --manifest-path crustify/reviews/modules/libgit2/src/util/utf8.c/wave-229/batch-1/ub_safe/first-filter-loads-race-on-builtin-filter-flags/Cargo.toml
```

On `680ac73bf` it exits 66, five runs of five (`run-affected.log`, paths shortened):

```text
WARNING: ThreadSanitizer: data race
  Read of size 4 at 0x721800000014 by thread T2 (mutexes: read M0):
    #0 git_filter_list__load src/libgit2/filter.c:569
    #1 git_filter_list_load src/libgit2/filter.c:640
    #2 git_repository_hashfile src/libgit2/repository.c:3601
    #3 <RepositoryRef>::hash_file src/libgit2/src/libgit2/repository.rs
  Previous write of size 4 at 0x721800000014 by thread T1 (mutexes: read M0):
    #0 git_filter_list__load src/libgit2/filter.c:344
    #1 git_filter_list_load src/libgit2/filter.c:640
    #2 git_repository_hashfile src/libgit2/repository.c:3601
SUMMARY: ThreadSanitizer: data race src/libgit2/filter.c:569 in git_filter_list__load
```

(`filter.c:344` is `filter_initialize`'s `fdef->initialized = true`, inlined.)

## Fix

`runtime_pinned` (`src/libgit2/src/util/errors.rs`), the `OnceLock` every safe constructor
passes first, calls `initialise_builtin_filters` once libgit2 is initialised and before it
publishes the pinned runtime: the flags are set before any repository exists, and every
later load is ordered after that write by the `OnceLock`, so libgit2 only reads them. The
pinned runtime is never released, so the registry, and its flags, last for the process.
`initialise_builtin_filters` now only looks the two filters up: `git_filter_lookup` runs
`filter_initialize` for a filter not yet initialised, and neither built-in filter has an
`initialize` callback, so pinning allocates nothing. A first version kept the scratch list
the helper built and freed, and pinning then allocated 145 and 192 bytes, which eleven
allocator-tracing `equiv` tests, each run in a fresh child process, saw in their Rust traces
(`buf`, `odb::sys`, `oidarray`, `strarray` and `util::alloc`); with the lookups alone they
pass unchanged. `filter.rs`'s module docs and `initialise_builtin_filters`' say so; its
other callers stay, returning at once. A filter registered later through `git2/sys/filter.h` is still
initialised by its first load, which that header's wrapper must settle, as before.

The reproducer exits 0 on the fix, five runs of five (`run-fixed.log`). It becomes the UB
test `ub_safe::src::libgit2::filter::first_loads_on_two_threads_do_not_race`, run alone in
a child process (`fixture::isolated`) so that its loads are the process's first: it passes
under TSan with the fix, and fails with this race when `runtime_pinned` skips the call.

API stability: no public item changes.

`disclosure.md` beside this report is for libgit2's maintainers: the defect is theirs, and
any C or Rust consumer that loads filters from two threads meets it.
