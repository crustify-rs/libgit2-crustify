# The foundational modules' todo waves numbered among the other modules'

## What the earlier plan decided

Planner run 9 (`66def428e`) numbered the `todo` waves from 169 in the order of the
modules, by `layer` (`crustify/artifacts/planner/run-9/compose/write8.py`):

```python
regular.sort(key=lambda p: (mlayer(p[0]['module']), MIDX[p[0]['module']], p[0]['sublayer'], p[0]['index']))
```

The seven foundational modules sit in layers 0 to 2 of that order, beside every other
module of those layers, so their waves were interleaved with the others'. In run 9's
`schedule.json`, wave 169 is `src/cli/opt.c`'s; `deps/reftable/basics.c`'s first wave is
199, after 30 waves of other modules; and the foundational `todo` waves are 199-205, 227,
263-266, 283, 284, 287, 288 and 290, with 105 waves of other modules numbered before the
last of them.

## The evidence against it

The decomposition's Foundational modules section: "The order then runs them first, and
among them the type-erased ones before the string ones that call them." The scheduling
skill's Order section: "The foundational modules depend on no other, so they run first."
The schedule schema's Indices section makes the `todo` indices the plan's one total order
on disk, which the waves follow as they start, so the numbering is where "first" is
recorded. The user asked for this correction (`crustify/artifacts/planner/run-10/task.md`,
item 3).

No foundational wave depends, through `depends_on.scheduled`, on a wave of a module that
is not foundational: run 9's order already cuts the foundational modules' dependencies on
the others (`order8.py`), so numbering them first stays a topological order.

```sh
cd /target/libgit2-bindings
python3 -I -c '
import json, subprocess
D = json.loads(subprocess.run(["git", "show", "66def428e:crustify/artifacts/schedule.json"], capture_output=True).stdout)
t = [w for w in D["waves"] if w["status"] == "todo"]
f = [w["index"] for w in t if w["foundational"]]
print(t[0]["index"], t[0]["module"], f, sum(1 for w in t if not w["foundational"] and w["index"] < f[-1]))'
```

## The edit and what it moves

`crustify/artifacts/planner/run-10/compose/write10.py` numbers the foundational modules'
`todo` waves first, from 169: the type-erased ones (`deps/reftable/basics.c`,
`src/util/allocators/failalloc.c`, `src/util/alloc.c`, `stdalloc.c`, `win32_leakcheck.c`,
`debugalloc.c`, by the order of the modules) before the string one that calls them
(`src/util/strlist.c`, whose routines call `alloc.c`'s `git__calloc`, `git__strdup` and `git__free`).
`deps/reftable/basics.c` holds reftable's type-erased allocators (`reftable_malloc`,
`reftable_realloc`, `reftable_calloc`, `reftable_free`), which its string routines call,
and depends on no other module, so it counts among the type-erased ones. The
rest follow as run 9 numbered them: the translation waves in the order of the modules,
each later-stage wave right after the last wave it depends on.

The foundational waves are now 169-185, and the first wave of another module is 186
(`src/cli/opt.c`). No wave or item is added, removed or moved between waves by this edit:
over run 10's re-derived later stages, it renumbers 122 `todo` waves
(`crustify/artifacts/planner/run-10/compose/counts10.py`, change 3), their `depends_on`
and the release points that name them renumbered with them. `check10.py` checks that the
foundational waves come first and the string one last among them.

## Questions for the user

None.
