# wave-12/batch-2/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/alloc.c/wave-12/batch-2`, on `crustify/batches/wave-12/batch-2`.
- Worklist: `crustify/artifacts/foundational/libgit2/src/util/alloc.c/wave-12/batch-2/run-1/task.json` in it.
- Batch: `wave-12/batch-2` of the foundational module `libgit2/src/util/alloc.c` (`modules.json`'s
  `foundational: true` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-12`. Name it
  `refs/heads/crustify/waves/wave-12` in every git command: a `crustify/reviews/...` or
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
- Your worklist is batch 1's (`c0c753430`), bound again. Departure: a `done` batch's
  items are bound anew because the build under them changed. Batch 1 found the family
  hidden. It reached `git__malloc` and `git__free` through `git_odb_backend_data_alloc` and
  `_free`, odb.c's exports, and composed `git__calloc`, `git__mallocarray`, `git__strdup`,
  `git__strndup` and `git__substrdup` in Rust over them. That tied alloc.c's strategies to
  odb.c, whose export would loop back through them once odb.c is ported. All seven now
  link: the five functions directly, and the two `static inline` ones in `alloc.h`, which
  read the hidden `git__allocator`, through `static_fns.c`. Rebind them through their raw
  bindings. The `AllocBox`, `AllocVec` and `AllocStr` strategies call the C, and the
  composition, its documented differences and its dependency on odb.c's exports go. Keep
  its surface, its tests (compare with the C directly now) and its notes that still hold.
- Leave `odb.rs`'s own uses of `git_odb_backend_data_alloc` (`data_alloc`, `str_dup`,
  `OdbDataStrVec::dup_all`) as they are; they are odb.c's batches' to revisit. Report which
  of them could take your strategies.
- Wave 12's batch 0 (`8d14c83d9`, `git_allocator`) is the wrap campaign's. Batch 1's open
  proposal, `hidden-lifecycle-primitives`, is superseded by the static link. Leave it to
  the assistant.
