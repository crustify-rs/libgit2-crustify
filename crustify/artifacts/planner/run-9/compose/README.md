# Planner run 9's composition

Run 9 is run 8's composition again, run by the assistant in its own session at the user's
request, over wavefront 7c244c2, which makes a unit of every macro a published header
defines, whether or not the closure expands it, and leaves include guards out of the API.
The CodeQL database is unchanged; of the T1/T2 tables only `t1/macros.csv` was extracted
again, with 7c244c2's `macros.ql` (its new `include_guard` column); the T3 tables and the
ownership store are unchanged. The scripts are run 8's, keeping their names, with their
paths moved to run 9, and two changes:

- `write8.py` also gives the started waves' items, and their batches' items, their release
  points afresh from this schedule, at the user's request: run 8 had left them as run 7
  computed them, with the route kinds and run 7's wave numbers. Nothing else of the started
  waves changes.
- `check8.py` checks the started waves against run 8's schedule, so it allows the C kinds
  they already carry, and leaves their release points out of the comparison.

`decomp8.py`, `config8.py` and `counters8.py` reproduce run 8's `modules.json` and
configuration byte for byte: the 19 macros the new units add are claimed by the modules
whose file entries already select them. The 19 macros are added as `port` items to their
modules' existing waves; no wave is added, removed or renumbered.

The inputs go under the ignored `crustify/tmp/planner-run-9/`:

```sh
cd /target/libgit2-bindings
T=/target/libgit2-bindings/crustify/tmp/planner-run-9; mkdir -p $T
C=crustify/artifacts/planner/run-9/compose
python3 -I $C/decomp8.py            # modules.json: run 7's, with run 8's placements and new header modules
python3 -I $C/config8.py            # the configuration, from modules.json (--check: reproduces run 7's but for kinds)
python3 -I -c 'import json; c=json.load(open("crustify/wavefront/configs/wavefront-config.json")); c.pop("schedule_seeds"); json.dump(c, open("crustify/tmp/planner-run-9/config-noseeds.json","w"), indent=1)'
wavefront /target/libgit2-bindings/c --config ../crustify/wavefront/configs/wavefront-config.json \
  schedule --output $T/schedule-port.json                       # the plan schedule
wavefront /target/libgit2-bindings/c --config $T/config-noseeds.json \
  schedule --output $T/schedule-deps.json                       # the whole-closure count
python3 -I $C/testfacts8.py $T/schedule-deps.json   # testfacts.json
python3 -I $C/ties8.py              # ties.json: each module's tests (areas.py: each suite's subject)
/opt/venv/bin/python $C/users8.py   # users.json: every unit's whole user lists, as wavefront's release points count them
python3 -I $C/enums8.py             # enums.json: closed and flags enums
/opt/venv/bin/python -I $C/order8.py  # order.json: the order of the modules
python3 -I $C/counters8.py          # modules.json's tests, depends_on, files and function_pointers
python3 -I $C/write8.py             # schedule.v8.json, wave-map.json
python3 -I $C/check8.py             # OK
python3 -I $C/test_referrers8.py    # 0 violations
cp $T/schedule.v8.json crustify/artifacts/schedule.json
```

- `decomp8.py` places what no module owned once wavefront 88793d7 made it a unit: the API
  macros, the function pointers written inline and the typedefs of their own of the
  shared headers, and the headers holding only macros and typedefs, which no module
  listed and the closure left out. Each split follows the decomposition's Bundling rule,
  as its docstring and `EDITS` record; six headers implementing what no TU does are
  header modules of their own (`NEW_MODULES`). `not_api.json` lists the attribute, linkage
  and configuration macros, which no module owns. `fp_owners.json` is `fp8.py`'s: each
  inline function pointer no module claimed, with the module of the struct or function
  holding it, read from the whole-closure schedule of run 7's decomposition in C kinds.
- `config8.py` writes the configuration from `modules.json`: every item `{name, kind}` with
  its C kind, a function pointer written inline named by its signature. Run 7's items took
  the route (`type`, `symbol`) as their kind, which wavefront 88793d7 refuses. With
  `--check`, it shows that run 7's `modules.json` gives run 7's modules back, kinds aside.
- `testfacts8.py` and `ties8.py` are run 7's, over the v6 schedule. A test's code reaches the
  macros it expands too. An enum's constants and values, and a header's `static inline`
  functions, are no internals. `areas.py` is run 7's, with `tests/util/hashmap.c`'s subject
  `src/util/hashmap.h` besides `hashmap_str.h`.
- `order8.py` reproduces wavefront's order of the modules from its graph, then cuts each
  foundational module's dependencies on other modules before the cycles are: the
  departure runs 4 to 7 took after wavefront's cut.
- `stages8.py` is run 7's `stages7.py`, plus a closed enum's `port/layout` and a function
  pointer's `port/unexport`. The C tests refer to a function pointer that a typedef
  declares where their code names the typedef or calls through it. They refer to one
  written inline through the function or global holding it.
- `write8.py` is run 7's `write7.py` in C kinds. The started waves take their items' C kinds
  and typedefs in place, at the user's request, and nothing else of them changes.
- `counters8.py` recounts the dependency records and function pointers. It counts the
  kinds the modules schema counts, at both ends of an edge.
