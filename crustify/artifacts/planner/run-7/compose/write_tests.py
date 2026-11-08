"""Writes each module's `tests` into crustify/artifacts/modules.json from
crustify/tmp/planner-run-7/ties.json (ties.py), and the comment's sentence on
how the tests are tied. Nothing else in modules.json changes."""
import json
R = '/target/libgit2-bindings/'
P = R + 'crustify/artifacts/modules.json'
import subprocess
# the decomposition this run forked from (547ab887f), whose tests it re-derives
M = json.loads(subprocess.run(['git', '-C', R, 'show', '547ab887f:crustify/artifacts/modules.json'],
                              capture_output=True, check=True).stdout)
T = json.load(open(R + 'crustify/tmp/planner-run-7/ties.json'))
FND = ' (foundational)'
seen = set()
for lu in M['link_units']:
    for x in lu['modules']:
        k = f"{lu['name']}/{x['name']}" + (FND if x['foundational'] else '')
        x['tests'] = sorted(T.get(k, []), key=lambda e: e['path'])
        seen.add(k)
assert set(T) <= seen, set(T) - seen
OLD = ("The test executables (libgit2_tests, util_tests) and deps/clar are tests, listed by the modules whose "
       "internals they use (the earlier rule of specs/planning/decomposition.md's Bundling; the current one, which "
       "also ties a test to each module whose results its assertions check, is not yet applied).")
NEW = ("The test executables' sources (tests/libgit2, tests/util) are tests, each listed by the modules whose subject "
       "it is, as specs/planning/decomposition.md's Bundling describes: those whose internals it uses (the fields or "
       "layout of the types they own, macro expansions included, and their static functions and static globals) and "
       "those whose own results or effects its assertions check, the subject area its suite is named for "
       "(crustify/artifacts/planner/run-7/compose/areas.py) that its code reaches. A test file is split by test "
       "function where its tests divide between modules; a clar test case's code includes the static helpers of its "
       "file it calls and its suite's __initialize and __cleanup. deps/clar is the harness every test links, a "
       "fixture rather than a test, and is tied to no module.")
assert OLD in M['_comment']
M['_comment'] = M['_comment'].replace(OLD, NEW)
open(P, 'w').write(json.dumps(M, indent=2, ensure_ascii=False) + '\n')
print('modules with tests:', sum(1 for lu in M['link_units'] for x in lu['modules'] if x['tests']))
