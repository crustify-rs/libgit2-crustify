This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/obligation-spans-what-the-holder-keeps/review-1/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`e8e55af`), this checkout.

- The proposal, with its evidence and drafted row: `/target/libgit2-crustify/crustify/evolution/obligation-spans-what-the-holder-keeps/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/crustify diff e8e55af a3552fb`, readable in
  `/crustify-rs/crustify-wt/sound` at that commit (read-only; `git -C /crustify-rs/crustify show a3552fb:<path>`).

It is one paragraph of `binding`'s Safe boundary section. The proposal also drafts a case for
`reviewing` § 3.2, which this change leaves out: it is a protected path, and the `soundness`
skill's `hidden-dependency` pattern now gives reviewers the case.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, leave a signal
unfiled, let a check be weakened unseen, state a rule in the target's names, or contradict
another rule. A wording you would sharpen, an edge case no run is likely to meet, and a gap
whose fix would add a rule or a definition are listed apart, as not required, and do not make
the verdict a rejection. Where a required fix can be made by removing text rather than adding
it, say so. Check that the rule agrees with the `soundness` skill's patterns, which landed on
2026-11-14, and with `ffibox`'s documentation where it names a primitive. No gate is to be
run, and no rerun: the proposal's validation is the batch that met the defect. Verdict,
approved or rejected with the fixes required, briefly, in your final message. Keep scratch
under `/target/libgit2-crustify/crustify/tmp/`, and create no build directory.
