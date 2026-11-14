# fixes/gates-green/run-1: the tests that keep the end gates and the full runs from passing

- Repository: your worktree, `/target/libgit2-crustify/crustify/.worktrees/fixes/gates-green`, checked out on `crustify/fixes/gates-green` at `bd127a6d8`,
  the working branch's tip. Its `c/` is the fork at `230c9dbdb`, and `crustify/builds` links to
  the variants built from it (`builds.gen5`).
- Run dir: `crustify/artifacts/fixes/gates-green/run-1/`, with `task.json`. Reviews dir:
  `crustify/reviews/fixes/gates-green/batch-0/`.
- Crustify, for this campaign, is `/crustify-rs/crustify.libgit2-crustify`, the worktree of
  its working branch: read its skills there. Scan with ffimap naming
  `--sys-crates crate::sys,pcre2_sys`.

## Departures from your role, named so you keep them

This is not a scheduled review batch: it has no wave, no commits to review and no worklist
beyond the tests below, and it lands nothing through a wave branch. It is a review batch
with `fix`, whose findings are given. You land by committing on `crustify/fixes/gates-green`
in this worktree, one commit per concern, with the identity `versioning` gives and the
settings' `identity` answer; the assistant fast-forwards the working branch to it. Use no
landing lock, push nothing, and fill no dashboard cells: report what the assistant needs to
record in your final message. Change nothing beyond what the three tasks need.

The gates rule changed on 2026-11-14 (`testing-gates`' Regressions section, and the
settings' `sanitizers` answer): a wave's and a module's end run every stage over the
workspace less the slow list, `crustify/scripts/slow-list.txt`, and a full run, no skip
applied, runs periodically and at ship points. A test BSan cannot finish within its timeout
fails every full run, and is sized for BSan, not skipped: in the configuration BSan's build
sets (`cfg(bsan)`), it runs fewer inputs, chosen to keep reaching every path of the code
it tests, while every other build runs it whole.

## The tasks

The user approved these three on 2026-11-14.

1. **Size the four `indexer.c` tests that never finish under BSan.** They have been skipped
   by name at every module's end since `failalloc.c`'s integration (dashboard Notes), so BSan
   checks none of their paths:
   - `lib`: `libgit2::src::libgit2::indexer::unit_tests::a_break_cancels_the_indexing_with_git_euser`,
     `…::a_panic_cancels_then_resumes_after_the_walk` and
     `…::libgit2_calls_the_closure_with_its_progress` (`src/libgit2/src/libgit2/indexer.rs`);
   - `equiv`: `src::libgit2::indexer::a_progress_callback_sees_and_cancels_as_a_c_callback`
     (`tests/equiv/src/libgit2/indexer.rs`).

   Establish what makes each one not finish, since the cost of BorrowSanitizer grows with a
   buffer's borrow history (`crustify/evolution/bsan-borrow-history-grows`, and
   `src/util/strnlen.h`'s sizing in `wave-225/batch-1` as the precedent: bounds up to 24
   under `cfg(bsan)`, 70 elsewhere). Size each for BSan as the rule above says, and show
   each passes alone under BSan, named and under a `timeout`, with its time. A test that no
   smaller input lets finish is not ignored: say so, with what you tried, and leave it for
   the user. Then take the four names off `crustify/scripts/slow-list.txt`, where the three
   of `lib` are listed.

2. **Fix the two tests that fail under ASan+UBSan and under TSan**, on the working branch's
   tip, which every wave since has recorded as "the known nightly failures": the `ub_safe`
   suite's `compile_fail` test, and the compile-fail doctest of `CommitBuilder` in
   `src/libgit2/src/libgit2/commit.rs`. Establish why each fails under the sanitizer
   variants and passes plain: the variants build on the pinned nightly, and a compile-fail
   expectation whose wording or error code differs between toolchains is the likely cause
   (`testing`'s UB tests section, and
   `crustify/evolution/compile-fail-doctest-codes-unchecked-on-stable`). Make each pass
   under plain, ASan+UBSan and TSan without weakening what it proves: the misuse it rejects
   must still be rejected for the reason it names. If a test cannot be made to pass under
   both toolchains without loosening it, say so and leave it for the user.

3. **Read the three provisional lessons** of `crustify/LESSONS.md`
   (`cmake-property-naming-a-removed-source`, `http-exports-gated-through-a-local-remote`,
   `http-gate-tallies-connections`), added on 2026-11-14 from
   `crustify/evolution/module-switch-procedure`, as `evolution`'s Lessons section has the
   next review batch do: against their evidence, their scope and the generality test. Mark
   each `held`, or correct it, retire it or say which proposal it should have been.

## Gates

Run the gates of your own work: the format check, the lints with warnings denied, the plain
suite over the workspace, ASan+UBSan and TSan over the workspace, Miri on the tests you
change, and BSan on each test you size, alone. Do not run BSan over a binary or the
workspace. Two tests fail under ASan+UBSan and TSan before your change, the two of task 2:
after it, none should. Each gate runs in the background and is waited on.

Eleven other tests are ignored under BSan for their cost alone
(`#[cfg_attr(bsan, ignore = …)]` in `blob`, `object`, `oid`, `trailer`, `tree` and `index`):
they are out of this run's scope. List them in your final message if you meet them, and
change none.
