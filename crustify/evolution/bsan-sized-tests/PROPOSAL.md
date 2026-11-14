# A test BSan cannot finish is sized for it, not skipped

Status: `open`: the change is crustify `c18c8a8` on
`libgit2-crustify/evolution/bsan-sized-tests`, approved by its review. It touches a protected path, so it
lands only with the user's approval.

Goal: Safety, its soundness measure: BSan keeps checking a test's paths.

## Signal

Review findings and gate failures that recur across modules: a rule the how-to skills lack.

## Evidence

- `wave-225/batch-1/run-1` (`src/util/strnlen.h`'s module pass): on the wave tip
  `7ef895eb2` BSan's `unit_pub` and `equiv` binaries timed out at 5400 s, each on one of the
  module's tests (`every_nul_position_in_every_bound`, `every_nul_position_matches_the_c`),
  every other test of both binaries having passed. Translation batches run BSan only on the
  tests they expect to trigger it, so the module end was the first BSan run of either.
  BorrowSanitizer's cost grows with a buffer's borrow history
  (`crustify/evolution/bsan-borrow-history-grows`).
- Four `src/libgit2/indexer.c` tests that never finish under BSan are skipped by exact name
  at every module end since `failalloc.c`'s integration, "until the indexer module sizes
  them for BSan" (dashboard Notes), so BSan checks none of their paths.
- `ub_safe`'s `util::alloc::resized_runs_cross_threads` takes about 35 of `ub_safe`'s 39
  minutes under BSan, with "no `cfg(bsan)` sampling" (dashboard Notes, `alloc.c`'s wave 177
  module pass), and `alloc.rs` already ignores one test under `cfg(bsan)`.

- Eleven tests of `equiv` and `ub_safe` (`blob`, `object`, `oid`, `trailer`, `tree` and
  `index`'s) carry `#[cfg_attr(bsan, ignore = …)]` for their cost alone, "its large inputs
  outlast the BorrowSanitizer gate", "tens of thousands of additions stall BorrowSanitizer",
  so BSan checks none of their paths, where a smaller corpus would keep them running
  (`grep -rn -A3 'cfg_attr($' tests src`, at `095f69596`).

`testing-gates` gives BSan its runs and its `timeout`, but says nothing of a test that cannot
finish within it, so each module end chose by hand between a skip, an ignore and nothing.

## Destination

The spec: `testing-gates`' Regressions section, after its BSan runs.

## Change

> The slow list keeps a slow test out of the ends, not out of a full run, which a test BSan
> cannot finish within that `timeout` fails every time. Such a test is sized for BSan: in the
> configuration BSan's build sets, it runs fewer inputs, chosen to keep reaching every path
> it checks, while every other build runs it whole.

The record's entry is below. Sizing weakens no check: a test sized so runs under
BSan where a skip ran nothing, and runs whole everywhere else. A test no smaller input lets finish keeps failing the full run, which the user sees. A reviewer should still weigh
that a sized test checks fewer inputs under BSan than a test that could run whole would.

## Record

The row for `EVOLUTION.md`, copied in when the change lands. The change was made as crustify
`6336c30`, which also edited `EVOLUTION.md`; on 2026-11-14 it was rebased onto the working
branch as `c18c8a8`, without that edit, since a change's row is drafted in its proposal. It was reworded there to fit `gates-slow-list`, which landed first: the slow list keeps a slow test out of the ends, so this rule is for the test that cannot finish in a full run either, and the skip by name it kept as a last resort is gone.

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a test BSan cannot finish within its timeout is sized for BSan | `libgit2-crustify`, `bsan-sized-tests` | crustify `c18c8a8` | tests BSan could not finish held their binaries past the gate's timeout, and each module's end chose by hand between a skip, an ignore and nothing: four skipped by name, eleven ignored under BSan | a BSan binary timing out in a full run on a test, or a test newly ignored or skipped under BSan for its cost | watched |

## Validation

The run that exposed it, under the change: `wave-225/batch-1` sized the two tests
(a buffer per word offset, and bounds up to 24 bytes under `cfg(bsan)`, past the 16 at which
core's `memchr` scans by words), and each passes alone under BSan in 5.3 s (`unit_pub`) and
4.2 s (`equiv`), where it ran past 5400 s before; plain and the other sanitizers still
sweep to 70 bytes.

## Review

`review-1` (Opus 5.5, `$1.30`), on `23c0f41`: **rejected**, one fix required, by removal: its last
sentence ignored a test under BSan in every run, unseen by the user. Removed, as `c18c8a8`,
so the change weakens no check; the row's signal and recurrence signal were corrected with it,
as the review listed. It did not rerun the validation, on the task's instruction, and read it
from the record. Listed apart, not required: "every path it checks" could say "of the code
it tests"; a full run's BSan timeout is not stated; who sizes a test is not said; the eleven
existing ignores and four skips stay until the campaign sizes them.

`review-2` (Opus 5.5, `$0.26`), on `c18c8a8`: **approved**: the fix is made as review-1 asked and
nothing else changed.

On 2026-11-14 the approved `2309e83` was rebased as `c18c8a8`, its diff unchanged (`git patch-id
--verbatim`), once `gate-findings-outside-the-workset` landed beneath it.
