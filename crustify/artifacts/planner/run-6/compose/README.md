# Planner run 6's composition of schedule.json v6

Run 5's composition (`../../run-5/compose/`), converted to schedule.json v6. Each type's
first stage and every later stage now follow wavefront 2a95e3c's release points, instead
of run 4's own layout query.

Inputs go under the ignored `crustify/tmp/planner-run-6/`:

```sh
cd /target/libgit2-bindings
mkdir -p crustify/tmp/planner-run-6
# the plan schedule: every module's files seeded, enums scheduled, no caps
wavefront /target/libgit2-bindings/c --config ../crustify/wavefront/configs/wavefront-config.json \
  schedule --with-enums --output /target/libgit2-bindings/crustify/tmp/planner-run-6/schedule-port.json
# the whole-closure count (no seeds, no enums), which run 5's depends_on come from
python3 -c 'import json; c=json.load(open("crustify/wavefront/configs/wavefront-config.json")); c.pop("schedule_seeds"); json.dump(c, open("crustify/tmp/planner-run-6/config-noseeds.json","w"), indent=1)'
wavefront /target/libgit2-bindings/c --config /target/libgit2-bindings/crustify/tmp/planner-run-6/config-noseeds.json \
  schedule --output /target/libgit2-bindings/crustify/tmp/planner-run-6/schedule-deps.json
/opt/venv/bin/python crustify/artifacts/planner/run-6/compose/users.py   # users.json
python3 -I crustify/artifacts/planner/run-6/compose/write6.py            # schedule.v6.json, wave-map.json
python3 -I crustify/artifacts/planner/run-6/compose/check6.py            # OK
python3 -I crustify/artifacts/planner/run-6/compose/test_referrers.py    # 0
```

Both schedules are identical to run 5's, apart from the release points that wavefront
2a95e3c adds. That holds for every wave, unit, edge and cut. The CodeQL database and
every T1/T2 table are unchanged; the only new table is `t2/layout_uses.csv`.

- `users.py` writes every unit's whole user lists: layout users closed over embedding,
  raw-site functions and referrers. They are computed as wavefront's schedule computes
  them, and each list's length equals the `users` of its release point, 6479 points in
  all.
- `stages.py` gives each entity of a port module its verdict for each stage it can take:
  `port/layout` and `port/memory` for a struct or union, `port/unexport` for a function or
  global.
  - A stage waits for the modules its point's `closure.modules` names, apart from the
    item's own. With none left, the stage is not planned (`none`).
  - With a user in a foundational module, which stays C, it is never planned (`never`).
  - For `port/unexport`, the C tests count as referrers too: a test file tied to modules
    leaves with the first of them to be integrated, so the stage also waits for one of
    them. A test tied to no port module keeps the export for good.
- `write6.py` composes the schedule:
  - It keeps the started waves, converting their items to v6.
  - It plans each wave's translation items, `wrap` or `port`, as run 5 did. A type's
    first stage is `wrap` when its `port/layout` is planned or never planned.
  - The later stages share a wave of their module for each set of freeing modules. That
    wave comes after the module's translation waves and depends on the last translation
    wave of each freeing module.
  - A todo wave whose items it reproduces keeps its index. For that match, a v5 second
    `port` item counts as `port/layout`.
- `check6.py` checks the v6 invariants:
  - objectives by kind;
  - release points and `field_users` on every planned item;
  - each stage wave's dependencies;
  - the started waves, unchanged apart from the v6 renames and release points.
- `test_referrers.py` counts the `port/unexport` items that a remaining C test still calls
  after the integrations their wave waits for. It must find none.
