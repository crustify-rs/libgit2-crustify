This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/rebase-with-unchanged-diff/review-3/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`be8cb16`), this checkout.

This is a third, narrow review. Review-2 rejected `602c028` with one required fix: the clause's
"a change to the spec" also covered programs. `e64f119` limits it to "the text of skills, agents
or templates alone, no script or program among its files", and changes nothing else. Review-2's
verdict is in the proposal's Review section,
`/target/libgit2-crustify/crustify/evolution/rebase-with-unchanged-diff/PROPOSAL.md`.
The change: `git -C /crustify-rs/crustify diff be8cb16 e64f119`; from what review-2 read:
`git -C /crustify-rs/crustify diff 602c028 e64f119`.

Check only that this fix is made as review-2 asked, that nothing else changed, and that the
proposal's row matches. Do not repeat the earlier reviews' checks, and do not raise again
what they listed as not required. Require a fix only for what would lead a run wrong, let a
check be weakened unseen, or contradict another rule. Verdict, approved or rejected, in a few
lines, in your final message. Create no build directory.
