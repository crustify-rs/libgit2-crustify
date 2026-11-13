# wave-225/batch-1/run-1 (review)

- Repository: your batch's worktree, `/target/libgit2-crustify/crustify/.worktrees/modules/libgit2/src/util/strnlen.h/wave-225/batch-1`, on `crustify/batches/wave-225/batch-1`.
- Worklist: `crustify/artifacts/modules/libgit2/src/util/strnlen.h/wave-225/batch-1/run-1/task.json` in it.
- Batch: `wave-225/batch-1` of the module `libgit2/src/util/strnlen.h` (`modules.json`'s `foundational: false` record of that name).
- Land on the unchecked-out wave branch `crustify/waves/wave-225`. Name it `refs/heads/crustify/waves/wave-225` in every git command.

## Instructions

- No `objectives` are given, so every objective that applies runs, `fix` included (the
  settings' `review-objectives` answer is `all`).
- This is `strnlen.h`'s module pass and its only review: its run landed one translation
  batch, `wave-225/batch-0` (`7ef895eb2`), which ported `p_strnlen` as the native
  `const fn strnlen(bytes: &[u8]) -> usize` over `CStr::from_bytes_until_nul`, with no unsafe
  code and no export. Wave gates: yes. Module end gates: yes, this is the module's last wave.
  The module switches from C at its integration, which the assistant runs after you land.
- The batch ported the macro once, although the header has two configured branches
  (`#define p_strnlen strnlen`, and a `memchr` body under `NO_STRNLEN`, which no variant
  defines), and filed `crustify/evolution/config-branches-of-one-result` on it. Judge that
  against `porting-symbols`' Macros section. It ran neither TSan nor BSan, as the batch adds
  no unsafe code; your module end gates run them.
- This run is a pilot of the harness and runs alone on the host: take up to 48 jobs
  (`CARGO_BUILD_JOBS=48`, and the same for C builds and tests), each BSan and Miri
  invocation under a `timeout`. Keep every scratch file, log and scratch build in your
  worktree's `crustify/tmp/`. End only processes of your own worktree.
- Your run dir in the worktree holds `task.json` and `task.md`; commit them with your
  landing. The process records of your run are kept by the assistant in the base checkout.
- The tree changed on 2026-11-13: the safe crate is `libgit2-crustify` (lib
  `libgit2_crustify`), the raw bindings are `sys/<link-unit>/` reached as
  `crate::sys::libgit2`, stable is Rust 1.100.0, and each variant's C is the static archive
  `crustify/builds/<variant>/crustify/libcrustify.a`. `crustify/builds` in your worktree is a
  link to the base checkout's, which you never rebuild.
- Known failures that are not this module's: under the nightly variants, 8 `compile_fail`
  snapshots blessed on stable mismatch, and the `CommitBuilder` compile-fail doctest (E0624).
- The dashboard's usage cells are the assistant's; fill your landing cells only.
