This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/config-branches-of-one-result/review-1/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`215a692`), this checkout.

- The proposal, with its evidence and drafted row: `/target/libgit2-crustify/crustify/evolution/config-branches-of-one-result/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/crustify diff 215a692 769d102`, readable in
  `/crustify-rs/crustify-wt/config-branches-of-one-result` (read-only).

It is one paragraph of `porting-symbols`' Macros section. It touches no protected path. Check
it above all against `binding-symbols`' Macros section, whose shim exception it narrows for a
port, and against `dependencies`' Routes and Confirmation sections, which its last sentence
cites: say whether it weakens a check, since a macro ported once is no longer reached through
the C's own expansion, and who establishes that every configuration computes the same.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, leave a signal
unfiled, let a check be weakened unseen, state a rule in the target's names, or contradict
another rule. A wording you would sharpen, an edge case no run is likely to meet, and a gap
whose fix would add a rule or a definition are listed apart, as not required, and do not make
the verdict a rejection. Where a required fix can be made by removing text rather than adding
it, say so. Check the rule against every other skill that speaks of the same thing, since
each first review of the six changes before this one found such a contradiction. No gate or
rerun is to be run: the proposal's validation is the batches that met the case. Verdict,
approved or rejected with the fixes required, briefly, in your final message. Keep any
scratch under `/target/libgit2-crustify/crustify/tmp/`, removed from outside it before you
end.
