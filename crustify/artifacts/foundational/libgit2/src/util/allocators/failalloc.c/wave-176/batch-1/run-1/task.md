# wave-176/batch-1/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/allocators/failalloc.c/wave-176/batch-1`, on `crustify/batches/wave-176/batch-1`.
- Worklist: `crustify/artifacts/foundational/libgit2/src/util/allocators/failalloc.c/wave-176/batch-1/run-1/task.json` in it.
- Batch: `wave-176/batch-1` of the foundational module `libgit2/src/util/allocators/failalloc.c` (`modules.json`'s
  `foundational: true` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-176`. Name it
  `refs/heads/crustify/waves/wave-176` in every git command: a `crustify/reviews/...` or
  artifact path can shadow the short name.

## Instructions

- Four batches run at once on this host: `wave-169/batch-1`, `wave-169/batch-2`,
  `wave-176/batch-1` and `wave-12/batch-2`. Other workloads share the host. It has about
  40 GB of memory free and the disk about 140 GB. Cap your builds at 16 jobs
  (`CARGO_BUILD_JOBS=16`, and the same for the C builds and tests), keep scratch builds
  small, and delete them when done. End only processes of your own worktree, as the
  `concurrency` skill describes.
- Campaign mode now links each variant's C statically (`d21c78c70`, Crustify `20500ce`:
  `setup`, Builds, and `bindgen`, Build modes). Every function the C defines with external
  linkage links through the raw bindings, the internal ones included. Extend
  `allowlist.txt` and `wrapper.h` as `bindgen`'s Extending section describes. The build
  takes the internal include dirs from libgit2's own compile flags. Do not reproduce a C
  body in Rust, or route around a primitive through another exported function, because a
  symbol is hidden: that is no longer needed. `libgit2-sys/tests/hidden_symbols.rs` is the
  fix's proof; keep it. The open proposal `internal-headers-in-the-bindings` notes that
  wrappers of internal `static inline` functions do not build in installed-library mode.
  Follow the build as it stands, and add evidence to that proposal if you meet it.
- Your wave branch now holds a merge of the working branch, which took the static link
  in. Name the branch as `refs/heads/crustify/waves/wave-<i>` in every git command.
- The workspace adopted crates for the out-of-tree libraries (`dependencies.json`'s
  `decision`s): reach libc through `std`, `nix` or `glibc-bindings` as each entity's route
  says, and report each entity's route as the `dependencies` skill's Confirmation section
  asks.
- The base's `ub_safe::compile_fail` has 5 known trybuild mismatches that print
  `/opt/venv/share/ffibox/...` where the snapshot expects `$FFIBOX`
  (`commit_arc`, `object_arc`, `tag_and_tree_arc_no_mut_handle`, `mailmap_not_clone`,
  `tree_builder_not_clone`). Under `cargo bsan`, the bindings check differs by doc comments
  alone, because the bsan clang documents macro constants. Neither is a regression of
  yours.
- Your worklist is batch 0's (`9a64abfc7`), bound again. Departure: a `done` batch's
  items are bound anew because the build under them changed. Batch 0 found
  `git_failalloc_malloc`, `_realloc` and `_free` hidden. It reproduced their bodies in Rust
  (`FailAlloc`, `FailArrayAlloc`, with `Replaces` anchors), and documented that a
  reproduction never equals the C function's address. `git_allocator_global_init`
  compares `git__allocator.gmalloc` with `git_failalloc_malloc`. All three now link.
  Rebind them: the strategies call the C primitives through their raw bindings, and the
  `Replaces` reproductions and their caveat go. Hand C its own function's address where a
  `git_allocator` is filled with them. Keep what batch 0 got right: its tests, its
  surface, and its notes that still hold.
- Batch 0's open proposal, `unexported-lifecycle-primitives`, is superseded by the static
  link. Leave it to the assistant.
