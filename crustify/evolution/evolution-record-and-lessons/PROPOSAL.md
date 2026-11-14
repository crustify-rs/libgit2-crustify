# The record holds every signal; lessons are reviewed entries; protected paths

Status: `open`: the change is made on Crustify's
`libgit2-crustify/evolution/evolution-record-and-lessons` (`74f960c`), at the user's direction on
2026-11-14, and awaits its review. It touches protected paths (the `evolution` and
`reviewing` skills), so it lands only with the user's approval.

## Signal

`evolution` §12's own signals, and the user's decisions on them: signals a run met that no
proposal answered in time, proposals that repeat another's signal, a record never checked,
and lessons no review reads.

## Goal

**Maintainability** (goal 6): its measure counts a proposal that repeats another's signal and
a procedure worked out by hand a second time; this change is expected to bring both down.
**Safety** (goal 1): its soundness measure needs the tier that caught a defect, which this
change makes review reports record, so that escapes can be counted. **Performance** (goal 4):
what was redone, which a fix reviewed before the next wave reduces.

## Evidence

- The pilot of 2026-11-13 and 14 (40 runs): 50 proposals filed, 48 left open; three signals
  filed two or three times (`packaging-gate-procedure` and `package-gate-procedure`;
  `doc-alias-equal-to-the-name` here and `doc-alias-equal-to-name` in glibc-crustify;
  `reproducer-repeats-the-workspace-patches` here and
  `reproducer-carries-the-workspace-patches` there); eight finished spec changes waited for
  a review while later batches worked around what they fixed (three batches renamed items
  around the alias rule; three packaged by hand).
- The record's 49 rows were all `watched` at the pilot's end; no module's end read them, and
  at least three had recurred (`wavefront-enum-constant-uses`, the coverage scripts,
  `ffibox-canonical-path` in the side campaign).
- Lessons: this campaign's `LESSONS.md` holds 8 and glibc-crustify's 4, each written by a
  run in its landing commit and read by every later run; four successive runs rewrote the
  side campaign's gates lesson, and no review batch was asked to read any of them.
- The goals' soundness measure (`goal-fitness-functions`) marks the tier that caught a
  defect as not taken yet: the pilot's one soundness escape, `opt.c`'s dead borrow, was
  identified by hand from two runs' reports.
- `crustify/evolution/evolution-protocol-research/RESEARCH.md`: the survey behind
  recommendations 1, 2, 4, 5 and 6, with its sources. Its recommendation 3, a
  machine-checkable key per row, is not taken: the user and the assistant judged it too
  strict, a key catching only an exact repeat; the match is made when a signal is indexed.

## Destination

The spec: `evolution` (Proposals, Lessons, Review, Autonomy, Adoption, Record),
`reviewing` (Review batches, Reports), `agents/common.md`, the assistant's role, and
`EVOLUTION.md`'s opening.

## Change

Crustify `74f960c`:

- **The record** takes a row when a proposal is first filed, with the runs that met its
  signal; its statuses are the proposal's (`open`, `deferred`, `declined`, `superseded`)
  until the change lands, then `watched`, `recurred` or `confirmed`. It has one writer per
  campaign. A new signal is matched against the rows when it is indexed: a repeat adds an
  occurrence, and a match to a landed row marks it `recurred`. The rows still `watched` are
  read at each ship point.
- **A proposal** names the goal it serves and the measure it expects to move, as the user
  asked, and is filed once: a run that meets a recorded signal adds its evidence to that
  proposal.
- **A lesson** is an entry with an id, a status (`provisional`, `held`, `retired`) and a
  scope line. A review batch of its wave reads it; an edit changes one entry; the lessons
  are reconciled at each ship point and whenever five have been added or changed.
- **A finished change** is reviewed before the next wave of its campaign launches.
- **A review report** names the tier that caught its finding and the tiers that passed it;
  a finding caught by a later tier than the first that had it in scope is an escape.
- **The protected paths**, `GOALS.md`, the `evolution`, `reviewing`, `testing`,
  `testing-builds` and `testing-gates` skills with their scripts, the reviewer's role, the
  lints, and a provider's permission settings, change only with the user's approval. The
  repair of a defect that blocks the campaign lands once its review approves it and finds it
  weakens no check, and is put to the user at the next ship point.

Not in this change: the rows of the proposals open today, which the assistant indexes once
it lands; the lessons' ids and statuses in the two campaigns' files, likewise; and the
gates rule (`testing-gates`), a change of its own.

The `evolution` skill grows from 2,938 to 3,395 words, at or past the size `authoring` gives
a skill (about 4,500 tokens): the review is asked whether it must be split, and where.

## Validation

No single failure prompted it. Its rules are stated from what the pilot's runs did by hand:
several added their evidence to an open proposal rather than filing again, which is the
occurrence rule; none could have read the record for open signals, which held none.

## Review

`review-1` (session `013f03c8-ef4c-4210-8ab5-fa86b5551b01`, Opus 5.5, `$1.92`, 7m06s), on `74f960c`: **rejected**, its
direction approved. It weakens a check without marking it: the rows still `watched` would be
read at ship points, not at each module's end. Required:

1. index rows committed on the working branch as batches land move it under every change
   awaiting review, which lands by fast-forward of the reviewed commit alone; and a change
   that drafts its own row is a second writer of the record;
2. a run cannot always read the record or add to a proposal: `agents/common.md` does not
   say where the record is, the proposals open today are not in it, and a run of one
   campaign cannot commit to a proposal of another's repository;
3. the lesson review has no owner: who marks a lesson `held`, what a reviewer does with a
   wrong one, who reads lessons a last review batch or the reconciliation adds; ids need a
   form two batches cannot both take;
4. the protected paths are not stated as paths a run can match its diff against, and miss
   what builds the reviewer's prompt, `spawn.py`'s permission mode, the tools' gate code and
   the record's statuses;
5. the blocking-defect exception leaves a relaxed gate to one reviewer's judgement until a
   ship point: such a repair keeps the check's tests and adds its own, and goes to the user
   at the next module's end;
6. text elsewhere still describes the old protocol (the skill's description, `authoring`'s
   table, the `propose` row, the assistant's role on `GOALS.md`, the settings templates);
7. whether a follow-up change of one proposal takes a row of its own;
8. the skill is past its size: split Record and Evolving evolution as `evolution-record`;
9. the record row's recurrence signal is met on the day it lands.

The proposal's Evidence above is wrong in places, by the review's count: 46 of the 49 rows
are `watched` and 3 have no status; 55 proposals were filed in the pilot's period, 48 open
and 7 landed; the side campaign's gates lesson was added to by four runs, not rewritten;
review batches did read lessons, and what is missing is a review of each lesson; and the
three recurrences are not confirmed from the artifacts. It ran no validation: a dry index of
the open proposals would have shown the read-before-filing rule has nothing to read yet.

The fixes to 1 and to the weakened check are choices of the user's, put to the user on
2026-11-14 before a second commit.
