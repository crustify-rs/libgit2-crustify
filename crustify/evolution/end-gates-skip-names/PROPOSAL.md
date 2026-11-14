# `end-gates` cannot skip some tests, and checks a variant stage's skip against one binary

Status: `open`: the change is crustify `01064ed` on `libgit2-crustify/evolution/end-gates-skip-names`,
awaiting its review. It changes a script under `skills/verification/**`, a protected path, so
it lands only with the user's approval.

Goal: Safety, its soundness measure: a test the slow list cannot name either holds an end's
gate past its timeout or is left off the list by hand.

## Signal

A tool that reports what the source contradicts: two defects of `end-gates`, the script
beside the `testing-gates` skill, in how it takes the skip file, which the gates rule
(`gates-slow-list`, crustify `4557a8f`) now calls the slow list.

## Evidence

Both were found by `gates-slow-list`'s review-1
(`crustify/artifacts/evolution/gates-slow-list/review-1`), reading `check_skips` and
`variant` of `skills/verification/testing/gates/end-gates` at `4557a8f`:

1. A test whose name is a prefix of another test's cannot be skipped. libtest's `--skip`
   matches by substring, so `check_skips` refuses a skip that would also skip another test,
   and refuses the whole file with it. Two names of the BSan timing run's `equiv` list
   (`crustify/tmp/assistant/gates-timing/bsan-equiv.slow.txt`) are such prefixes: one ending
   `constructors_build_and_fail_as_c`, which also matches its `_over_sha256` sibling, and one
   ending `from_env_opens_match_c`, which also matches its `_unparsable` sibling. A slow test
   so named stays in every end's run.
2. A variant stage's skip (`plain` and each sanitizer but `bsan`) is checked against the one
   binary its line names, then passed to `cargo test --workspace`, which applies it to every
   binary. A name that is unique in its binary can still skip a test of another binary, unseen.
   Before the gates rule only BSan had skips, so this had no effect.

## Destination

A tool: `end-gates` and its tests, `test_end_gates.py`.

## Change

In `end-gates` and `test_end_gates.py`:

1. A run with skips passes libtest `--exact`, so that each skip takes the one test it names,
   and a name that begins another test's is no longer refused. Under `--exact` Miri's
   `compile_fail` skip would be exact too, so for a binary with skips it is passed as the
   names of that binary's tests that hold it.
2. A variant stage still runs the workspace in one invocation, which hands every binary the
   same skips. Before any stage runs, each of its skips is counted over the workspace's
   whole test list, and one that more than one binary holds is refused.

Running a variant stage per test binary, as first sketched here, was not taken: it would
drop the workspace's other packages and its doctests from the stage.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| `end-gates` skips a test by its exact name, and refuses a variant stage's skip that more than one binary of the workspace holds | `libgit2-crustify`, `end-gates-skip-names` | crustify `01064ed` | a test whose name begins another's could not be put on the slow list, and a variant stage's skip was checked against one binary and applied to all | test | |

## Validation

`python3 -m unittest test_end_gates`: 10 tests pass. Two are new: a skip whose name begins
another test's skips it alone, under a variant stage and a per-binary one, and a variant
stage's skip that two binaries hold is refused, where a per-binary stage takes it. The first
replaces the test that such a skip is refused.

## Review

None yet.
