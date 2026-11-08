"""Which later stages each scheduled entity of a port module calls for, from
wavefront's release points (crustify/tmp/planner-run-6/schedule-port.json) and
the whole user lists (users.json, from users.py): the scheduling spec's Later
stages section. A stage waits for the modules of its point's closure.modules
less the item's own; none left, it is not planned. A user in a module that
stays C (a foundational module, wrap in every campaign) keeps it from being
planned at all."""
import json, collections
R = '/target/libgit2-bindings/'
T = R + 'crustify/tmp/planner-run-6/'
S = json.load(open(T + 'schedule-port.json'))
U = json.load(open(T + 'users.json'))
M = json.load(open(R + 'crustify/artifacts/modules.json'))
FND = ' (foundational)'
def keyof(lu, name, f): return f"{lu}/{name}" + (FND if f else '')
OBJ = {}
for lu in M['link_units']:
    for x in lu['modules']:
        OBJ[keyof(lu['name'], x['name'], x['foundational'])] = x['objective']['objective']
UNITS = {}
for w in S['waves']:
    for b in w['batches']:
        for it in b['items']:
            UNITS['|'.join((it['name'], it['defined_in'], 'type' if it['kind'] == 'type' else 'symbol'))] = it
def mod(k): return UNITS[k]['module'] if k in UNITS else None
POINT = {'port/layout': ('opaque_after', 'layout'), 'port/memory': ('memory_unmanaged_after', 'memory'),
         'port/unexport': ('unreferenced_after', 'referrers')}
# ---- the C tests' references, which wavefront's closure leaves out. The
# exports serve the remaining C, its tests included (the conventions' Exports
# section): a test file tied to modules (modules.json `tests`) leaves with the
# first of them to be integrated, so waiting for any one of them is enough; a
# test tied to none stays C, and keeps every export it reaches.
import csv
T2 = R + 'crustify/wavefront/codeql/t2/'
def is_test(f): return f.startswith(('tests/', 'deps/clar/'))
TIED = collections.defaultdict(list)
for lu in M['link_units']:
    for x in lu['modules']:
        for t in x['tests']:
            TIED[t['path']].append((keyof(lu['name'], x['name'], x['foundational']), t['items']))
def tied_modules(tf, fn):
    out = []
    for m, items in TIED.get(tf, ()):
        inc, exc = items['included'], items['excluded']
        if (inc and fn not in inc) or (exc and fn in exc):
            continue
        out.append(m)
    return out
TESTREFS = collections.defaultdict(set)   # (name, def_file) -> {(test file, test fn)}
for path, cn, cf, tn, tf in (('function_calls.csv', 'caller_name', 'caller_file', 'callee_name', 'callee_def_file'),
                             ('function_addresses.csv', 'enclosing_name', 'access_file', 'target_name', 'target_def_file'),
                             ('global_accesses.csv', 'enclosing_name', 'access_file', 'global_name', 'global_def_file')):
    for r in csv.DictReader(open(T2 + path)):
        if is_test(r[cf]) and r[tf] and not is_test(r[tf]):
            TESTREFS[(r[tn], r[tf])].add((r[cf], r[cn]))
LAYER = {}
MDEPS = {}
import subprocess
for m in json.loads(subprocess.run(['git', '-C', R, 'show', '22a6e9493:crustify/artifacts/schedule.json'],
                                  capture_output=True, check=True).stdout)['modules']:
    k = keyof(m['link_unit'], m['module'], m['foundational'])
    LAYER[k] = m['layer']; MDEPS[k] = [keyof(d['link_unit'], d['module'], d['foundational']) for d in m['depends_on']['scheduled']]
_anc = {}
def ancestors(m):
    if m not in _anc:
        _anc[m] = set()
        a = set()
        for d in MDEPS.get(m, ()):
            a |= {d} | ancestors(d)
        _anc[m] = a
    return _anc[m]
def frontier(ms):
    return sorted(m for m in ms if not any(m in ancestors(o) for o in ms if o != m))
def test_wait(k, own):
    """(modules the stage also waits for, untied test referrers)."""
    it = UNITS[k]
    wait, untied = set(), set()
    for tf, fn in TESTREFS.get((it['name'], it['defined_in']), ()):
        ms = tied_modules(tf, fn)
        if own in ms:
            continue                       # leaves with the item's own module
        ms = [m for m in ms if OBJ[m] == 'port']
        if not ms:
            untied.add(f'{tf}:{fn}')
            continue
        wait.add(min(ms, key=lambda m: (LAYER[m], m)))
    return wait, untied
def stage(k, objective):
    """('none'|'plan'|'never', freeing modules, C keepers) for one stage of unit k."""
    it = UNITS[k]; own = it['module']
    field, lst = POINT[objective]
    p = it.get(field)
    if p is None:
        return 'none', [], []
    keepers = sorted({mod(u) for u in U.get(k, {}).get(lst, []) if mod(u) and mod(u) != own and OBJ[mod(u)] == 'wrap'})
    freeing = sorted(m for m in p['closure']['modules'] if m != own)
    if objective == 'port/unexport' and (freeing or keepers):
        # a function or global that remaining C never references has no export:
        # the tests count only for one that the closure's C references
        wait, untied = test_wait(k, own)
        keepers += sorted(untied)
        freeing = frontier(set(freeing) | wait)
    if keepers:
        return 'never', freeing, keepers
    if not freeing:
        return 'none', [], []
    return 'plan', freeing, []
def stages():
    """{unit key: {objective: (verdict, freeing, keepers)}} over the units of port modules."""
    out = {}
    for k, it in UNITS.items():
        m = it['module']
        if m is None or OBJ[m] != 'port':
            continue
        if it['kind'] == 'type' and it['source_kind'] in ('struct', 'union'):
            out[k] = {o: stage(k, o) for o in ('port/layout', 'port/memory')}
        elif it['kind'] == 'symbol' and it['source_kind'].startswith(('function', 'global')):
            out[k] = {'port/unexport': stage(k, 'port/unexport')}
    return out
if __name__ == '__main__':
    st = stages()
    c = collections.Counter((o, v[0]) for d in st.values() for o, v in d.items())
    print(sorted(c.items()))
    for o in ('port/layout', 'port/memory', 'port/unexport'):
        nv = [(k, v[2]) for k, d in st.items() for oo, v in d.items() if oo == o and v[0] == 'never']
        print(o, 'never:', len(nv), nv[:8])
