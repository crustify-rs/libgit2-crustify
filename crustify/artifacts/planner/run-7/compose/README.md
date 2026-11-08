# Planner run 7's composition

Run 7 re-derives each module's `tests` under the decomposition's current Bundling rule,
and the `port/unexport` stages that those tests hold back. It also numbers the `todo`
waves compactly, as the schedule schema's Indices section now asks. Both of this run's
wavefront schedules (the plan's and the whole-closure count) are identical to run 6's.
The CodeQL database and the T1/T2/T3 tables are unchanged, and so is every wave's set of
translation items.

The inputs go under the ignored `crustify/tmp/planner-run-7/`:

```sh
cd /target/libgit2-bindings
T=/target/libgit2-bindings/crustify/tmp/planner-run-7; mkdir -p $T
python3 -I -c 'import json; c=json.load(open("crustify/wavefront/configs/wavefront-config.json")); c.pop("schedule_seeds"); json.dump(c, open("crustify/tmp/planner-run-7/config-noseeds.json","w"), indent=1)'
wavefront /target/libgit2-bindings/c --config ../crustify/wavefront/configs/wavefront-config.json \
  schedule --with-enums --output $T/schedule-port.json         # the plan schedule
wavefront /target/libgit2-bindings/c --config $T/config-noseeds.json \
  schedule --output $T/schedule-deps.json                      # the whole-closure count
C=crustify/artifacts/planner/run-7/compose
python3 -I $C/testfacts.py          # testfacts.json: every test function's references and internals uses
python3 -I $C/ties.py               # ties.json: each module's tests (areas.py: each suite's subject)
python3 -I $C/write_tests.py        # modules.json's tests, over 547ab887f's modules.json
/opt/venv/bin/python $C/users.py    # users.json, identical to run 6's
python3 -I $C/write7.py             # schedule.v7.json, wave-map.json
python3 -I $C/check7.py             # OK
python3 -I $C/test_referrers7.py    # 0 violations
```

- `testfacts.py` reads the C tests' facts from the campaign's tables, which trace
  `tests/` as well as the closure. For every function of a test file it records the
  closure units it calls, takes the address of or accesses, and its calls to other test
  functions. It also records its uses of a module's internals: fields and layout of the
  types the module owns, from `t2/field_accesses.csv` and `t2/layout_uses.csv`, macro
  expansions included, and the module's static functions and static globals. Enums are
  no units, so using one ties nothing.
- `areas.py` holds the judgement the Bundling rule asks for: the modules whose results
  each test suite's assertions check. libgit2 names every clar suite for its subject.
  Everything else a test calls sets up or tears down its fixtures, or reads the result.
  Its docstring gives the rule, and each entry gives its suite's subject.
- `ties.py` ties each clar test case to the modules whose internals its code uses, its
  file's static helpers and its suite's `__initialize`/`__cleanup` included. It also ties
  the case to the area modules its code reaches, directly or through test helpers, and to
  the file's `ALWAYS` modules. A test case that reaches no area module is tied to the
  area modules of its file's other cases. A helper is tied by its own internals uses. A
  file is split by function only where its test cases divide. `deps/clar`, the harness,
  is no test.
- `stages7.py` is run 6's `stages.py` with the tests read anew. Tests count as referrers
  of every function and global. A test case leaves with the first module it is tied to
  to be integrated. A test file's other functions leave with its last test case, and a
  helper file with the last test that reaches it. `deps/clar` never leaves.
- `write7.py` is run 6's `write6.py` with `stages7.py`'s verdicts. The `todo` waves are
  numbered from 169, after the highest started index, in one topological order: the
  translation waves in wavefront's order, and each later-stage wave right after the last
  wave it depends on.
