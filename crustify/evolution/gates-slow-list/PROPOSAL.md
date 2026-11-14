# End gates run every stage less a slow list; full runs periodically

Status: `open`: the change is made on Crustify's
`libgit2-crustify/evolution/gates-slow-list` (`4557a8f`), at the user's direction on
2026-11-14, and awaits review-1. It changes `skills/verification/**`, a protected path, so it
lands only with the user's approval.

## Signal

The user's correction, and a rule two campaigns departed from: `testing-gates` ran BSan over
the whole workspace at every module's end, which no campaign could afford, so both campaigns'
settings replaced the end gates wholesale with "only before a ship point".

## Goal

Performance, the harness: cost and wall time per translated item. Its pair is Safety's
measure, the defects a later tier catches: a gate run less often lets more escape, which the
periodic full run bounds.

## Evidence

- The settings of this campaign and of ffmpeg-crustify both depart from the Regressions
  section, in their `sanitizers` answers. Here, from 2026-11-13, BSan runs over the workspace
  only before a ship point. In ffmpeg-crustify, from 2026-11-14, so do the workspace-wide
  ASan/UBSan and TSan runs and the Miri gate.
- A timing run of the end gates on `crustify/devel` at `69c8efa79`, with a 30-minute cap per
  BSan binary (`crustify/tmp/assistant/gates-timing/summary.txt`): ASan/UBSan 160.6 s, TSan
  232.7 s, Miri 40.1 s to 366.6 s per binary, side by side. BSan finished two binaries
  (14.4 s, 192.2 s) and was stopped at 1119.1 s on the other four.
- In those four, the tests that ran over 60 s under BSan
  (`crustify/tmp/assistant/gates-timing/bsan-*.slow.txt`): `lib` 3, `ub_safe` 26, `unit_pub`
  1, `equiv` 77 of the 345 that had finished, of 532. The cost sits in a known set of tests,
  not in the stage.

## Destination

The spec: `testing-gates`' Regressions section; `reviewing`'s Gates section, which restated
it; the translate assistant's workflow.

## Change

Crustify `4557a8f`:

- A wave's end and a module's end, its switch included, run one gate: the workspace plain,
  under every available sanitizer, under Miri and under BSan, less each stage's slow list.
  Before, a wave's end ran the workspace's tests and Miri, and a module's end every test
  under every sanitizer and BSan.
- A full run, every test under every stage with no skips, runs in the background by default
  after every fifth module, and at each ship point with the coverage measurement.
- The slow list is the campaign's skip file, which `end-gates` already reads: per stage, the
  tests that take over a minute under it, by exact name with the time measured. Its changes
  are put to the user at each ship point.
- The assistant starts the full runs and keeps the list. `reviewing` names who runs each
  end's gates and no longer restates what they are.

Once it lands, the two campaigns' settings departures are removed in favour of the rule.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a wave's and a module's end run every stage over the workspace less a per-stage slow list of the tests over a minute; a full run without skips runs every fifth module by default and at each ship point; the list's changes go to the user at ship points | `libgit2-crustify`, `gates-slow-list` | crustify `4557a8f` | two campaigns' settings replaced the end gates with ship-point runs, since BSan over the workspace at every module's end could not finish | a campaign's settings departing from the end gates again; an end's gate that does not finish within its timeout with the slow list applied; or a defect a full run finds in a test the slow list had skipped for more than one period | watched; weakens a check: a test on the slow list is skipped at a module's end, which ran every test before |

## Validation

None beyond the timing run above: no end's gate has run under the rule yet. The first wave's
end after it lands is its validation, and the BSan slow list starts from the timing run's
lists, which are incomplete for `equiv`.

## Review
