Filed by the assistant from ab7d4915c, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host). The commit names the finding only by its objective; its slug is the assistant's.

# `Alloc`'s safety contract forbids the calls `C_ALLOCATOR` exists to make

- **finding**: `contracts/ffi_import/alloc-safety-forbids-libgit2-calls`
- **batch**: `wave-176/batch-2/run-1` of `libgit2/src/util/allocators/failalloc.c`, the review of batches 0 and 1 (`9a64abfc7`, `7045f4428`) and `Alloc::C_ALLOCATOR`
- **items**: `fn::git_failalloc_free`, `fn::git_failalloc_malloc`, `fn::git_failalloc_realloc`
- **affected commit**: `7045f4428`
- **source commits**: `ab7d4915c` (fix; the batch filed no separate commit)

## Finding

`Alloc`'s `# Safety` ended with "None of them may call into libgit2". The `C_ALLOCATOR`
bullet added by `7045f4428` exists to hold libgit2's own routines, and `FailAlloc`'s methods
call `git_failalloc_*`, so the impl breaks the rule as written. Its SAFETY comment justified
the impl by a narrower rule the contract never stated: none allocates through libgit2 or
touches its state. "Them" did not say whether `C_ALLOCATOR`'s functions were covered, and
"they are its C counterparts" did not say whether those functions share storage with the
methods.

## Evidence

The impl is sound and no input breaks it, so there is no reproducer.
Rerun: `sed -n 299,358p src/libgit2/src/util/alloc.rs` on `7045f4428`.

## Fix

The contract now forbids allocating through libgit2 or touching its state, for the methods
and `C_ALLOCATOR` alike. It names the failing family's routines as allowed, and it asks for
one family over the methods' storage. `FailAlloc`'s SAFETY comment cites that rule
(`ab7d4915c`).
