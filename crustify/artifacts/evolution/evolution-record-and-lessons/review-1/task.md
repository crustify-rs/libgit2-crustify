This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/evolution-record-and-lessons/review-1/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`5933a5d`), this checkout.

- The change: `git -C /crustify-rs/crustify diff 5933a5d 74f960c`, readable in
  `/crustify-rs/crustify-wt/evolution-record-and-lessons` (read-only): the `evolution` and
  `reviewing` skills, `agents/common.md`, the assistant's role, `EVOLUTION.md`, and the
  provider's regenerated agents.
- Its proposal: `/target/libgit2-crustify/crustify/evolution/evolution-record-and-lessons/PROPOSAL.md`,
  and the research it cites, `.../evolution-protocol-research/RESEARCH.md`. The campaign's
  evidence is in `/target/libgit2-crustify` at `crustify/devel` and `/target/glibc-crustify`.

The change touches protected paths and changes what checks the rest, so the user reads your
verdict before it lands. It writes down what the user and the assistant settled on
2026-11-14: the record as the index of every signal, written by one writer; a proposal
filed once, naming its goal and the measure it expects to move; lessons as reviewed entries
with an id, a status and a scope, reconciled at ship points or after five changes; a
finished change reviewed before the next wave; a review report naming the tier that caught
its finding; protected paths changing only with the user's approval, the repair of a
blocking defect apart. Beyond the Review section's checks, judge:

1. whether each rule can be followed as written by the run it falls to: a stage agent that
   must read the record before filing (where is the record, in a worktree of the campaign's
   repository?), a reviewer asked to read lessons and name tiers, the assistant indexing as
   batches land; name any step no role is given, or that two roles could both think is
   theirs;
2. consistency with the rest of Crustify: every text that still describes the old protocol
   (the module-end check of watched rows, lessons proposed only once another campaign records
   them, `GOALS.md` never changed, three statuses), in skills, agents, templates, the
   `task-schema` and `layout` skills, and the READMEs; and whether the skills name an agent
   where `authoring` says they must not;
3. the protected paths: whether the list is complete for what judges the work (name what is
   missing, such as a file a gate reads), whether it is stated so that a run can tell if its
   diff touches one, and whether the blocking-defect exception can be used to loosen a check;
4. the proposal's evidence against the campaigns' artifacts, figure by figure, saying which
   you could not confirm;
5. the size of the `evolution` skill against `authoring`'s limit for a skill, and, if it must
   be split, at which section;
6. the `EVOLUTION.md` row: whether its recurrence signal is what a campaign would observe,
   and is not met on the day the change lands;
7. that `python3 -I providers/claude/compose.py build` in a `git archive 74f960c` copy
   leaves `providers/claude/` unchanged;
8. whether the change weakens a check.

Verdict, approved or rejected with the fixes required, in your final message, with what you
would word differently listed apart from what must change. Keep scratch under
`/target/libgit2-crustify/crustify/tmp/`, and remove any build directory you create.
