# wave-169/batch-0/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/deps/reftable/basics.c/wave-169/batch-0`, on `crustify/batches/wave-169/batch-0`.
- Worklist: `crustify/artifacts/foundational/libgit2/deps/reftable/basics.c/wave-169/batch-0/run-1/task.json` in it.
- Batch: `wave-169/batch-0` of the foundational module `libgit2/deps/reftable/basics.c` (`modules.json`'s
  `foundational: true` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-169`. Name it
  `refs/heads/crustify/waves/wave-169` in every git command: a `crustify/reviews/...` or
  artifact path can shadow the short name.

## Instructions

- Three batches run at once on this host (`wave-169/batch-0`, `wave-176/batch-0`,
  `wave-12/batch-1`), beside other workloads: take your share of the jobs, as the
  `concurrency` skill describes, and end only processes of your own worktree.
- The workspace adopted crates for the out-of-tree libraries (`dependencies.json`'s
  `decision`s): reach libc through `std`, `nix` or `glibc-bindings` as each entity's
  route says, and report each entity's route as the `dependencies` skill's Confirmation
  section asks.
- Waves 170 to 175 are packed with wave 169: this batch binds the module's types first;
  its symbols follow in batches 1 and 2 once you land. The module's file is scaffolded
  empty.
