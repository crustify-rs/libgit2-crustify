# Coverage figures that mean something in a port

Status: `open`. Filed by the assistant on 2026-11-13, at the user's request, from the
pilot of the first `port` module outside the foundational ones. No change is made yet: the
pilot runs under the rules as they are, and its modules are observed first (Observations,
below).

## Signal

Two of `evolution` §2's: a rule that forces a worse result than its omission would, and a
cost outlier. The whole-crate `cov (C/Rust)` figure every batch measures answered a wrap
campaign's question, how much of the C the safe API's tests exercise. In a port its C half
no longer answers one question, and measuring it per batch costs a share of each run.

## Evidence

- `wave-225/batch-0/run-1` (`7ef895eb2`, `src/util/strnlen.h`, one item): the batch's
  coverage commands took 147 s of its 510 s of tool time, 18 calls between its second and
  its last minute, to measure base and tree over every suite. The figure it recorded,
  `45.02`% C and `98.81`% Rust, is the crate's, which one `const fn` of five lines cannot
  move. What the batch learned about its own work was the worklist's: 5/5 Rust lines, and
  the one C line of `git__strndup` that expands `p_strnlen`, reached by its `equiv` tests.
- The same landing moved `C LoC` from `86833` to `89080`, and the total C coverage with it,
  because the variant's archive gained the CLI's objects on 2026-11-13: the C figure moved
  for a reason unrelated to any test. Every switch will move it again, by removing the
  module's C from the denominator.
- The C the figure counts is of three kinds, which it does not tell apart: C that stays for
  good (the foundational modules, the configured-out scope), C that waits for its port, and,
  after a switch, C that reaches ported Rust through its exports.
- A header module's C has no lines of its own: `p_strnlen` is a macro, counted in its
  callers. Its module's C coverage is nothing to report.
- Two gaps read from the skills, not yet met in a run:
  - `testing-builds` §2.1 sets a module's C coverage under its equivalence tests "against
    what its own C tests reach", but nothing records a module's C-test coverage as that
    baseline, per module.
  - After a switch, the C tests that remain run over the Rust static library, as the
    integration regression gate. `testing-builds` §2 reads Rust coverage "from every test and
    doctest binary of the crate", so what those C tests reach of the ported Rust is not
    credited.

## Destination

The spec: `testing-builds` (Coverage, A worklist's coverage), `accounting` (Landing cells),
`testing-gates` where it times a module's end and a ship point, and the translate
dashboard template's `cov (C/Rust)` columns, `Coverage stats` and Legend.

## Change

Not made. As proposed, for a port campaign:

1. The per-module figures lead. A batch reports its worklist's coverage alone: the Rust it
   added, and, until its module's switch, the reference C its equivalence tests reach
   beside what the C's own tests reach of the same lines.
2. The whole-crate figures are measured at a module's end and at a ship point, not in every
   batch, and the C figure is reported over a denominator that does not move with the
   build: split into the C that stays and the C still to port.
3. After a switch, what the remaining C tests reach of the ported Rust, over the
   integration link, is credited to the module's Rust coverage.

A wrap campaign keeps the figures it has: there the C does not leave, and the whole-crate
C figure keeps its meaning.

Whether 2 weakens a check: coverage is "maximized, not gated" (`testing-builds` §2.1), so
no gate reads the per-batch figure; the review is to say so or mark it.

## Observations

The user asked that the figures' meaning keep being observed. Until the change is made,
each pilot module's end adds a row here, from what its batches and module pass measured
anyway, so that the change is written from more than one module:

| module | Rust of the module (line) | reference C reached by `equiv` | by the C's own tests | whole-crate C / Rust, and what moved it | coverage's share of batch tool time |
|---|---|---|---|---|---|
| `src/util/strnlen.h` | 5/5 | 1 line, in `git__strndup` (a macro: no lines of its own) | no C tests tied to it | `45.02`% / `98.81`%; C moved by the archive's new objects, not by tests | 147 s of 510 s (batch 0) |

What to read from the rows: whether the per-module figures told the batch or its reviewer
something they acted on (a gap filled, a path reported unreachable); whether the
whole-crate figures moved for any reason a test explains; and what measuring them cost.

## Validation

None yet. The change, once written, is validated on the modules observed above: their
batches' figures recomputed under it from the profiles they kept, without rerunning a
suite.

## Record

Drafted for the change, not yet an entry: recurrence signal, a batch or review that
reports a coverage figure it could not act on or explain (a figure that moved with no test
behind it, or a module whose figure is empty), or a `test-adequacy` gap a module pass finds
that the per-module figures had shown covered. Status `watched` from its landing.

## Review

None yet: no change to review.
