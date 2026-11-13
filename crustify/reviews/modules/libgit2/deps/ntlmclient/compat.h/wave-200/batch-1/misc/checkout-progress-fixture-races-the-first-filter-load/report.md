# The checkout progress fixture races libgit2's first filter load (TSan)

- **item**: none of this module's. The fixture
  `checkout.rs::unit_tests::progress_callback::Staged`, which the progress-callback and the
  notify and perfdata callback unit tests build their repositories with, came with
  `git_checkout_progress_cb`'s wrapper (`3b654f097`), and its priming with the notify and
  perfdata callbacks' (`b694c3653`).
- **affected commit**: `4b6e5a4db`, the tip of `crustify/waves/wave-200`, this review's base.
- **found by**: the wave's TSan gate, which this module pass runs over the whole workspace.
- **class**: test defect. Its cause is libgit2's known race, which the safe API closes and
  this fixture bypasses. No public API reaches it.

## Finding

libgit2 initialises a registered filter on the first list load that selects it:
`filter_initialize` (`src/libgit2/filter.c:344`) writes `fdef->initialized` under only the
registry's *read* lock, and every later load reads the flag (`:569`). Two threads making
their first loads at once therefore race. `filter.rs`'s module docs describe this, and the
crate closes it with `filter::initialise_builtin_filters`. That function initialises `crlf`
and `ident` once per process, under a `Once`. Every filter constructor and blob writer calls
it before loading a list, so no load can run before the flag has been written.

`Staged::new` loads lists through raw calls: `git_index_add_bypath` →
`git_blob__create_from_paths` → `git_filter_list_load`. It did not wait on that `Once`. It
primed the filters with a `OnceLock` of its own, which orders the checkout fixtures among
themselves but not against the lib binary's other first loads. The blob and filter unit
tests make those loads through `initialise_builtin_filters`. So a checkout fixture's
staging could read `initialized` while a blob test's initialisation wrote it.

`end-gates . --stages plain,asan-ubsan,tsan,miri --rev refs/heads/crustify/waves/wave-200`,
at `RUST_TEST_THREADS=24`, failed the `lib` binary under TSan (exit 66). The pair alone
fails on every run. From a gate snapshot or any checkout of the affected commit:

```sh
CARGO_TARGET_DIR=$PWD/target/tsan crustify/scripts/cargo-variant.sh tsan test -p libgit2-crustify --lib -- \
  --exact libgit2::src::libgit2::blob::unit_tests::absolute_filter_paths_are_refused_before_c \
  libgit2::src::libgit2::checkout::unit_tests::notify_and_perfdata_callbacks::a_break_cancels_before_any_write_and_skips_the_counts \
  --test-threads=2
```

It failed 6 runs of 6, and the whole `lib` binary failed 2 runs of 2 (2 reports each).
`tsan.log` here holds the first report, with the tree's, the C's and the toolchain's paths
shortened:

```
WARNING: ThreadSanitizer: data race
  Read of size 4 at 0x721800003014 by thread T2 (mutexes: read M0):
    #0 git_filter_list__load c/src/libgit2/filter.c:569
    #1 git_filter_list_load c/src/libgit2/filter.c:640
    #2 git_blob__create_from_paths c/src/libgit2/blob.c:233
    #3 git_index_add_bypath c/src/libgit2/index.c:1043
    #4 <…::checkout::unit_tests::progress_callback::Staged>::build src/libgit2/src/libgit2/checkout.rs
    ...
    #15 …::notify_and_perfdata_callbacks::a_break_cancels_before_any_write_and_skips_the_counts checkout.rs:2944
  Previous write of size 4 at 0x721800003014 by thread T1 (mutexes: read M0):
    #0 filter_initialize c/src/libgit2/filter.c:344
    #1 git_filter_lookup c/src/libgit2/filter.c:360
    #2 …::filter::initialise_builtin_filters::{closure#0}
    ...
    #7 …::blob::create_from_workdir
    #8 …::blob::unit_tests::absolute_filter_paths_are_refused_before_c blob.rs:2122
SUMMARY: ThreadSanitizer: data race c/src/libgit2/filter.c:569 in git_filter_list__load
```

The race depends on scheduling. The strnlen.h module pass's TSan gate passed the same
fixture on `7ef895eb2`, at 48 test threads, and nothing in `filter.rs`, `blob.rs` or
`checkout.rs` has changed since. The equivalence suite's checkout fixture
(`tests/equiv/src/libgit2/checkout.rs`) primes the same way. It is not affected: that
binary's other list-loading tests, the blob ones, each run in a child process of their own
(`isolated`).

## Fix

`Staged::new` pins libgit2's runtime, then calls `filter::initialise_builtin_filters()`
before it stages a file, as the safe constructors do. The fixture's own `OnceLock` priming
and its `build("prime", …)` repository are gone, `build` is folded into `new`, and the
comment says why. No test's
scenario or assertion changed.

On the fixed tree, the pair above passes 8 runs of 8 under TSan, and the whole `lib`
binary 3 runs of 3 (840 passed, 1 ignored, no report). A first version of the fix called
`initialise_builtin_filters` before the fixture's `git_libgit2_init`. TSan then reported
`git__malloc` reading the allocator that another thread's `git_libgit2_init` was writing.
That is the precondition the function's docs state, a pinned runtime, which the final
version meets.
