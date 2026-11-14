# `end-gates` cannot skip some tests, and checks a variant stage's skip against one binary

Status: `open`. No change is made yet.

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

Not made. One way for both: run a variant stage per test binary, as Miri and BSan already
run, and pass each binary its skips with `--exact` handled per name, so that a skip names one
test of one binary. Each defect's reproducer becomes a test.

## Validation

None yet.

## Review

None yet.
