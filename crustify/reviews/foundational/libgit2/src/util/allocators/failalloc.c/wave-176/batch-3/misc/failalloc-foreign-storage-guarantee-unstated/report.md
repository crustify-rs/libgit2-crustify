Filed by the assistant from 018f7da5e and 018f7da5e, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# `FailAlloc`'s acceptance of other families' storage was stated by no contract

- **finding**: `misc/failalloc-foreign-storage-guarantee-unstated`
- **batch**: `wave-176/batch-3/run-1`, the module pass of `libgit2/src/util/allocators/failalloc.c`
  (reviews batches 0-2: `9a64abfc7`, `7045f4428`, `ab7d4915c`, and `Alloc::C_ALLOCATOR` whole)
- **items**: `fn::git_failalloc_free`, `fn::git_failalloc_malloc`, `fn::git_failalloc_realloc`
- **affected commit**: `ab7d4915c`
- **source commits**: `018f7da5e` (the batch has one commit, which carries the finding and its fix)

## Finding

failalloc.rs's module docs advertised installing `Allocator::new::<FailAlloc>()` after
`git_libgit2_init`. `set_allocator`'s second obligation then asks that every allocation
libgit2 holds be one the new family can resize and release, but `Alloc`'s contract and
`FailAlloc`'s docs covered only the family's own storage (of which there is none). The
equivalence suite made that call anyway, on a property no contract stated ("releases
without touching what it is handed"), and called its foreign-buffer probes "beyond the
family's contract"; tests/ub_unsafe had no test of `Alloc::free` or `Alloc::realloc(Some)`
within a stated contract.

## Evidence

The C accepts any pointer (`GIT_UNUSED` bodies, failalloc.c:19-32), so the use is sound and
leaks.

Rerun on `ab7d4915c`:

```
sed -n 46,53p src/libgit2/src/util/allocators/failalloc.rs
sed -n 82,99p src/libgit2/src/util/allocators/failalloc.rs
sed -n 274,285p tests/equiv/src/util/allocators/failalloc.rs
```

## Reproducer

None: the use is sound and leaks, and no reproducer applies.

## Fix

In `018f7da5e`: `FailAlloc` documents that its release and resize never read, write, keep or
release the pointer handed to them, so storage of any family may reach them and is left
alone (`FailArrayAlloc`'s release too); the module docs say a post-init install leaks what
libgit2 frees and link it; the equiv SAFETY comment and module docs cite it; ub_unsafe gains
`realloc::resizes_of_live_storage_fail_and_leave_it_whole` and
`free::releases_nothing_it_is_handed`.
