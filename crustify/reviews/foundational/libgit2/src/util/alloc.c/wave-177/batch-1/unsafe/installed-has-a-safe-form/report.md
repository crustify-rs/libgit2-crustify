Filed by the assistant from e455870fc and 1a34b9caf, the host having refused the review batch's own write (crustify/evolution/review-reports-refused-by-the-host).

# `installed()`, the borrow of `git__allocator`, has a safe form

- **finding**: `unsafe/installed-has-a-safe-form`
- **batch**: `wave-177/batch-1/run-1` of `libgit2/src/util/alloc.c`
- **item**: `glb::git__allocator`
- **affected commit**: not recorded: the report was refused by the host; filed from
  e455870fc and 1a34b9caf (the batch follows wave 177 batch 0, `fce21ef4e`)
- **source commits**: `e455870fc` (file), `1a34b9caf` (fix)

## Finding

`installed()`, the borrow of `git__allocator`, was a `pub unsafe fn` whose caller had to
exclude every write on other threads, the first `git_libgit2_init` included, which safe code
makes. Pinning libgit2's runtime first leaves only the unsafe installers (`set_allocator`,
`global_init`) as writers, whose contracts already keep other threads out of libgit2 while
they write, so the borrow has a safe form (binding-symbols, Globals: a global C sets up in a
routine is reached safely once that routine has run).

## Evidence

As the finding states. The exact command to rerun: not recorded: the report was refused by
the host; filed from e455870fc and 1a34b9caf.

## Reproducer

None recorded: not recorded: the report was refused by the host; filed from e455870fc and
1a34b9caf.

## Fix

In `1a34b9caf`: `installed()` is a safe fn returning `Option<AllocatorRef<'static>>`: it
pins libgit2's runtime first, so the first initialisation's write happened before any read
through the handle and no later one installs anything; None when libgit2 cannot be
initialised. `set_allocator`'s and `global_init`'s contracts now name reads through the
handle among what they keep off other threads. `global_init` stays unsafe: it writes the
global unsynchronised, and after an unsafe `set_allocator` re-installed the failing family
it would race other threads' allocations. API change after first landing (wave 177 batch
0), kept for that reason: the item's only consumers are tests. ub_unsafe's installed suite
moves: its reads before initialisation read the C global directly, the threaded read and a
race of first borrows go to ub_safe, and the install-following test becomes
`set_allocator`'s. The equivalence test reads the pre-initialisation global through C and
compares installs after the borrow has pinned the runtime.
