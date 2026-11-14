# End gates run every stage less a slow list; full runs periodically

Status: `landed` as crustify `4557a8f` (record `336fdf4`) on `libgit2-crustify/devel`, with the
user's approval on 2026-11-14, which its protected path, `skills/verification/**`, needs.

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
  1, `equiv` 77, of which 60 had finished and 17 were still running, 353 of its 532 tests
  having finished. `ub_unsafe`, which finished, had 3 more. The cost sits in a known set of
  tests, not in the stage.
- The ASan/UBSan and TSan runs each exited 101 on the same two tests, `compile_fail` and a
  compile-fail doctest of `commit.rs`: failures of the tree, not of the gates' cost.

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
| a wave's and a module's end run every stage over the workspace less a per-stage slow list of the tests over a minute; a full run without skips runs every fifth module by default and at each ship point; the list's changes go to the user at ship points | `libgit2-crustify`, `gates-slow-list` | crustify `4557a8f` | two campaigns' settings replaced the end gates with ship-point runs, since BSan over the workspace at every module's end could not finish | a campaign's settings departing from the end gates again; an end's gate that does not finish within its timeout with the slow list applied; or a defect a full run finds in a test the slow list had skipped for more than one period | watched; weakens a check: a test on the slow list is skipped at a wave's end, which ran the workspace's tests and Miri over all of it, and at a module's end, which ran every test under every sanitizer |

## Validation

None beyond the timing run above: no end's gate has run under the rule yet. The first wave's
end after it lands is its validation, and the BSan slow list starts from the timing run's
lists, which are incomplete for `equiv`.

## Review

`review-1` (session `9cafdc80-7a23-49c0-bec1-9d6c4f50e82f`, Opus 5.5, `$1.79`, 6m26s), on `4557a8f`: **approved** as it
stands, with one fix to this proposal's row, made here: the mark says a wave's end is
weakened too. It recomputed the figures and corrected one, also made here. No other text
states the old depths, the provider build is clean, and it judged the missing validation run
acceptable for a rule that moves when existing gates run, on the user's waiver. Listed
apart, not required:

- two names of the timing run's `equiv` list are prefixes of other tests' names, which
  `end-gates` refuses: they leave the starting list;
- `end-gates` cannot skip a test whose name is a prefix of another's, and checks a variant
  stage's skip against one binary while applying it to the workspace: two defects to file;
- libtest reports no time for a test, only that it runs over 60 s, so "with the time
  measured" asks for what an end's run does not produce;
- a failing test can join the list;
- `bsan-sized-tests`, which waits on its own branch, edits the same section with a rule that
  conflicts, a skip as the last resort, and is reconciled before it lands;
- where the skip file sits, who edits it and which setting changes the period are not said.
