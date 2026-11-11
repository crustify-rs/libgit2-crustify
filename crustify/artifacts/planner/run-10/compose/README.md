# Planner run 10's composition

Run 10 is run 9's composition again, over the same CodeQL database, T1/T2/T3 tables and
ownership store, with wavefront ca31018 (7c244c2's code, plus docs) and Crustify 6f1e04c.
Its wavefront schedules reproduce run 9's exactly (every field but `oracle_config`'s path),
and `decomp8.py`, `config8.py` and `counters8.py` reproduce the committed `modules.json`
and configuration byte for byte, so neither changes. The scripts are run 9's, keeping their
names, with their paths moved to run 10, but for four, renamed for what they change:

- `stages10.py` (run 9's `stages8.py`) re-derives the later stages under Crustify 6f1e04c's
  rule for the C tests (porting-workflow, Tests follow their modules; conventions,
  Exports): a test case leaves at the integration of a subject whose internals it reaches,
  or, for a subject whose public API alone it uses, with an export of it its code
  references; a test tied to an item's own module through its public API alone holds no
  `port/unexport` back. The verdicts that depend on each other through the tests are
  settled by iteration, from the C referrers alone, until none moves (six rounds).
  Under the scheduling skill's Later stages section, a stage's freeing modules leave out
  the modules ordered before the item's own, which its port already passes; `port/layout`
  and `port/memory` each wait for their own point alone, as run 9's already did.
- `write10.py` (run 9's `write8.py`) numbers the foundational modules' `todo` waves first,
  the type-erased ones before the string one (`STRING_FOUNDATIONAL`), at the user's request
  (`crustify/reviews/planner/modules/foundational-waves-numbered-late/`), and gives the
  started waves' release points the renumbered waves. `FOUNDATIONAL_FIRST=0` numbers as
  run 9 did, for the counts.
- `check10.py` (run 9's `check8.py`, which failed on run 9's own output: a C kind looked up
  as a route, `KeyError: 'struct'`, in `crustify/tmp/planner-run-9/pipeline.log`) also
  checks that each stage wave runs after its own module's last translation wave and that
  the foundational waves come first.
- `test_referrers10.py` (run 9's `test_referrers8.py`) checks the tests' holds under the
  new rule, a test leaving at a module ordered before the item's own counting as gone.

`counts10.py` counts what each change moved, added or removed, against HEAD's schedule.

The inputs go under the ignored `crustify/tmp/planner-run-10/`:

```sh
cd /target/libgit2-bindings
T=/target/libgit2-bindings/crustify/tmp/planner-run-10; mkdir -p $T
C=crustify/artifacts/planner/run-10/compose
python3 -I $C/decomp8.py            # modules.json, as committed
python3 -I $C/config8.py            # the configuration, as committed
python3 -I -c 'import json; c=json.load(open("crustify/wavefront/configs/wavefront-config.json")); c.pop("schedule_seeds"); json.dump(c, open("crustify/tmp/planner-run-10/config-noseeds.json","w"), indent=1)'
wavefront /target/libgit2-bindings/c --config ../crustify/wavefront/configs/wavefront-config.json \
  schedule --output $T/schedule-port.json                       # the plan schedule
wavefront /target/libgit2-bindings/c --config $T/config-noseeds.json \
  schedule --output $T/schedule-deps.json                       # the whole-closure count
python3 -I $C/testfacts8.py $T/schedule-deps.json   # testfacts.json
python3 -I $C/ties8.py              # ties.json, ties-why.json
/opt/venv/bin/python $C/users8.py   # users.json
python3 -I $C/enums8.py             # enums.json
/opt/venv/bin/python -I $C/order8.py  # order.json
python3 -I $C/counters8.py          # modules.json's counters, as committed
python3 -I $C/stages10.py           # the verdicts, for reading
FOUNDATIONAL_FIRST=0 OUT=schedule.v10-run9numbering.json python3 -I $C/write10.py
python3 -I $C/write10.py            # schedule.v10.json, wave-map.json
python3 -I $C/check10.py            # OK
python3 -I $C/test_referrers10.py   # 0 violations
python3 -I $C/counts10.py           # the commit message's counts
cp $T/schedule.v10.json crustify/artifacts/schedule.json
```

`dependencies.json` is written by hand from the crate assessments, as the commit message
describes; no script of this directory produces it.
