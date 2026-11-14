This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/soundness-patterns/review-1/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`be8cb16`), this checkout.

- The proposal, with its evidence and drafted row:
  `/target/libgit2-crustify/crustify/evolution/soundness-patterns/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/crustify diff be8cb16 9ee085d`, readable in
  `/crustify-rs/crustify-wt/soundness-patterns` (read-only).

It adds a skill, `skills/rust/soundness/SKILL.md`, a catalogue of the patterns by which safe
Rust over C reached undefined behaviour in the campaigns, and one sentence of the `reviewing`
skill's Reports section. The user asked for it on 2026-11-14: a skill that documents generic
UB patterns, evolved from experience, with a way to see whether it leads to fewer cases. Do
not reopen that decision, nor ask for more patterns than the findings support.

The patterns come from a classification of three campaigns' review reports, which the
proposal says was made by skimming. Check a sample: for each of at least eight patterns, find
one finding that shows it, under `/target/libgit2-crustify/crustify/reviews/`,
`/target/ffmpeg-crustify/crustify/reviews/` or `/target/glibc-crustify/crustify/reviews/`
(sub-directories `ub_safe`, `ub_unsafe`, `ub_export`, `contracts`, `unsafe`, `misc`), and say
which pattern, if any, you found no finding for. Check that no pattern is stated in a target's
names, that what prevents each agrees with the `binding`, `porting`, `analysis` and `ffibox`
skills, and that the skill is written as `authoring` describes, its description included. No
gate or rerun applies.

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
