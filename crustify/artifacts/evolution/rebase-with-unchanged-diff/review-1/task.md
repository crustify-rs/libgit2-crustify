This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/rebase-with-unchanged-diff/review-1/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`be8cb16`), this checkout.

- The proposal, with its evidence and drafted row:
  `/target/libgit2-crustify/crustify/evolution/rebase-with-unchanged-diff/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/crustify diff be8cb16 6051f8c`, readable in
  `/crustify-rs/crustify-wt/rebase-with-unchanged-diff` (read-only).

It is one clause of the `evolution` skill's Review section, a rule of the review itself, so
it is marked as weakening a check. Recompute the proposal's patch-id claim for the five
commits it names (`git -C /crustify-rs/crustify show <commit> | git patch-id`; the older
commits are still in the object store), and judge whether the clause's two conditions are
enough for a rebased commit to land unread. No rerun is possible, as the proposal says.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, leave a signal
unfiled, let a check be weakened unseen, state a rule in the target's names, or contradict
another rule. A wording you would sharpen, an edge case no run is likely to meet, and a gap
whose fix would add a rule or a definition are listed apart, as not required, and do not make
the verdict a rejection. Where a required fix can be made by removing text rather than adding
it, say so. The change touches a protected path: the user
reads your verdict before it lands. Verdict, approved or rejected with the fixes required,
briefly, in your final message. Keep scratch under `/target/libgit2-crustify/crustify/tmp/`,
and remove any build directory you create from outside it, not while it is your working
directory.
