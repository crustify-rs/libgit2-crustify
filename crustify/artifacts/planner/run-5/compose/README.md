# Planner run 5's composition of the todo waves

Run 4's composition (`crustify/tmp/planner-run-4/py/`, not tracked), with its inputs
parameterized and a writer that keeps the started waves as they stand and matches the
re-planned waves to the todo waves they reproduce, so that those keep their indices.

Inputs, under the ignored `crustify/tmp/planner-run-5/`:

```sh
cd /target/libgit2-bindings
mkdir -p crustify/tmp/planner-run-5
# the plan schedule: every module's files seeded, enums scheduled, no caps
wavefront /target/libgit2-bindings/c --config ../crustify/wavefront/configs/wavefront-config.json \
  schedule --with-enums --output /target/libgit2-bindings/crustify/tmp/planner-run-5/schedule-port.json
# the schedule before run 5, and its started waves
git show ed81e050c:crustify/artifacts/schedule.json > crustify/tmp/planner-run-5/schedule.before.json
python3 -c '
import json; s=json.load(open("crustify/tmp/planner-run-5/schedule.before.json"))
s["waves"]=[w for w in s["waves"] if w["status"]!="todo"]
json.dump(s, open("crustify/tmp/planner-run-5/schedule.started.json","w"))'
export CURSCHED=$PWD/crustify/tmp/planner-run-5/schedule.before.json
python3 crustify/artifacts/planner/run-5/compose/write5.py
```

Layout uses: `crustify/artifacts/planner/run-4/layout-uses/layout_uses.csv`, and
wavefront's `t2/fa_with_root.csv`.

- `write5.py` composes the plan from `crustify/artifacts/modules.json`'s objectives and
  writes `crustify/tmp/planner-run-5/schedule.v5.json` and `wave-map.json` (the todo waves
  kept, appended and removed), matching against the todo waves of `CURSCHED`.
- `check_run4.py` reproduces run 4's todo waves: with `MODULES` naming run 4's
  `modules.json` and `OLDSCHED` naming run 4's v4 input,
  `git show d0d04711f~1:crustify/artifacts/schedule.json`, it reports
  `diff waves 0`; with the started waves since 0985002ee (the default `OLDSCHED`) it
  reports the 47 waves the nine unplanned `wrap` types change.
