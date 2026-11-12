Filed by the assistant from 1b6fb129c and ab780bd47, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# A refused copy of a string list leaks the array and the copies made before it

- **finding**: `logic/refused-copies-leak`
- **batch**: `wave-184/batch-1/run-1`, the module pass of `libgit2/src/util/strlist.c`, over
  wave 184 batch 0 (`aba03725e`), its six items
- **items**: `fn::git_strlist_copy`, `fn::git_strlist_copy_with_null`
- **affected commit**: `aba03725e`
- **source commits**: `1b6fb129c` (file), `ab780bd47` (fix)

## Finding

`StrListFree::copy`, `NullStrListFree::copy` and the lists' `try_clone`/`Clone` called
`git_strlist_copy` / `git_strlist_copy_with_null`, whose `GIT_ERROR_CHECK_ALLOC` returns -1
without releasing the `git__calloc` array and the `git__strdup` copies made before the
refused one; the wrappers documented and kept the leak. A leak on a reported failure departs
from libgit2's convention for partial results (merge_env's on_error path) and from a None
result's contract.

## Evidence

The reproducer: a libc-backed allocator counting live allocations and refusing 5-byte
requests; copying ["one", "four"] leaves 2 allocations live per wrapper:

```
assertion `left == right` failed: StrListFree::copy: the refused copy leaks the array and the copy of "one"
left: 2 right: 0
```

exit 101 (`run-affected.log`, beside this report). `disclosure.md` sits beside it.

## Reproducer

`Cargo.toml` and `src/main.rs` beside this report; it fails on `aba03725e`. Run from its
directory with `cargo run` (plain variant).

## Fix

In `ab780bd47`: the copies are composed in Rust from the requests strlist.c makes, in its
order: the zeroed array through `ArrayFree::new_zeroed` (`git__calloc`), then each string
through `StrFree::dup` (`git__strdup`), collected as owners, so a refusal drops the copies
and the array through `git__free`; the strings then move into the array, which the list
adopts under its policy, whose release is still `git_strlist_free` /
`git_strlist_free_with_null`. `StrListFree::copy`, `NullStrListFree::copy` and both
`try_clone` anchors move from Wraps to Replaces. `refused_copies_report_out_of_memory`
asserts each release after a refusal and runs under LeakSanitizer;
`c_copies_leak_what_the_wrappers_release` keeps C's failure paths covered and pins the
divergence (C: no release; the wrappers: two). The reproducer exits 0 (`run-fixed.log`).
