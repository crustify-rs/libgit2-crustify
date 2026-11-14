This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/spec-language/review-4/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`8b52186`), this checkout.

- The proposal, with its drafted row:
  `/target/libgit2-crustify/crustify/evolution/spec-language/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/crustify diff 8b52186 f20edc9`
  (`skills/crustify/authoring/SKILL.md`, `GOALS.md`), readable in
  `/crustify-rs/crustify-wt/spec-language` (read-only).
- Review-3's report on the commit this one replaces, `44186d8`: the longest `result` of
  `/target/libgit2-crustify/crustify/artifacts/evolution/spec-language/review-3/stream.jsonl`.
  It found every fact of `GOALS.md` at `5933a5d` kept at `44186d8`.

What changed since `44186d8`: the user judged the Language section's seven numbered rules
too complicated and replaced them with one paragraph that names a public style guide and
keeps two rules of its own. That is the user's decision: do not reopen it, and do not ask
for definitions or rules to be added back. `GOALS.md` differs from `44186d8` in two
sentences, put back in the order they had at `5933a5d`
(`git -C /crustify-rs/crustify diff 44186d8 f20edc9 -- GOALS.md`). `GOALS.md` is a protected
path: the user reads your verdict before the change lands.

The user's instruction for this round: run every check of your role and of the Review
section, and report all you find. Require a fix only for what would lead a run wrong, lose a
fact of `GOALS.md` at `5933a5d`, let a check be weakened unseen, or contradict another rule.
A wording you would sharpen, an edge case no run is likely to meet, and a gap whose fix
would add a rule or a definition are listed apart, as not required, and do not make the
verdict a rejection.

Check that the two restored sentences and the rest of `GOALS.md` keep every fact of
`5933a5d` (review-3's finding at `44186d8` need not be derived again: read the two-sentence
diff); that the Language section says nothing another section of `authoring` contradicts;
that `python3 -I providers/claude/compose.py build` in a `git archive f20edc9` copy leaves
`providers/claude/` unchanged; and whether anything weakens a check. Verdict, approved or
rejected with the fixes required, briefly, in your final message. Keep scratch under
`/target/libgit2-crustify/crustify/tmp/`, and remove any build directory you create from
outside it, not while it is your working directory.
