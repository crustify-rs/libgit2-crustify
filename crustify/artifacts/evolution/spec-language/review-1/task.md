This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/spec-language/review-1/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`5933a5d`), this checkout.

- The change: `git -C /crustify-rs/crustify diff 5933a5d 01830f8`, readable in
  `/crustify-rs/crustify-wt/spec-language` (read-only): a new Language section in
  `skills/crustify/authoring/SKILL.md`, `GOALS.md` rewritten under it, and a row of
  `EVOLUTION.md`. `git show 01830f8` is the second commit alone.
- Its proposal: `/target/libgit2-crustify/crustify/evolution/spec-language/PROPOSAL.md`.

`GOALS.md` is a protected path: the user asked for this change and reads your verdict
before it lands. The rewrite must lose nothing and add nothing. Beyond the Review section's
checks, do this:

1. **Every fact kept.** Make your own list of the distinct facts of `GOALS.md` at `5933a5d`
   (its opening paragraphs; each goal's statement; each measure; each pair and the reason
   given for it; every "not taken yet" marker; both TODO comments; goal 11's weighing
   against Least constraint; goal 8's order and what it says of Safety and Performance), and
   check each against the new text, citing the line. Report every fact that is missing,
   weakened, strengthened or moved under another goal, and every statement of the new text
   that the old one did not make. One addition is known: "A *tier* is one of the checks that
   code meets one after another."
2. **The nine settled sentences** (`git show 01830f8 -- GOALS.md`): the goals' author chose
   one reading of each sentence the rewrite had kept verbatim. Say for each whether the old
   text allowed that reading, and whether another reading was at least as likely.
3. **The rules against their own text**: does the Language section, and does the new
   `GOALS.md`, keep the four firm rules; where does either depart from a default, and does
   the departure keep information the default would have lost?
4. **The two refinements** of rules 2 and 3 in `01830f8`: are they needed, by the cases in
   `GOALS.md`, and do they open a hole a writer could use to keep a text hard to read?
5. Placement and consistency: the section in `authoring`, its numbering, the skill's size
   against its own limit, and whether any other text of Crustify contradicts the rules as
   rules (not whether old text follows them: it is brought to them when touched).
6. The `EVOLUTION.md` row's recurrence signal, and whether the change weakens a check.
7. That `python3 -I providers/claude/compose.py build` in a `git archive 01830f8` copy
   leaves `providers/claude/` unchanged.

Verdict, approved or rejected with the fixes required, in your final message, with what you
would word differently listed apart from what must change. Keep scratch under
`/target/libgit2-crustify/crustify/tmp/`, and remove any build directory you create.
