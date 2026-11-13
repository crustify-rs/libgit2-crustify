# wave-225/batch-0/run-1

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/strnlen.h/wave-225/batch-0`, on `crustify/batches/wave-225/batch-0`.
- Worklist: `crustify/artifacts/modules/libgit2/src/util/strnlen.h/wave-225/batch-0/run-1/task.json` in it.
- Batch: `wave-225/batch-0` of the module `libgit2/src/util/strnlen.h` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-225`. Name it `refs/heads/crustify/waves/wave-225` in every git command.

## Instructions

- This is the campaign's first `port` batch outside the foundational modules, run alone as a
  pilot of the harness: nothing else of the campaign runs on this host, so take up to 48 jobs
  (`CARGO_BUILD_JOBS=48`, and the same for C builds and tests). Keep every scratch file, log
  and scratch build in your worktree's `crustify/tmp/`. End only processes of your own
  worktree.
- Your run dir in the worktree holds `task.json` and `task.md`; commit them with your
  landing. The process records of your run (`session_id`, `stream.jsonl`, `stderr.log`,
  `exit_code`) are kept by the assistant in the base checkout, not in your worktree.
- `src/util/strnlen.h` is a header-only module whose one item, `p_strnlen`, is on this
  build `#define p_strnlen strnlen` (the `GIT_INLINE` body is compiled only under
  `NO_STRNLEN`, which no variant defines). Port what this build defines, and say in its docs
  what the other branch is. Its `modules.json` record names `core` equivalents and the
  `memchr` crate; the `dependencies` skill decides the route of libc's `strnlen`.
- The module's file is scaffolded at `src/libgit2/src/util/strnlen_h.rs` (base `50c93dd23`).
  This is the module's only wave; its module pass follows your landing.
- The tree changed on 2026-11-13: the safe crate is `libgit2-crustify` (lib
  `libgit2_crustify`), the raw bindings are `sys/<link-unit>/` reached as
  `crate::sys::libgit2`, stable is Rust 1.100.0, and each variant's C is the static archive
  `crustify/builds/<variant>/crustify/libcrustify.a`. `crustify/builds` in your worktree is a
  link to the base checkout's, which you never rebuild.
- Known failures that are not yours: under the nightly variants (asan-ubsan), 8 `compile_fail`
  snapshots blessed on stable mismatch, and the `CommitBuilder` compile-fail doctest (E0624).
  The plain suite passes at your base (2759 passed, 1 ignored).
