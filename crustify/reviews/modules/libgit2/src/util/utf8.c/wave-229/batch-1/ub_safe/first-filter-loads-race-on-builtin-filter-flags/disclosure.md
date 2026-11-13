For: libgit2's issue tracker (https://github.com/libgit2/libgit2/issues), as a public issue:
a data race with no security impact we know of, which `SECURITY.md` does not reserve for the
private list.

Title: Data race on `git_filter_def.initialized` when two threads load their first filter lists

### Summary

`git_filter_list__load` and `git_filter_lookup` initialise a registered filter lazily, the
first time it is selected, and they do it holding only the filter registry's *read* lock:

```c
/* src/libgit2/filter.c, git_filter_list__load */
if (!fdef->initialized && (error = filter_initialize(fdef)) < 0)   /* :569, read  */
	break;

/* filter_initialize */
fdef->initialized = true;                                          /* :344, write */
```

Several threads may hold the read lock at once, so two threads whose first filter loads
overlap read and write `initialized` unsynchronised, and may run a filter's `initialize`
callback twice. `docs/threading.md` allows each thread to use its own repository, and the
threads here share no libgit2 object.

### Reproduction

At `0551dfd4a`, built with `-fsanitize=thread`: two threads, each with its own repository
whose `.gitattributes` holds `* text`, call `git_repository_hashfile` on a file at the same
time (any routine that loads a list does: `git_index_add_bypath`, checkout, workdir diffs).
ThreadSanitizer reports, every run:

```text
WARNING: ThreadSanitizer: data race
  Read of size 4 by thread T2 (mutexes: read M0):
    #0 git_filter_list__load src/libgit2/filter.c:569
    #1 git_filter_list_load src/libgit2/filter.c:640
    #2 git_repository_hashfile src/libgit2/repository.c:3601
  Previous write of size 4 by thread T1 (mutexes: read M0):
    #0 git_filter_list__load src/libgit2/filter.c:344
    #1 git_filter_list_load src/libgit2/filter.c:640
    #2 git_repository_hashfile src/libgit2/repository.c:3601
SUMMARY: ThreadSanitizer: data race src/libgit2/filter.c:569 in git_filter_list__load
```

We found it through Rust bindings, which reach it from safe code; the reproducer, a small
program, is available on request.

### Possible fixes

Initialising a filter under the registry's write lock (taking it, and checking the flag
again, when a reader finds a filter not yet initialised), or initialising the built-in
filters in `git_filter_global_init`, would close it. Our bindings work around it by
selecting `crlf` and `ident` once, right after `git_libgit2_init`, before any repository
exists. We would be glad to prepare a patch along either line if that helps.

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
