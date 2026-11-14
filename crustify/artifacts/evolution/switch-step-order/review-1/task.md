This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/switch-step-order/review-1/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`aac2eca`), this checkout.

- The proposal, with its evidence and drafted row: `/target/libgit2-crustify/crustify/evolution/switch-step-order/PROPOSAL.md`. It is one of nine split from
  `module-switch-procedure` on 2026-11-14 (`/target/libgit2-crustify/crustify/evolution/module-switch-procedure/MAP.md`).
- The change, one commit: `git -C /crustify-rs/crustify diff aac2eca 663ed2d`.

It adds an ordered list of a switch's four parts to `porting-workflow`'s Builds section and
removes one sentence, "The shared variants are rebuilt from the C that remains once the
integration lands". It touches no protected path. Check it above all against the other texts
that state a switch's steps: `testing-gates`' Regressions section (a module's end and its
switch), `versioning`'s Integration section, `setup`'s Builds section, `testing-builds`, the
`bindgen` skill's Build modes, and the conventions' Export features section. Say whether the
removed sentence leaves any run without a rule it needs, whether the order contradicts any of
those texts or the assistant's workflow (`agents/assistant/translate.md`), and whether the
list states anything in the target's names. Read the evidence's runs where you doubt a step:
`/target/libgit2-crustify/crustify/artifacts/modules/libgit2/deps/llhttp/api.c/integration/run-1`.
No gate or rerun is to be run: the proposal's validation is the switches that ran this order.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, leave a signal
unfiled, let a check be weakened unseen, state a rule in the target's names, or contradict
another rule. The rest is listed apart, as not required. Where a required fix can be made by
removing text, say so. Verdict, approved or rejected with the fixes required, briefly, in
your final message. Create no scratch.
