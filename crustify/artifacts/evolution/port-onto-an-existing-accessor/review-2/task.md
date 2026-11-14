This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/port-onto-an-existing-accessor/review-2/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`58a4115`), this checkout.

This is a second, narrow review. Review-1 rejected `0b77b97` with three fixes. `50eb634` makes them:
(1) the clause added to `conventions` § 3.2 is removed, so the commit touches
`porting-symbols` alone; (2) the paragraph is scoped to "a function no wrapper gives an API
yet"; (3) of the two ways review-1 gave, the user chose the first on 2026-11-14: the rule is
narrowed to an existing item of the function's own module, and "wherever it is homed" is
gone. Do not reopen that choice. It also takes review-1's "or only returns". Review-1's
verdict and the user's choice are in `/target/libgit2-crustify/crustify/evolution/port-onto-an-existing-accessor/PROPOSAL.md`. The change:
`git -C /crustify-rs/crustify show 50eb634`; what review-1 read: `show 0b77b97`.

Check only that the three fixes are made as review-1 asked, that nothing else changed, and
that the row matches. Do not repeat review-1's other checks, and do not raise again what it
listed as not required. Verdict, approved or rejected, in a few lines, in your final
message. Create no scratch.
