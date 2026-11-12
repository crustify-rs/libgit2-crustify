# Every change is reviewed, recorded and watched

Status: `open`, drafted by the user in another workspace as crustify `ae35f81` on
`review/evolution/reviewed-evolution`.

## Signal

A defect in this guidance (`evolution` §2: "a proposal that proved wrong"): a change landed
on what the run that made it judged right. Validation of a spec change was a judgement, no
second run read a change, and nothing told a later campaign whether a fix held. Systems
that improve themselves against a check they can edit are known to loosen it, and each
role evolves the skills it is judged by.

## Evidence

- The user's direction, and the published record of self-improving agents (the commit's
  own evidence).
- This campaign, 2026-11-12: `ffibox-vtables`'s first change (ffibox `23e0a9b`) passed its
  author's suite, Miri over its 14 new tests, clippy, and a downstream port. An independent
  soundness review, run before landing, found three paths to UB from safe Rust with Miri
  reproducers (handles ignoring the state's thread-safety, a deferred drop on the wrong
  thread, `#[target_feature]` on a trampoline) and three API hazards; its re-review of the
  fix (`041fc6c`) found that the macro dropped the caller's `unsafe` token, hiding the
  obligation from `forbid(unsafe_code)` and clippy. None of it was in the author's report.
- The four changes the record starts with landed this week from a read of the spec, with
  no campaign signal, and nothing yet says whether they held.

## Destination

The spec: the `evolution` skill (Validation, a new Review, Autonomy, Adoption, a new Record
and Evolving evolution), the assistant and reviewer roles, `task-schema` (Evolution
reviews), `layout`, `authoring`, and a new `EVOLUTION.md` at Crustify's root.

## Change

Crustify `ae35f81`.

## Validation

Its own §7 reruns what failed: a review. Its first use is its own review, `review-1/`.

## Review

`review-1` (session `ad57da11-48a5-4608-a27b-bb8c7e2cc7a7`, Opus 5.5, $1.08, 4m44s):
**rejected as it stands**, its direction approved; it weakens no check. Required:

1. When the record is written contradicts itself (`assistant.md:143` "as it lands" against
   `SKILL.md:222` "at each module's end"): add the entry as the change lands, update statuses
   at the module's end.
2. Record commits, and proposal status or verdict edits, are changes §8 would review, without
   end: exempt them.
3. Under `propose` and `off` nothing of Crustify changes, yet the record is committed each
   module's end; and an audit has no module's end. Say who keeps the record under each answer
   and when an audit does.
4. The reviewer ran under the change it reviews (`revisions.crustify` = `ae35f81`, the skills
   loaded from the branch): a review runs under the component's `devel`, reads the change from
   its branch, and its task pins the reviewed `commit`, which alone lands.
5. The user's approval covers "this skill" (`SKILL.md:173`) but not the roles' evolution rules
   or the review's checks, which §12 also changes: extend it to them.

Advisory: the author's task message should not override the review's checks; record the
review's reasons in the proposal, not only its verdict; scope "a regression test, whatever it
changes" to what a tool does (docs and CHANGELOGs have none); say when a tool's opt-in tests
run (wavefront's `WAVEFRONT_CODEQL_TESTS` since `bf21e16`); update the reviewer's routing
description (`compose.py:38-41`); review an unblocking fix before asking the user; `args` is an
array. The seed rows' "from" should name `libgit2-crustify` and their proposals; the record
omits `spec-consistency-2026-11-12`'s commits and the 2026-11-12 landings, and should say
whether it is retroactive; `37683f4` loosened the templates' default autonomy and would now go
to the user. An `accounting` row for evolution reviews is missing. Cost: a review is cheap next
to a batch; scale its depth by what a change touches, and never exempt soundness, unblocking
or check changes.
