# Every change is reviewed, recorded and watched

Status: `landed` on Crustify's `libgit2-crustify/devel`, the campaign's working branch, by
fast-forward to `acb7aea` (`ae35f81`, `c2f21a0`, `4c7c40e`, `acb7aea`), after review-4
approved it; Crustify's `devel` takes it only with the user's approval. It is marked as
weakening a check (its record entry, row 51): a reviewed change lands on a working branch
without the user's approval.

## Signal

A defect in this guidance (`evolution` §2: "a proposal that proved wrong"): a change landed
on what the run that made it judged right. Validation of a spec change was a judgement, no
second run read a change, and nothing told a later campaign whether a fix held. Systems
that improve themselves against a check they can edit are known to loosen it, and each
role evolves the skills it is judged by.

## Evidence

- The user's direction.
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

The spec: the `evolution` skill (Changes, Validation, a new Review, Autonomy, Adoption, a new
Record and Evolving evolution), `agents/common.md`, the assistant and reviewer roles,
`task-schema` (Evolution reviews), `layout`, `authoring`, `accounting`, the `ffibox` and
`wavefront` tool skills, the templates and the READMEs, and a new `EVOLUTION.md` at
Crustify's root.

## Change

Crustify `ae35f81` (every change reviewed, recorded and watched), `c2f21a0` (each campaign's
working branch, `devel` the user's gate), `4c7c40e` (review-1's and review-2's fixes,
rebased onto `c2f21a0`) and `acb7aea` (review-3's fixes).

## Validation

Its own §7 reruns what failed, here a review: each round of review reran on the commit
that answered the last. review-1 on `ae35f81` and review-2 on `6d533ec` (before the rebase)
rejected it; review-3 on `4c7c40e` rejected it for five contradictions, each fixed in
`acb7aea`.

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

`review-2` (session `05a34716-5b36-4804-a92e-89094983e9e2`, under crustify `devel` `e2a51c6`,
pinned to `6d533ec` on `libgit2-crustify/evolution/reviewed-evolution`; Opus 5.5, $1.13,
4m35s): **rejected as it stands**, its direction approved; review-1's five required fixes
are made (R3 partly: where the statuses put to the user under `propose` are kept is
unsaid), the record is complete (44 commits of 2026-11-12, 15 entries sampled against
their proposals) and the generated agents match the build. Required:

A. §6 drops the regression test for a packaging defect, which `devel` requires of every
   tool defect: its reproducer becomes a test run against an installed build.
B. The reviewer role (`agents/reviewer.md:45`) reruns validation only "where the proposal's
   record of it is in doubt", less than §8 asks of a tool's code, a check, a soundness fix
   and this skill's rules: make it match.
C. Record entries escape review, though `test`, `none` and `confirmed` end or skip a watch,
   and "in the commit that lands its change" contradicts "only the reviewed commit lands"
   (`6d533ec` itself has no entry): draft the entry in the reviewed change, or its proposal
   for a tool's change; have §8 check its recurrence signal and any `test`; and say how a
   reviewed commit lands (fast-forward, or a cherry-pick that is reviewed again).

Also: the reviewer's shared prompt still lets a task message win over the role; a review's
cost is folded into the `assistant` row; the `wavefront` tool skill does not name
`WAVEFRONT_CODEQL_TESTS`; this proposal still names `ae35f81` and cites no source for the
record of self-improving agents; how deeply a skill change outside the listed kinds is
reviewed is unsaid (documentation has misled agents, `ffibox-usage-docs`); the record's
"(weakens a check)" notes sit in the recurrence column, "no proposal" rows have no format,
and two signals (one of the settled contradictions; scratch outside `crustify/tmp/`) are not
ones a campaign observes. The user's `review/evolution/reviewed-evolution` has moved on to
`c2f21a0` meanwhile.

`review-3` (session `afe6fb64-3eeb-446b-b076-869f54cb1c40`, under crustify
`libgit2-crustify/devel` `e2a51c6`, pinned to `4c7c40e`; Opus 5.5, $1.23, 4m48s): **rejected
as it stands**, its direction approved; the rebase onto `c2f21a0` loses nothing, review-2's
A, B and C are resolved. Required: R1, the checklist's "weakens no check" contradicts
`c2f21a0`'s marking; R2, under `propose` the record's statuses waited on a working branch
that never moves; R3, `ffibox-vtables` landed with no entry; R4, against `devel`'s attended
mode a review replaces the user's approval before a change runs in the campaign, so this
change's entry and verdict must say it weakens a check; R5, a documentation defect's
reproducer must still become a test. Advisory: a task message's precedence in
`agents/common.md`, a review's cost per change, the `wavefront` skill's opt-in tests, two
READMEs calling `devel` the line campaigns run, "working branch" naming, merging two
campaigns' rows, revisiting what landed under a declined weakening, and this campaign's
`unattended` setting. All addressed in `acb7aea`, the setting in the campaign's
`CRUSTIFY.md`.

`review-4` (a `crustify:reviewer` subagent of the assistant's session, under
`libgit2-crustify/devel` `e2a51c6`, narrowed to `4c7c40e..acb7aea` and campaign `0df22bd1e`;
74.6k tokens, 2m26s): **approved**, marked as weakening a check as row 51 says. R1–R5
resolved, the advisories addressed or left by choice, and `providers/claude/` matches
`compose.py build` from `git archive acb7aea`. Advisory, for a follow-up: the `wavefront`
skill's new Evolution section is named by no step of its description and restates the
README; the Autonomy table's `propose` cell says "nothing" though record statuses are
committed; the module's-end consolidation omits the revisits for a declined weakening;
`authoring` still describes `common.md`'s task-message precedence as unconditional; the
accounting rule asks for tokens the Review rows did not give; `templates/docker/Dockerfile:98`
still calls `devel` the line campaigns run; and this campaign's `CRUSTIFY.md` evolution
question still read the old model (fixed with the template's wording).

Review-4's advisories landed as crustify `65cb926` and `10df2c5` on `libgit2-crustify/devel`:
a `crustify:reviewer` subagent rejected `65cb926` for its record entry, which named no signal
for two fixes a campaign can observe, and an ungrammatical sentence of `authoring`, and
approved the correction `10df2c5` (88.8k tokens, 3m14s for both rounds, with `59c454c`'s
review).

