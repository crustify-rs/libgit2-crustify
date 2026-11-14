This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/equiv-tests-without-c/review-1/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`91ea0cc`), this checkout.

- The proposal, with its evidence and drafted row: `/target/libgit2-crustify/crustify/evolution/equiv-tests-without-c/PROPOSAL.md`. It is one of nine split from
  `module-switch-procedure` on 2026-11-14.
- The change, one commit: `git -C /crustify-rs/crustify diff 91ea0cc 11a7f05`: three sentences of
  `porting-workflow`'s One language per module section.

It touches no protected path. Check it above all against the `testing` skill (Equivalence
tests, and the Miri attribute a test that calls C takes), the `conventions`' Testing section
(where a module's unit tests sit), the switch's parts as `porting-workflow`'s Builds section
now lists them (`10500c9`), and `reviewing`'s test-adequacy objective. Say whether "the marker
the Miri gate puts on a test that calls C" names something the skills define, whether a test
that calls no C and still compares with a C constant through the bindings is a unit test by
the skills' own definition, and who carries the tests over and when. No gate or rerun is to
be run.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, leave a signal
unfiled, let a check be weakened unseen, state a rule in the target's names, or contradict
another rule. The rest is listed apart, as not required. Where a required fix can be made by
removing text, say so. Verdict, approved or rejected with the fixes required, briefly, in
your final message. Create no scratch.
