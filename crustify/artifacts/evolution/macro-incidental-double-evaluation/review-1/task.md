This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/macro-incidental-double-evaluation/review-1/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`36f9c7f`), this checkout.

- The proposal, with its evidence and drafted row: `/target/libgit2-crustify/crustify/evolution/macro-incidental-double-evaluation/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/crustify diff 36f9c7f 4a69dd7`, readable in
  `/crustify-rs/crustify-wt/macro-incidental-double-evaluation` (read-only).

It is three sentences added to the fourth case of `binding-symbols`' Macros list, which
`porting-symbols`' Macros section takes its forms from: the proposal drafted the sentence for
`porting-symbols`, and the change puts it once, where the exception is stated. It touches no
protected path. Say whether it weakens a check: a function evaluates its argument once where
the C evaluated it twice, and the rule rests on every expansion site's argument having no side
effect. Say who establishes that, and what happens when a later expansion site passes one
that has.

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
