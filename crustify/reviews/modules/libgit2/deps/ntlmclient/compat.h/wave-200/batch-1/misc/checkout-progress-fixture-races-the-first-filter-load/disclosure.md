To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-crustify: the checkout progress test fixture races libgit2's first filter load (TSan)

Hi Marius,

This is a test defect, not a soundness bug in the public API, so it can be handled in
public.

In `src/libgit2/src/libgit2/checkout.rs` (commit 4b6e5a4db), the unit-test fixture
`progress_callback::Staged` stages files through raw `git_index_add_bypath`, which loads
filter lists. libgit2 marks a filter initialised on its first load under only the
registry's read lock (`filter_initialize`, `filter.c:344`), and later loads read that flag
(`filter.c:569`). The crate closes this race with `filter::initialise_builtin_filters`,
which every filter constructor and blob writer calls. The fixture used a `OnceLock` of its
own instead, so its first load could still run beside a blob test's. Under TSan, with 24
test threads, the `lib` binary fails every time:

```text
WARNING: ThreadSanitizer: data race
  Read of size 4 ... by thread T2 (mutexes: read M0):
    #0 git_filter_list__load src/libgit2/filter.c:569
    #3 git_index_add_bypath src/libgit2/index.c:1043
    #4 checkout::unit_tests::progress_callback::Staged::build
  Previous write of size 4 ... by thread T1 (mutexes: read M0):
    #0 filter_initialize src/libgit2/filter.c:344
    #2 filter::initialise_builtin_filters::{closure#0}
    #8 blob::unit_tests::absolute_filter_paths_are_refused_before_c
```

Running those two tests alone on two threads reproduces it (6 runs of 6).

The fix: the fixture pins the runtime and then calls `initialise_builtin_filters()` before
staging, as the safe constructors do. The fixture's own priming is removed. After that,
the pair passes 8 runs of 8 and the whole `lib` binary 3 of 3 under TSan. The fix is on
the review branch (`crustify/waves/wave-200`), and the report is under
`crustify/reviews/modules/libgit2/deps/ntlmclient/compat.h/wave-200/batch-1/misc/`. I can
send the reproducer commands and the exact diff on request.

The race itself is libgit2's: two threads making their first filter loads at once, in
different repositories, race on that flag.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
