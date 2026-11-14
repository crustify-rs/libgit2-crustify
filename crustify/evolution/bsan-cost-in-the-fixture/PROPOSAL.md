# Before a test is sized for BSan, what its cost grows with is found

Status: `open`: no change made. The reviewer evolves `testing-gates`, but this run's task
changed nothing beyond its three tasks, so the change waits for its own run.

Goal: Safety, its soundness measure: BSan checks a test's paths with its whole inputs where
the cost was never in them.

## Signal

A rule worked out by hand that a skill should have stated: `testing-gates`' Regressions
section sizes a test BSan cannot finish by giving it "fewer inputs" under `cfg(bsan)`, and
says nothing of finding first where BSan's cost lies.

## Evidence

- `fixes/gates-green/batch-0` (`misc/indexer-fixture-built-byte-by-byte`): four
  `src/libgit2/indexer.c` tests never finished under BSan, skipped by name at every module's end
  since `failalloc.c`'s integration. Their cost was not in their inputs or in the code they
  test: each built its 39,850-byte fixture with `flat_map(…).collect()`, a byte at a time into
  one `Vec`, whose cost under BSan grows faster than quadratically (1.8 s for 1,042 bytes,
  9.5 s for 2,194, 45 s for 4,498), while the indexing itself takes 0.02 to 0.55 s at full
  size. Pushed a line at a time, the same fixture takes 3.3 s, and each test passes whole
  under BSan, with no input removed. Joined with `concat()` instead, each test ran past a
  600 s `timeout`: how a fixture writes its buffer, not only how much, decides the cost. Sized as the rule has it, each would have
  run a smaller pack under BSan, for no gain.
- `wave-225/batch-1` (`crustify/evolution/bsan-sized-tests`' Validation): of `strnlen.h`'s two
  tests, half the remedy was the same kind of thing, a buffer per word offset in place of one
  reused, beside the bound lowered to 24 bytes.

The cause is `crustify/evolution/bsan-borrow-history-grows`: BSan's cost per access to one
allocation grows with the borrows of it already made, so a test's fixture, not only its
inputs, can hold it past its timeout.

## Destination

The spec: `testing-gates`' Regressions section, the paragraph `bsan-sized-tests` added.

## Change

After "Such a test is sized for BSan: …":

> Before it is sized, what its cost grows with is found, by timing its steps alone under BSan:
> a buffer its fixture fills in many small writes, or reuses across cases, is filled in few
> or replaced per case, in every build, and only a cost that remains in the inputs the code
> under test reads is sized.

## Validation

The run that exposed it: under the change, the four tests' fixtures are rebuilt and each
passes whole under BSan (`fixes/gates-green/batch-0`: 5.0 s, 5.1 s, 5.5 s and 11.3 s, alone,
under `timeout 600`), where each ran past its 25-minute timeout before.

## Record

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| before a test is sized for BSan, what its cost grows with is found, and a fixture's cost is removed rather than inputs | `libgit2-crustify`, `bsan-cost-in-the-fixture` | — | four tests skipped under BSan for a fixture built a byte at a time, which sizing their inputs would not have answered | a test sized or skipped under BSan whose cost lies in its fixture, found by a later review | — |

## Review

None yet.
