# fixes/ffibox-audit/run-1 (two fixes from the ffibox usage audit)

- Repository: your worktree, `/target/libgit2-crustify/crustify/.worktrees/fixes/ffibox-audit`, checked out on `crustify/fixes/ffibox-audit` at `fd0120c5e` (the working branch's tip).
- Run dir: `crustify/artifacts/fixes/ffibox-audit/run-1/`, with `task.json` naming the two files.

## Departures from your role, named so you keep them

This is not a scheduled batch: it has no wave, no items and no worklist, and your
translation steps apply only as far as the two fixes below need them. It touches two
modules, both already integrated: `src/libgit2/config.c` (wrap) and the foundational
`deps/reftable/basics.c`. You land by committing on `crustify/fixes/ffibox-audit` in
this worktree, one commit per fix, with the identity `versioning` gives; the assistant
fast-forwards the working branch to it. Use no landing lock, push nothing, fill no
dashboard cells (the assistant records the run), and change nothing beyond the two fixes
and their tests and docs.

## The fixes

The user approved both after an audit of how the campaign's Rust uses ffibox.

1. **`config::BackendFree::c_drop` panics on a NULL `free` slot**
   (`src/libgit2/src/libgit2/config.rs:3789`). It calls
   `.expect("backend lacks free callback")` inside a releaser, so a backend whose `free`
   member is NULL panics in `Drop`. That aborts when it is reached from C or while
   unwinding. `odb::OdbBackendFree` (`odb.rs:3381`) leaves such a backend alone instead.
   - Establish whether any safe path can reach a NULL `free`: `CustomBackend`,
     `UnopenedBackend::new`, snapshots, and `from_c`'s stated contract.
   - Bring the releaser to the odb's behaviour: a backend without `free` is left alone,
     not called through and not panicked on. Keep the safety comments and rustdoc
     consistent with it, including `BackendBox`'s "NULL slots other than free are
     permitted" paragraph.
   - Add a test that reaches the case through whatever path can reach it: the public API
     if one can, otherwise an `unsafe` adoption whose contract allows it.
   - If you find the contract already excludes a NULL `free` on every path, keep the
     non-panicking releaser anyway, since a releaser never panics, and say so in the
     commit message.

2. **`basics.rs`'s reftable allocator hooks abort on a panic**
   (`src/libgit2/deps/reftable/basics.rs`, around `malloc_with` / `realloc_with` /
   `free_with`, about :1574–1602). `alloc.rs`'s hooks for libgit2's allocator
   (`src/libgit2/src/util/alloc.rs:517–580`) put `malloc` and `realloc` behind
   `ffibox::callback::barrier`, so a panic returns NULL, C's out-of-memory convention, and
   `realloc` leaves `ptr` live. Their `free` aborts, since C's `free` cannot fail.
   Both modules are foundational and stay for good, so they should agree.
   - Put reftable's `malloc` and `realloc` hooks behind the barrier the same way.
   - Update the `Alloc` trait's rustdoc ("a panic in one aborts the process") to say
     what now happens per hook.
   - Add tests: a panicking `malloc` and a panicking `realloc` each yield a NULL
     result, with `realloc`'s `ptr` still live and released afterwards. Use the
     module's existing test placement.
   - If `barrier` records the panic somewhere a caller should resume or inspect, follow
     what `alloc.rs` does.

## Gates

Run the scoped gates `testing-gates` gives a batch for the two modules' test binaries:
lib, ub_safe, ub_unsafe and equiv, with each sanitizer that applies, and Miri on the
tests you add. Run them once on the base before any change and once on your commits.
Also run clippy and `cargo +nightly-2026-10-08 fmt --all --check`, the formatting gate.

## Instructions

- Use `/target/libgit2-crustify/crustify/.worktrees/fixes/ffibox-audit/crustify/tmp/` for every scratch file, log and scratch build. Cap builds at 16
  jobs (`CARGO_BUILD_JOBS=16`). End only processes of your own worktree.
- These failures are not yours: 5 trybuild mismatches in `ub_safe::compile_fail` that
  print `/opt/venv/share/ffibox/...` for `$FFIBOX`; under the nightly, the
  `CommitBuilder` compile-fail doctest (E0624); and
  `first_discoveries_race_to_pin_the_runtime` in a tree that is not a git repository.
- In your final message, give each commit's sha and the gate results before and after.
