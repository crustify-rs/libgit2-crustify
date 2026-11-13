# A test BSan cannot finish is sized for it, not skipped

Status: `open`, on crustify's `libgit2-crustify/evolution/bsan-sized-tests` at `6336c30`,
awaiting review.

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

> A test that BSan cannot finish within that `timeout` holds its whole binary past it, so the
> gate cannot pass. Such a test is sized for BSan rather than skipped: in the configuration
> BSan's build sets, it runs fewer inputs, chosen to keep reaching every path it checks, while
> every other build runs it whole. A skip by exact name is kept for a test that no smaller
> input lets finish, and names the module whose batch is to size it.

The record's entry is in the change. It does not weaken a check: a test sized so runs under
BSan where a skip ran nothing, and runs whole everywhere else. A reviewer should still weigh
that a sized test checks fewer inputs under BSan than a test that could run whole would.

## Validation

The run that exposed it, under the change: `wave-225/batch-1` sized the two tests
(a buffer per word offset, and bounds up to 24 bytes under `cfg(bsan)`, past the 16 at which
core's `memchr` scans by words), and each passes alone under BSan in 5.3 s (`unit_pub`) and
4.2 s (`equiv`), where it ran past 5400 s before; plain and the other sanitizers still
sweep to 70 bytes.
