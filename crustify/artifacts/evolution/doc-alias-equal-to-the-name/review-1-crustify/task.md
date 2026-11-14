This run is not a review batch: it reviews one change to crustify, as Crustify's `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/doc-alias-equal-to-the-name/review-1-crustify/task.json`.
You run under crustify's working branch of the campaign, this checkout, `/crustify-rs/crustify`.

- The proposal, with its evidence, the user's decision and the drafted row:
  `/target/libgit2-crustify/crustify/evolution/doc-alias-equal-to-the-name/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/crustify diff e6cc1ba a5ae2ea`.

It is one sentence of `conventions`' Anchors section. Its other half is a change to ffimap,
`c3203c7`, reviewed apart. Check that no other skill still asks every stand-in for an alias
without this exception, and judge the known gap the proposal states, the driver's wrapper
counters, filed as `ffimap-wrapper-without-alias`. No gate or rerun applies.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, leave a signal
unfiled, let a check be weakened unseen, state a rule in the target's names, or contradict
another rule. A wording you would sharpen, an edge case no run is likely to meet, and a gap
whose fix would add a rule or a definition are listed apart, as not required, and do not make
the verdict a rejection. Do not reopen the user's decision of 2026-11-14, which the proposal's
Decision section records: the alias is dropped where it would equal the item's name, and
nowhere else. Verdict, approved or rejected with the fixes required, briefly, in your final
message. Keep any scratch under `/target/libgit2-crustify/crustify/tmp/`, and remove it from
outside it before you end.
