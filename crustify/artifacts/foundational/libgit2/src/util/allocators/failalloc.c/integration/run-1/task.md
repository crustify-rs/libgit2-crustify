# failalloc.c integration, run-1

- Repository: the module's worktree,
  `/target/libgit2-crustify/crustify/.worktrees/foundational/libgit2/src/util/allocators/failalloc.c/integration`,
  checked out on `crustify/foundational/libgit2/src/util/allocators/failalloc.c` at
  `018f7da5e`. That is wave 176 promoted: batches 0 to 3, its module pass included.
- Run dir: `crustify/artifacts/foundational/libgit2/src/util/allocators/failalloc.c/integration/run-1/`.

## Departures from your role, named so you keep them

This is not a scheduled batch. You carry out the integration merge of `failalloc.c` into the
working branch, as `versioning`'s Integration section describes, because the merge has a
conflict that is not additive. The assistant hands such a conflict to an agent of the
module's batch kind. You have no items and no worklist. Your translation steps do not apply.
You land by committing the merge on the module's branch in this worktree. The assistant
then fast-forwards the working branch to it. Use no landing lock and push nothing.

## What to do

1. Merge the working branch's tip (`crustify/devel`, at `69ff068e8` or later) into the
   module's branch: `git merge crustify/devel` in this worktree. It conflicts in:
   - `crustify/LESSONS.md`, `crustify/artifacts/dashboard.md` and
     `libgit2-sys/bindgen/allowlist.txt`: additive. Keep both sides. In the dashboard, keep
     every row both sides added, and recompute the `Σ` rows from the merged rows.
   - `libgit2-sys/bindgen/wrapper.h`: additive. Keep both sides' includes.
   - `libgit2-sys/src/bindings.rs` (and `static_fns.c` if it conflicts): do not hand-merge.
     Regenerate from the merged input in campaign mode with `CRUSTIFY_BINDINGS_UPDATE=1`, as
     `bindgen`'s Extending section describes.
   - `src/libgit2/src/util/alloc.rs`: **not additive.**
     - The working branch rewrote the file into an `ffi_import` region with its strategies
       over libgit2's allocator. That came from `alloc.c` wave 12: `c0c753430`,
       `1dc9911d7`, and the review fix `3dbb73d6d`, which set `MAX_ALIGN` to `usize`/`u64`
       alignment.
     - Wave 176 added `Alloc::C_ALLOCATOR: Option<Allocator>` (default `None`), so that
       `Allocator::new::<A>()` returns C's own routines when set. It also added the
       contract wording its review fixed (`alloc-contract-forbids-its-own-c-families`), on
       the old flat layout.
     - Carry wave 176's additions into the region layout, keeping everything the working
       branch has, so `failalloc.rs` compiles against it unchanged or with the minimum
       change.
   - No conflict marker may stay anywhere (Crustify `5de4538`): check with
     `git diff --check` and `git grep -n '^<<<<<<< '` before you commit.
2. The merged tree passes the gates of `testing-gates`' Regressions section:
   - check, fmt (`cargo +nightly-2026-10-08 fmt --all --check`) and clippy over the
     workspace, warnings denied;
   - the whole workspace's tests on `plain` and `asan-ubsan`;
   - Miri over the tests of `util::alloc` and `util::allocators::failalloc`, one run per
     test binary, since the merge combines both modules' unsafe code in `alloc.rs`;
   - TSan over `util::alloc`'s tests.

   Both sides already passed their module and wave gates, so judge further gates by what the
   combination could break, and say what you ran.

   Known failures, not yours:
   - 5 trybuild `$FFIBOX` mismatches (13 under the nightly sanitizers);
   - the nightly `CommitBuilder` compile-fail doctest;
   - `first_discoveries_race_to_pin_the_runtime` outside a git repository;
   - under BSan, four indexer tests that never finish
     (`indexer::unit_tests::{a_break_cancels_the_indexing_with_git_euser,
     a_panic_cancels_then_resumes_after_the_walk, libgit2_calls_the_closure_with_its_progress}`
     and equiv's `src::libgit2::indexer::a_progress_callback_sees_and_cancels_as_a_c_callback`);
   - packaging against an installed libgit2.
3. Commit the merge as `crustify: integrate src/util/allocators/failalloc.c (foundational)`,
   with the identity of the settings' `identity` answer (author Marius Momeu, the single
   `Co-Authored-By: crustify <crustify@berkeley.edu>` trailer). Describe how you resolved
   `alloc.rs` in the message.
4. Measure the API with `ffimap api` from the module branch's tip before the merge
   (`018f7da5e`) to the merged tree, and from the working branch's tip to it, over
   `alloc.c`'s and `failalloc.c`'s items. Report any item that is not `kept` or `extended`.
   Put ffibox's private rustdoc JSON in this worktree's `crustify/tmp/`.

## Host

Other agents run module passes beside you. Cap builds at 16 jobs, keep all scratch in
`crustify/tmp/`, and end only processes of this worktree.

## Report

Report:
- the merge commit, and how you resolved each conflict, `alloc.rs` in detail;
- the gates and their results;
- the API measure.
