# A module pass seeds faults in the ported code before the switch retires its equivalence tests

Status: `open`.

## Signal

A landed change that did not hold. The record's row of `equiv-tests-without-c` (crustify
`7da3744`) watches for "a review or test-adequacy finding of behaviour only a retired equivalence
test checked". This pass found such behaviour before the retirement, and only by seeding faults:
`porting-workflow` keeps what the retiring equivalence tests established "by its unit tests", and
`reviewing`'s Test adequacy objective names "assertions too weak to fail when the code is wrong",
but neither says how a module pass tells, and coverage, the one measure the skills give, does
not: a test runs a line whether or not it asserts what the line computes.

It is also a measure `GOALS.md` names and no stage takes: Verification's pair, "the share of
deliberate faults that the ported Rust's tests catch", **Not taken yet** ("the seeded faults,
which are taken only where a review happens to seed them"), with the TODO "seeding faults is not
a step of any stage yet".

## Evidence

- `wave-221/batch-3/run-1`, `src/util/hash/sha1dc/ubc_check.c`'s module pass. `ubc_check`, a
  statement-for-statement port of 470 lines of generated conditions, was covered to 100% of its
  lines and regions (`wave-221/batch-1`'s figures, over every suite), and is so by its unit
  tests alone on this pass's tree (`lib` and `unit_pub` merged: `ubc_check.rs`'s 557 lines, 15
  functions and 891 regions, `crustify/scripts/coverage/run.sh`); the base's unit tests were not
  measured alone. The pass seeded 558 faults in it, one per
  site: each word index moved by one (312), each bit of a `1 << k` moved by one (118), each
  `== 0`/`!= 0` inverted (128), each built as its own binary with the function's unit-test
  checks, and each compared with the unmutated function over 2^17 seeded blocks and every one-bit
  change of a block selecting each vector, which told all 558 apart from the original. The
  module's `unit_pub` tests caught 406 and let 152 through (27%); its `equiv` tests, which compare
  with the C and retire at the switch, caught the 8 hand-made mutants of the same kinds that the
  pass first built in the crate, the one `unit_pub` missed among them. Two `unit_pub` tests the
  pass added (`ubc_check_of_each_one_bit_change_of_a_witness_is_cs` and
  `ubc_check_of_many_seeded_blocks_folds_as_c_does`, their expected values computed by the C)
  catch all 558.
- The same pass seeded one fault in the gateway, an export that writes a zero mask: the gateway's
  own test, the only test of the export that stays after the switch, passed it, every one of its
  35 blocks giving a zero mask; the relinked `util_tests`' `sha1::detect_collision_attack` failed
  it.
- `wave-197/batch-1/run-1` (`deps/llhttp/http.c`) and `wave-194/batch-7/run-1`
  (`deps/llhttp/api.c`), the module passes before those switches, mapped each retiring
  equivalence test to the unit tests that keep it by reading (http.c's run dir's
  `equiv-retirement.md`), and added unit tests where the reading found none; neither seeded a
  fault, so neither could tell an assertion too weak to fail.

## Goal

Verification: its pair, the share of seeded faults the ported Rust's tests catch, taken at the
tier where it matters most, the last before the comparison with the C ends. And Equivalence:
"the differences found after the comparison ended", which a fault the unit tests let through
becomes once a later change makes it.

## Destination

The spec, both texts protected (`skills/verification/**`), so the change waits for the user:

- `reviewing`, Test adequacy (`test-adequacy`): in a module pass before a switch, the ported
  code's faults are seeded, and the share the tests that outlive the C catch is measured, with the
  module's equivalence tests telling a fault apart from an equivalent rewrite; a gap is filled with
  tests, as now, until they catch what the equivalence tests catch.
- `testing-builds`, Coverage: the share beside the coverage figures, as the measure that coverage
  is paired with.

A script beside `testing-builds` that seeds a function's faults by kind and runs a test filter
over each would make the step deterministic; this pass's was a scratch harness over a copy of the
function, which is not one.

## Change

Not made. One paragraph in `reviewing`'s Test adequacy: "A module pass before its module's switch
seeds faults in the module's ported functions, one per site of a kind it names (an index, a
constant, a comparison or an operator changed), and runs over each the tests that outlive the C,
its unit and UB tests and its gateway's, with the module's equivalence tests run over the same
faults to set aside those that change nothing. The faults the equivalence tests catch and the
others let through are gaps, filled as this objective fills any. The pass records the faults
seeded and the share caught, before and after." And one line in `testing-builds`' Coverage naming
that share as the figure coverage is read beside.

## Record

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a module pass seeds faults in the ported code before its switch, and fills the gaps the unit tests leave against the retiring equivalence tests | `libgit2-crustify`, `module-pass-seeds-faults-before-the-switch` | — | a module whose unit tests ran every line of its port let 27% of its seeded faults through, which only its equivalence tests, retired at the switch, caught | a finding, after a switch, of behaviour that a fault seeded at the module pass would have shown and the unit tests do not check | watched |

## Validation

The pass above is the run that showed the gap; the change's rerun is the same seeding on another
module's pass under the rule, recording the share before and after its gaps are filled.

## Review

Pending.
