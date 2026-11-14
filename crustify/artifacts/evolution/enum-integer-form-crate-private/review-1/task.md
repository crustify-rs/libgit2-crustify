This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/enum-integer-form-crate-private/review-1/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`4a0e778`), this checkout.

- The proposal, with its evidence, validation and drafted row: `/target/libgit2-crustify/crustify/evolution/enum-integer-form-crate-private/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/crustify show 166d8fd`. It is one of five
  changes stacked on the working branch, each a commit of its own, and its parent is
  `4a0e778`: review this commit's diff alone, not its parents'.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, let a check be
weakened unseen, state a rule in the target's names, or contradict another rule. A wording
you would sharpen, an edge case no run is likely to meet, and a gap whose fix would add a
rule or a definition are listed apart, as not required, and do not make the verdict a
rejection. Where a required fix can be made by removing text rather than adding it, say so.

Check the proposal's evidence against the runs and commits it cites, as deeply as the change
reaches: it is a rule of the spec, so run its validation again where the proposal gives one
that can be rerun cheaply, and say so where it cannot. Verdict, approved or rejected with
the fixes required, briefly, in your final message. Keep scratch under
`/target/libgit2-crustify/crustify/tmp/`, and remove any build directory you create from outside it, not
while it is your working directory.
