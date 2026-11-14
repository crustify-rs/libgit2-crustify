This run is not a review batch: it reviews one change to Crustify a third time, after
review-2 rejected the commit it read, as the `evolution` skill's Review section describes,
and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/spec-language/review-3/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`5933a5d`), this checkout.

- Review-2's report, with its six required fixes (A to F) and its optional wordings: the
  longest `result` of
  `/target/libgit2-crustify/crustify/artifacts/evolution/spec-language/review-2/stream.jsonl`.
  The proposal: `/target/libgit2-crustify/crustify/evolution/spec-language/PROPOSAL.md`.
- The fix: `git -C /crustify-rs/crustify diff 437cc1d 44186d8`; the whole change:
  `git -C /crustify-rs/crustify diff 5933a5d 44186d8` (`skills/crustify/authoring/SKILL.md`,
  `GOALS.md`, `EVOLUTION.md`), readable in `/crustify-rs/crustify-wt/spec-language`
  (read-only).

`GOALS.md` is a protected path: the user asked for this change and reads your verdict before
it lands. Check that each required fix of review-2 is resolved (cite the line). The fix
decided what review-2 left open: a clause is what has a finite verb, so an infinitive or a
participle makes none; a clause of time is a condition only when it says what happens first;
the scope is a list of examples, and review reports are no longer named in it. Judge whether
each decision leaves a hole a writer could use to keep a text hard to read, and name every
place in the Language section and in `GOALS.md` that breaks a firm rule as now worded.
Check that the sentences the fix touched lost no fact of `GOALS.md` at `5933a5d` and added
none; that `python3 -I providers/claude/compose.py build` in a `git archive 44186d8` copy
leaves `providers/claude/` unchanged; and whether anything weakens a check. Read only as
deeply as the fix reaches; what the earlier reviews approved need not be derived again.
Verdict, approved or rejected with the fixes required, briefly, in your final message, with
what you would word differently listed apart. Keep scratch under
`/target/libgit2-crustify/crustify/tmp/`, and remove any build directory you create from
outside it, not while it is your working directory.
