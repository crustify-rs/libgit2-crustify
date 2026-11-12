# wavefront's suite runs without CodeQL by default

Status: `landed`, as wavefront `bf21e16` on `devel`, at the user's request on 2026-11-12. Filed
after it landed, as `reviewed-evolution`'s review (`review-1`) asked: it weakens a check, so
under that change's Review section it goes to the user, as it did.

## Signal

Measured waste (`evolution` §2): wavefront's whole suite took 658 s, almost all of it CodeQL,
at 15–35 s per `codeql query run` even on an eight-line fixture. Two test files ran the full
30-query extraction, about 7–8 minutes each, and every change to wavefront, the
`wavefront-enum-constant-uses` fix among them, waited on it.

## Destination

wavefront: its tests and README.

## Change

The two end-to-end fixtures, `enum_constant_uses` and `header_macro_bodies`, commit their
extracted tables (60 CSVs), and their schedule and query tests run on a copy of them. Every
test that runs CodeQL, the six query-against-golden tests and one freshness check per
committed fixture, runs only under `WAVEFRONT_CODEQL_TESTS=1`. `tests/regenerate_tables.py`
re-extracts the tables, and the README's Tests section gives the three commands.

What it weakens: the default suite no longer checks the queries against C, so committed
tables can drift from the queries unnoticed. The opt-in run, or a regeneration and a read of
its diff, must pass before a change to a query or a fixture's C lands, which the README says
and `reviewed-evolution`'s change to §6 requires ("its opt-in tests included wherever the
change touches what they cover"). Rows naming the host's system headers stay in the committed
tables and are not compared, so another host's regeneration differs in those rows only.

## Validation

The default suite: 256 tests in 2.6 s, 9 skipped (the CodeQL tests), rerun by the assistant
(2.06 s). The opt-in suite: 250 tests in 652 s, nothing skipped, before the six table tests
were added; those need no CodeQL and pass in both modes.

## Review

None before it landed, which predates `reviewed-evolution`. Its recurrence signal: a query or
fixture change landing with stale committed tables, or a golden mismatch found later under
`WAVEFRONT_CODEQL_TESTS=1`.
