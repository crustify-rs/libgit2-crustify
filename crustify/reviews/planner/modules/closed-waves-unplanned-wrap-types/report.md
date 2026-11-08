# Nine wrap types were left unplanned when the wip waves of port modules were closed

## What the earlier plan decided

Planner run 4 did not plan an item again when a started wave's batches held it with the
same objective (`crustify/tmp/planner-run-4/py/compose2.py`, `is_held`). Nine structs of
port modules whose layout later modules' C uses were to be `wrap` where the order places
them and `port` once those modules are integrated; their `wrap` was held by batches of wip
waves of the wrap campaign, so run 4 planned their deferred `port` alone, and made the waves
of their consumers depend on the wip wave for the `wrap`:

| type | wip wave (batch) that held its `wrap` | run 4's `port` wave |
|---|---|---:|
| `git_diff_line`, `git_diff_hunk`, `git_diff_perfdata`, `git_diff_binary_file` | 26 (batches 2-5), `src/libgit2/diff.c` | 1528, 1528, 1528, 1522 |
| `git_diff_binary`, `git_diff_similarity_metric` | 37 (batches 3, 1), `src/libgit2/diff.c` | 1508, 1522 |
| `git_diff_find_options` | 54 (batch 0), `src/libgit2/diff.c` | 1508 |
| `git_filter`, `git_filter_options` | 66 (batches 4, 1), `src/libgit2/filter.c` | 1437 |

Commit 0985002ee then closed the 17 wip waves of port modules, at the user's request,
removing their 28 unlaunched batches and the 111 items those held, on the ground that
"Planner run 4 already plans each of those items again, in the new port waves". That holds
for the functions and callbacks among them, which a port module only ports, and for the
types ported where the order places them, but not for these nine, whose `wrap` run 4 had
left to the removed batches.

## Evidence

After 0985002ee no wave of `schedule.json` holds these types with `wrap`, and no batch
landed one; the consumers' waves still name the closed waves for them. `git_diff_line`, for
example, is used by 19 todo waves' items, `src/libgit2/patch.c`'s wave 702 among them, whose
`depends_on.scheduled` lists 26 and 37, closed without the batches that held it:

```sh
cd /target/libgit2-bindings
git show 0985002ee --stat
python3 - <<'PY'
import json, subprocess
s = json.loads(subprocess.check_output(['git', 'show', '0985002ee:crustify/artifacts/schedule.json']))
for w in s['waves']:
    for i in w['items']:
        if i['name'] == 'git_diff_line' and i['kind'] == 'type':
            print(w['index'], w['status'], i['objective'])          # 1528 todo port: no wrap
    for b in w['batches']:
        for i in b['items']:
            if i['name'] == 'git_diff_line':
                print('batch', w['index'], b['status'])              # none
print([w['depends_on'] for w in s['waves'] if w['index'] == 702])
PY
# run 4's v4 input, the schedule at its parent, where wave 26's batch 2 still held it:
git show d0d04711f~1:crustify/artifacts/schedule.json > /tmp/schedule.v4.json
python3 -c "import json; s=json.load(open('/tmp/schedule.v4.json')); print([(w['index'],k,b['status']) for w in s['waves'] for k,b in enumerate(w['batches']) for i in b['items'] if i['name']=='git_diff_line'])"
```

Run 5's composition (`crustify/artifacts/planner/run-5/compose/`, inputs as its `README.md`
gives them) reproduces run 4's 1,325 todo waves exactly from run 4's inputs:

```sh
git show ed81e050c:crustify/artifacts/modules.json > /tmp/modules.run4.json
export CURSCHED=$PWD/crustify/tmp/planner-run-5/schedule.before.json MODULES=/tmp/modules.run4.json
OLDSCHED=/tmp/schedule.v4.json \
  python3 crustify/artifacts/planner/run-5/compose/check_run4.py   # diff waves 0
python3 crustify/artifacts/planner/run-5/compose/check_run4.py     # diff waves 47
```

From the started waves as they stand since 0985002ee, the 47 waves it plans differently all
follow from these nine `wrap` items: the functions, callbacks and port-only types among the
111 are planned again whether a batch held them or not. The scheduling spec's Wrap batches
section requires every unit a batch's items depend on to have landed before it launches;
nothing would land these.

## Edit

`schedule.json`: each type's `wrap` is planned where the order places it, in its module's
wave of that sublayer, and its consumers depend on that wave:

| type | `wrap` wave | replaces |
|---|---:|---:|
| `git_diff_line`, `git_diff_hunk`, `git_diff_perfdata` | 1559 (`src/libgit2/diff.c`, sublayer 3) | 621 |
| `git_diff_binary_file`, `git_diff_similarity_metric` | 1560 (sublayer 4) | 632 |
| `git_diff_binary`, `git_diff_find_options` | 1561 (sublayer 5) | 643 |
| `git_filter`, `git_filter_options` | 1562 (`src/libgit2/filter.c`, sublayer 2) | 842 |

The replaced waves held the same port items without these `wrap`s and are removed. Their
deferred `port` waves stay as run 4 planned them. With the
[port-campaign-objectives](../port-campaign-objectives/report.md) correction, 296 todo
waves keep their items and change their `depends_on` alone.

## Questions for the user

None. Closing the wip waves stands; only what they held that nothing else plans is planned
again.
