# wave-12/batch-1/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/alloc.c/wave-12/batch-1`, on `crustify/batches/wave-12/batch-1`.
- Worklist: `crustify/artifacts/foundational/libgit2/src/util/alloc.c/wave-12/batch-1/run-1/task.json` in it.
- Batch: `wave-12/batch-1` of the foundational module `libgit2/src/util/alloc.c` (`modules.json`'s
  `foundational: true` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-12`. Name it
  `refs/heads/crustify/waves/wave-12` in every git command: a `crustify/reviews/...` or
  artifact path can shadow the short name.

## Instructions

- Three batches run at once on this host (`wave-169/batch-0`, `wave-176/batch-0`,
  `wave-12/batch-1`), beside other workloads: take your share of the jobs, as the
  `concurrency` skill describes, and end only processes of your own worktree.
- The workspace adopted crates for the out-of-tree libraries (`dependencies.json`'s
  `decision`s): reach libc through `std`, `nix` or `glibc-bindings` as each entity's
  route says, and report each entity's route as the `dependencies` skill's Confirmation
  section asks.
- Wave 12 started in the wrap campaign: its batch 0 landed `git_allocator` as
  `src/libgit2/src/util/alloc.rs` (commit `8d14c83d9`), before Crustify's unsafe-regions
  rule; its wave branch was forked anew from the module's branch. `alloc.c`'s other
  items (waves 177 to 180: the allocator's function pointers, `git__allocator`,
  `git__realloc`, `git__reallocarray` and setup) are bound in a later run of the module,
  once `failalloc.c` is integrated.
