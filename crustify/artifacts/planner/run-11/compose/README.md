# Planner run 11's composition

Run 11 is run 10's composition again, over the same CodeQL database, T1/T2/T3 tables and
ownership store, with wavefront ca31018 and Crustify 596b74e. Its wavefront schedules
reproduce run 10's exactly (every field but `oracle_config`'s path), `decomp8.py` and
`config8.py` reproduce the committed decomposition and configuration, and with run 10's
ties `write11.py` reproduces the committed schedule exactly (checked before any change).
The scripts are run 10's, their paths moved to run 11, but for these:

- `empty11.py` (new) lists the test cases whose body the configured build compiles empty
  (95 of 3618; `empty.json`): it preprocesses every test file with the `plain` build's own
  compile command and reads each case's body. Every one has no row in the T2 tables, and
  every case with no row is empty.
- `ties11.py` (run 10's `ties8.py`) leaves them out of scope (Crustify 737aebc): tied to
  no module, a file whose every case is empty tied to none, and an entry that ties a file
  whole names its out-of-scope cases as `excluded`. `areas.py` changes in its docstring
  alone; `counters11.py` (run 10's `counters8.py`) adds the rule to `modules.json`'s
  comment.
- `stages11.py` (run 10's `stages10.py`) re-derives the later stages under Crustify
  596b74e: the C tests hold no stage back, so a stage waits for its `closure` point
  alone, less the item's own module and those ordered before it. An export whose point
  its own port passes, but which C test cases or the harness (`deps/clar/main.c`'s `main`
  and what it reaches) still reference once its module is integrated, is exported for
  them (conventions, Exports) and takes a `port/unexport` freed by that integration
  alone, so that none is kept for good because of a test. A case is gone by then when it
  reaches the internals of the item's module or of one ordered before it. Run 10's
  iteration over the tests' departures, and its test hold, are gone.
- `write11.py` (run 10's `write10.py`) composes the schedule from them; a stage wave
  freed by its own module's integration alone sorts first among its module's.
- `carryover11.py` (new) records the test cases each `port/unexport` wave carries over
  (`carryover.json`), since the schedule schema has no field for them
  (`crustify/evolution/carried-over-test-cases/`): in the plan's order on disk, a case
  leaves at the integration of a port module whose internals it reaches, or with the
  first `port/unexport` that removes an export its code references (its body, its file's
  static helpers, fixtures and file-scope uses, and the helpers of other test files and
  of `deps/clar` it calls). Each wave also lists the harness functions that reference an
  export it removes (`harness`), and, for an export planned for the tests alone, the
  cases the plan's order removes first by an integration or a carry-over the wave does
  not depend on (`if_still_in_c`). It checks that no case that stays references an
  export a `port/unexport` removes.
- `check11.py` (run 10's `check10.py`) and `counts11.py` (run 10's `counts10.py`, HEAD's
  schedule against this run's).
- `deps11.py` (new) writes `dependencies.json` in the current schema from run 10's
  assessments (HEAD's `rust_equivalent` records and commit message) and the external
  records of the schedule's modules.

The inputs go under the ignored `crustify/tmp/planner-run-11/`:

```sh
cd /target/libgit2-bindings
T=/target/libgit2-bindings/crustify/tmp/planner-run-11; mkdir -p $T
C=crustify/artifacts/planner/run-11/compose
python3 -I $C/decomp8.py            # modules.json, as committed but for tests and comment
python3 -I $C/config8.py            # the configuration, as committed
python3 -I -c 'import json; c=json.load(open("crustify/wavefront/configs/wavefront-config.json")); c.pop("schedule_seeds"); json.dump(c, open("crustify/tmp/planner-run-11/config-noseeds.json","w"), indent=1)'
wavefront /target/libgit2-bindings/c --config ../crustify/wavefront/configs/wavefront-config.json \
  schedule --output $T/schedule-port.json                       # the plan schedule
wavefront /target/libgit2-bindings/c --config $T/config-noseeds.json \
  schedule --output $T/schedule-deps.json                       # the whole-closure count
python3 -I $C/empty11.py            # empty.json (about a minute)
python3 -I $C/testfacts8.py $T/schedule-deps.json   # testfacts.json
python3 -I $C/ties11.py             # ties.json, ties-why.json
/opt/venv/bin/python $C/users8.py   # users.json
python3 -I $C/enums8.py             # enums.json
/opt/venv/bin/python -I $C/order8.py  # order.json
python3 -I $C/counters11.py         # modules.json's tests, depends_on and counters
python3 -I $C/stages11.py           # the verdicts, for reading
python3 -I $C/write11.py            # schedule.v11.json, wave-map.json
python3 -I $C/carryover11.py        # carryover.json, fates.json
python3 -I $C/check11.py            # OK
python3 -I $C/counts11.py           # the commit message's counts
python3 -I $C/deps11.py             # dependencies.json
cp $T/schedule.v11.json crustify/artifacts/schedule.json
```
