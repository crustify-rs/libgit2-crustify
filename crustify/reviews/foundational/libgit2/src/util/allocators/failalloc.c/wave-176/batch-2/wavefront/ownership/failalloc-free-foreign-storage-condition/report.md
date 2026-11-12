Filed by the assistant from ab7d4915c, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host). The commit names the finding only by its objective; its slug is the assistant's.

# `git_failalloc_free`'s ownership note misstates when foreign storage reaches it

- **finding**: `wavefront/ownership/failalloc-free-foreign-storage-condition`
- **batch**: `wave-176/batch-2/run-1` of `libgit2/src/util/allocators/failalloc.c`
- **item**: `fn::git_failalloc_free`
- **affected commits**: `9a64abfc7`, `7045f4428`
- **source commits**: `ab7d4915c` (fix; the batch filed no separate commit)

## Finding

`git_failalloc_free`'s `ptr` note said foreign storage reaches the routine "only possible
before git_libgit2_init". `set_allocator` memcpys any family into `git__allocator`, and the
equivalence suite installs `Allocator::new::<FailAlloc>()` after initialisation
(`tests/equiv/src/util/allocators/failalloc.rs:275-285`), so from then on `git__free` hands
standard-allocator storage to it.

## Evidence

Not recorded beyond the citations above: the report was refused by the host; filed from
`ab7d4915c`.

## Fix

The note is resubmitted with the corrected condition, and its properties are unchanged
(`ab7d4915c`).
