# The record holds every signal; lessons are reviewed entries; protected paths

Status: `open`: the change is made on Crustify's
`libgit2-crustify/evolution/evolution-record-and-lessons` (`74f960c`, then `d3eace2` with
review-1's fixes), at the user's direction on 2026-11-14; review-1 rejected the first commit
and review-2 reads the second. It touches protected paths, so it lands only with the user's
approval.

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

Counted on 2026-11-14 by a script over both repositories (each `PROPOSAL.md`'s `Status:`
line, and the date of the commit that added it, `git log --diff-filter=A`), and over the
last cell of each row of Crustify's `EVOLUTION.md`:

- Proposals filed since 2026-11-13 12:00 UTC, the pilot and the day after it: 42 in this
  repository (40 `open`, 2 `landed`) and 12 in glibc-crustify (11 `open`, 1 `landed`).
  This repository holds 84 in all: 42 `landed`, 40 `open`, 2 `declined`.
- Three signals were filed two or three times: `packaging-gate-procedure` and
  `package-gate-procedure` here; `doc-alias-equal-to-the-name` here and
  `doc-alias-equal-to-name` in glibc-crustify; `reproducer-repeats-the-workspace-patches`
  here and `reproducer-carries-the-workspace-patches` there.
- Eight evolution branches of Crustify hold finished changes no review has read
  (`git -C /crustify-rs/crustify branch --list 'libgit2-crustify/evolution/*'`, less this
  change's and `spec-language`'s). Meanwhile three batches renamed items around the alias
  rule and three packaged by hand, by their reports; review-1 did not check those two counts.
- The record holds 50 rows: 47 `watched`, and 3 with no status (`test` or `none`). No
  module's end read them during the pilot. The assistant took three for recurred
  (`wavefront-enum-constant-uses`, the coverage scripts, `ffibox-canonical-path`); review-1
  could not confirm any of the three from the artifacts, and they are not claimed here.
- Lessons: this campaign's `LESSONS.md` holds 8 and glibc-crustify's 4. Each was written by
  a run in its landing commit. Review batches read the file, and four of glibc-crustify's
  eight lesson edits were made by review batches (`6e892ea`, `238650a`, `a5448ee`,
  `2fe3ab2`); what no run was asked for is a review of each lesson against its evidence and
  its scope.
- The goals' soundness measure marks the tier that caught a defect as not taken yet: the
  pilot's one soundness escape, `opt.c`'s dead borrow, was identified by hand from two runs'
  reports.
- `crustify/evolution/evolution-protocol-research/RESEARCH.md`: the survey behind
  recommendations 1, 2, 4, 5 and 6, with its sources. Its recommendation 3, a
  machine-checkable key per row, is not taken: a key catches only an exact repeat, and the
  match is made when a signal is indexed.

## Destination

The spec: `evolution` (Proposals, Lessons, Review, Autonomy, Adoption, Record),
`reviewing` (Review batches, Reports), `agents/common.md`, the assistant's role, and
`EVOLUTION.md`'s opening.

## Change

Crustify `74f960c` and `d3eace2`:

- **The index.** The campaign keeps `crustify/evolution/INDEX.md`, one row per proposal:
  its slug, its signal, its status, the runs that met it, and the record's row it matches.
  It has one writer and holds every proposal before a wave launches. A run reads it in its
  worktree before it files, with the record: a signal either holds is not filed again. The
  run adds its evidence to the indexed proposal when that is in its own repository, and
  otherwise reports the occurrence, which the writer records. The user chose this over
  rows of open signals in `EVOLUTION.md`, whose commits would have moved the working branch
  under every change awaiting its review.
- **The record** stays the record of what landed. A change's row is drafted in its proposal
  and copied in by the record's one writer when the change lands, so that no evolution
  branch edits `EVOLUTION.md`. The record and how evolution evolves are a skill of their own,
  `evolution-record`, the `evolution` skill having outgrown its size.
- **When the watched rows are read.** When a proposal is indexed, and at a period the
  campaign sets from its state, by default after every fifth module it integrates, and
  always at a ship point and an audit's end. The user chose the period over each module's
  end; it weakens that check, and the row says so.
- **A proposal** names the goal it serves and the measure it expects to move, and may be
  `deferred` or `superseded`.
- **A lesson** is an entry under a slug, with a status and a scope line. The first review
  batch that holds its commit reads it, or the next review batch launched when none does;
  that batch marks it `held`, or files what is wrong with it as a finding. An edit changes
  one entry. The lessons are reconciled at each ship point and whenever five have been
  added or changed.
- **A finished change**, one whose proposal names its commit, is reviewed before the next
  wave of its campaign launches.
- **A review report** names the tier that caught its finding and those that passed it. A
  finding a switch, a ship point's gates or a consumer catches is filed by the review batch
  it is handed to.
- **The protected paths** are listed as paths of Crustify's repository: `GOALS.md`;
  `skills/crustify/evolution/**`; `skills/verification/**`; `task-schema`; the Lints section
  of `toolchain`; the Testing, Reviewing and Audit sections of the tool skills;
  `agents/common.md`, `agents/reviewer.md` and the assistant's Evolution section; and the
  provider files that build the reviewer's prompt or launch a run. A change whose diff
  touches one lands only with the user's approval. The repair of a defect that blocks the
  campaign lands once its review finds that it weakens no check, leaves every existing test
  of the check unchanged and adds a test of its own; it goes to the user at the next
  module's end.
- The reviewer's role gains the lessons step; the assistant's role, `agents/common.md`,
  `layout`, the plugin's manifest and both settings templates follow.

Not in this change: the gates rule (`testing-gates`), a change of its own; and the lessons'
slugs and statuses in the two campaigns' files, which the assistant gives them once it
lands.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a campaign indexes its proposals, each filed once and naming its goal and measure; lessons are entries under a slug, read by a review batch and reconciled at ship points; a finished change is reviewed before the next wave; a review report names the tier that caught its finding; the protected paths change only with the user's approval; a change's row is drafted in its proposal; the record is `evolution-record`'s | `libgit2-crustify`, `evolution-record-and-lessons` | crustify `74f960c`, `d3eace2` | a pilot whose runs filed three signals twice, left eight finished changes unreviewed while later runs worked around what they fixed, read the record at no module's end, and wrote lessons no run was asked to review | by a run launched under this change: a signal the index held, filed again under a new slug; a wave launched while a finished change waited for its review; a lesson still `provisional` after the reviews of its wave landed; a review report that names no tier | watched; weakens a check: the watched rows are read at a period, by default every fifth module, not at each module's end; changes what checks the rest |

## Validation

A dry run of the index, which review-1 asked for: a script wrote
`crustify/evolution/INDEX.md` from this repository's 84 proposals, each row with the runs
its proposal cites. Read against it, the read-before-filing rule has something to read from
the day the change lands, and the three duplicated signals sit two rows apart or name each
other's repository. The index is committed as the campaign's, to be kept by the assistant;
glibc-crustify's is written when the change lands.

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

The user chose, on 2026-11-14: the index in the campaign's repository, and a periodic
reading of the watched rows whose period the campaign adjusts. `d3eace2` makes the nine
fixes under those choices.

`review-2` (session `84411ad8-7494-42e9-9696-d871edd1c1a2`, Opus 5.5, `$1.72`, 7m42s), on `d3eace2`: **rejected**. The
user's two choices are carried through, and fixes 1, 6, 7 and 8 of review-1 hold; fixes 2, 3,
4, 5 and 9 hold in part. Its report is the longest `result` of
`crustify/artifacts/evolution/evolution-record-and-lessons/review-2/stream.jsonl`. Required:

- R1. "Files nothing new" where the record holds a signal stops a recurrence of a landed
  change from being filed, an unmarked weakening: it must apply to proposals the index holds
  open alone, a signal that meets a landed row's recurrence signal being filed as a new
  proposal that cites the row.
- R2. A side campaign that shares the working branch reads its own index alone, so two of
  the three duplicates the evidence cites would still not be caught; and the validation's
  claim that the index shows them is wrong.
- R3. The drafted row's lesson clause is met by correct behaviour, a lesson a wave's last
  review batch adds being read in a later wave.
- R4. The protected paths miss the tool skills' safe-boundary scan, `toolchain`'s Unsafe code
  section, the other generated agents, the plugin's manifest and the templates' `evolution`
  question; and "the code whose output a gate reads" cannot be matched against a diff.
- R5. The blocking-defect exception's two test conditions mean nothing for a change to the
  spec.

Two of the recounted figures were still off: glibc-crustify's lesson edits are 4 of 9
commits, and two batches packaged by hand, not three. Not yet made: the fixes wait for the
user's word on R2 and R5.
