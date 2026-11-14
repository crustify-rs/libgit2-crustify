This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/end-gates-lint-stage/review-1/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`be8cb16`), this checkout.

- The proposal, with its evidence and drafted row:
  `/target/libgit2-crustify/crustify/evolution/end-gates-lint-stage/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/crustify diff 23c0f41 3b3316c`, readable in
  `/crustify-rs/crustify-wt/end-gates-lint-stage` (read-only).

It adds a `lint` stage to `end-gates`, the script beside the `testing-gates` skill, with its
tests, and one sentence of the skill; it is stacked on `23c0f41`, another change under review
that it does not depend on. Run `python3 -m unittest test_end_gates` in a `git archive
3b3316c` copy of `skills/verification/testing/gates`, and read the stage's code. No campaign
gate is to be run.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, leave a signal
unfiled, let a check be weakened unseen, state a rule in the target's names, or contradict
another rule. A wording you would sharpen, an edge case no run is likely to meet, and a gap
whose fix would add a rule or a definition are listed apart, as not required, and do not make
the verdict a rejection. Where a required fix can be made by removing text rather than adding
it, say so. Do not reopen the gates rule the user approved on 2026-11-14 (`4557a8f`, landed):
a wave's and a module's end run every stage less a per-stage slow list, and a full run without
skips runs periodically and at ship points. The change touches a protected path: the user
reads your verdict before it lands. Verdict, approved or rejected with the fixes required,
briefly, in your final message. Keep scratch under `/target/libgit2-crustify/crustify/tmp/`,
and remove any build directory you create from outside it, not while it is your working
directory.
