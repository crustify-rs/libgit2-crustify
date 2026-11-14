This run is not a review batch: it reviews one change to Crustify again, after review-1
rejected the commit it read, as the `evolution` skill's Review section describes, and
changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/spec-language/review-2/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`5933a5d`), this checkout.

- Review-1's report, with its seven required fixes and its optional wordings: the longest
  `result` of
  `/target/libgit2-crustify/crustify/artifacts/evolution/spec-language/review-1/stream.jsonl`.
  The proposal's Review section records it:
  `/target/libgit2-crustify/crustify/evolution/spec-language/PROPOSAL.md`.
- The fix: `git -C /crustify-rs/crustify diff 01830f8 437cc1d`; the whole change:
  `git -C /crustify-rs/crustify diff 5933a5d 437cc1d` (`skills/crustify/authoring/SKILL.md`,
  `GOALS.md`, `EVOLUTION.md`), readable in `/crustify-rs/crustify-wt/spec-language`
  (read-only).

`GOALS.md` is a protected path: the user asked for this change and reads your verdict before
it lands. Check that each required fix of review-1 is resolved (cite the line); that the fix
lost no fact of `GOALS.md` at `5933a5d` and added none (review-1's fact list is in its
report: check the sentences the fix touched against the old text); that the new `GOALS.md`
and the Language section now keep the four firm rules as the fix words them, naming every
place that still breaks one; that the two refinements, as reworded, leave no hole a writer
could use to keep a text hard to read; that `python3 -I providers/claude/compose.py build`
in a `git archive 437cc1d` copy leaves `providers/claude/` unchanged; and whether anything
weakens a check. Read only as deeply as the fix reaches; what review-1 approved need not be
re-derived. Verdict, approved or rejected with the fixes required, briefly, in your final
message, with what you would word differently listed apart. Keep scratch under
`/target/libgit2-crustify/crustify/tmp/`, and remove any build directory you create from
outside it, not while it is your working directory.
