# A soundness skill: the UB patterns the campaigns met, grown from their findings

Status: `open`: the change is crustify `9ee085d` on
`libgit2-crustify/evolution/soundness-patterns`, not yet reviewed. It adds one sentence to
`skills/verification/reviewing`, a protected path, so it lands only with the user's approval.

Goal: Safety, its soundness measure: the defects a reproducer proves, by kind. The change
names the kinds, and its measure is the findings of known patterns per module.

## Signal

The user's direction, on 2026-11-14: a skill that documents the generic UB patterns that
occur, evolved from experience, with a way to see whether that leads to fewer cases. And
review findings that recur across batches, in one objective: a rule the how-to skills lack.

## Evidence

A classification of the three campaigns' review reports on 2026-11-14 (libgit2, ffmpeg and
glibc: 479 report files, 143 of them soundness candidates, each skimmed by its title, its
class and the opening of its finding) placed 115 findings in 18 groups, 66 of libgit2, 43 of
ffmpeg and 6 of glibc; four fit none, and six were sanitizer false positives. Ten groups
occurred in two campaigns, such as:

- a failure path of the C that leaves a bad object: 9 findings (libgit2 8, ffmpeg 1);
- a handle that outlives what it depends on: 9 (libgit2 6, ffmpeg 1, glibc 2);
- an alignment or allocator rule vouched for too widely: 8 (libgit2 6, ffmpeg 2);
- memory the C leaves unwritten shown as initialised: 8 (libgit2 2, ffmpeg 6); and
- thread safety assumed from the C's own locking: 6 (libgit2 2, glibc 4).

No skill lists them, so each module met them without the earlier ones' record. The
classification's "caught by" column was partly inferred from the reports' text, and it
counted by skimming: the figures are a seed, not a measure.

## Destination

The spec: a new skill, `soundness`, under `skills/rust/`, and one sentence of `reviewing`'s
Reports section.

## Change

- `skills/rust/soundness/SKILL.md`: fifteen patterns in three tables, each with an id, what
  prevents it and which check caught it; how a finding names its pattern; and how the
  catalogue grows. Three of the 18 groups are left out as not undefined behaviour: leaks on
  a partial failure, unsafe seams left after their safe type landed, which the unsafe
  surface's scan counts, and misstated contracts no pattern names, which keep one closing
  sentence.
- `reviewing`'s Reports section: a finding of `ub` or `contracts` names its pattern, or
  `new`.

It weakens no check.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a `soundness` skill lists the patterns by which safe Rust over C reached undefined behaviour, each from a proven finding, and a `ub` or `contracts` finding names its pattern or `new` | `libgit2-crustify`, `soundness-patterns` | crustify `9ee085d` | the user's direction; 115 soundness findings of three campaigns fell into groups that recurred across modules and campaigns, which no skill listed | a module whose findings of known patterns, against the unsafe sites its reviews read, do not fall below the modules' before it | watched |

## Validation

None by rerun: the skill is read before work no run has done under it yet. Its measure is its
validation, taken at each module's end from the reports' pattern ids.

## Review

None yet.
