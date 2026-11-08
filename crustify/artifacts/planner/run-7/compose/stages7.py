"""Which later stages each scheduled entity of a port module calls for: the
scheduling spec's Later stages section, from wavefront's release points
(crustify/tmp/planner-run-7/schedule-port.json) and the whole user lists
(users.json, from users.py). Run 6's stages.py, with the C tests read anew:

- the tests are those modules.json now ties to each module (ties.py), by test
  function;
- a clar test case leaves with the first of its subject modules to be
  integrated; a test file's helpers, __initialize, __cleanup and file-scope
  uses leave with its last test case, and a helper file without test cases with
  the last test function of another file that reaches it;
- deps/clar, the harness every remaining test links, never leaves, so an export
  it reaches is kept for good;
- the tests count as referrers of every function and global, whether or not the
  closure's C references it too: the exports serve the remaining C, its tests
  included, so a function only a test of another subject calls keeps its export
  until that test leaves, and has a port/unexport then.

A stage waits for the modules of its point's closure.modules less the item's
own, and for one module of each test referrer that outlives those; with none
left it is not planned (`none`). A user in a module that stays C, a foundational
module, or a test that never leaves, keeps it from being planned (`never`)."""
import json, collections, csv, re, subprocess
R = '/target/libgit2-bindings/'
T = R + 'crustify/tmp/planner-run-7/'
S = json.load(open(T + 'schedule-port.json'))
U = json.load(open(T + 'users.json'))
M = json.load(open(R + 'crustify/artifacts/modules.json'))
TF = json.load(open(T + 'testfacts.json'))
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

# ---- the order of the modules: the committed schedule's, which this run's
# wavefront schedules reproduce
CUR = json.loads(subprocess.run(['git', '-C', R, 'show', '547ab887f:crustify/artifacts/schedule.json'],
                                capture_output=True, check=True).stdout)
LAYER, MDEPS = {}, {}
for m in CUR['modules']:
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

# ---- the tests: each test function's subject modules, and its callers
def is_tc(n): return bool(re.match(r'test_.+__.+', n)) and not n.endswith(('__initialize', '__cleanup'))
TIES = collections.defaultdict(set)          # (test file, fn) -> {module}
TIED_WHOLE = collections.defaultdict(set)    # test file -> {module}
for lu in M['link_units']:
    for x in lu['modules']:
        k = keyof(lu['name'], x['name'], x['foundational'])
        for t in x['tests']:
            inc = t['items']['included']
            if inc:
                for n in inc: TIES[(t['path'], n)].add(k)
            else:
                TIED_WHOLE[t['path']].add(k)
def subjects(f, n):
    return (TIES[(f, n)] | TIED_WHOLE[f]) if is_tc(n) else set()
# A test function leaves the build with its file, but for a clar test case,
# which the integration of its first subject module removes on its own. A test
# file's helpers, __initialize, __cleanup and file-scope uses therefore stay
# while any of its test cases does; a helper file without test cases stays
# while any test function of another file that reaches it does.
EXT_CALLERS = collections.defaultdict(set)   # helper file -> {(file, fn) of other files reaching it}
for f, fns in TF.items():
    for n, d in fns.items():
        for tc in d['testcalls']:
            tf, tn = tc.split('|', 1)
            if tf != f: EXT_CALLERS[tf].add((f, n))
TCS = {f: sorted(n for n in fns if is_tc(n)) for f, fns in TF.items()}
def tc_leave(f, n):
    ms = sorted(m for m in subjects(f, n) if OBJ.get(m) == 'port')
    return [{m} for m in ms] or None
def all_of(conds):
    """Every condition holds: one alternative of each, the cheapest first, so
    that the product stays small. None when one never holds."""
    combo = [frozenset()]
    for lc in conds:
        if lc is None: return None
        lc = sorted(lc, key=lambda s: (len(s), sorted(s)))[:3]
        combo = sorted({a | frozenset(s) for a in combo for s in lc}, key=lambda s: (len(s), sorted(s)))[:6]
    return [set(s) for s in combo]
_fleave = {}
def file_leave(f, stack=()):
    if f in _fleave: return _fleave[f]
    if f in stack: return [set()]             # a helper-file cycle: judged by the other callers
    if TCS.get(f):
        r = all_of(tc_leave(f, n) for n in TCS[f])
    elif EXT_CALLERS.get(f):
        r = all_of(leave(cf, cn, stack + (f,)) for cf, cn in sorted(EXT_CALLERS[f]))
    else:
        r = None                              # a helper file nothing reaches stays
    if not stack: _fleave[f] = r
    return r
def leave(f, n, stack=()):
    """How the test function leaves: a list of alternatives, each a set of port
    modules all of which must be integrated; None when it never leaves."""
    return tc_leave(f, n) if is_tc(n) else file_leave(f, stack)
TEST_FILE = lambda f: f.startswith(('tests/', 'deps/clar/'))   # the harness never leaves: no test file of its own
TESTREFS = collections.defaultdict(set)      # (name, def_file) -> {(test file, test fn)}
T2 = R + 'crustify/wavefront/codeql/t2/'
for path, cn, cf, tn, tf in (('function_calls.csv', 'caller_name', 'caller_file', 'callee_name', 'callee_def_file'),
                             ('function_addresses.csv', 'enclosing_name', 'access_file', 'target_name', 'target_def_file'),
                             ('global_accesses.csv', 'enclosing_name', 'access_file', 'global_name', 'global_def_file')):
    for r in csv.DictReader(open(T2 + path)):
        if TEST_FILE(r[cf]) and r[tf] and not TEST_FILE(r[tf]):
            TESTREFS[(r[tn], r[tf])].add((r[cf], r[cn] or '<file>'))

def test_wait(k, own, before):
    """(modules the stage also waits for, test referrers that never leave)."""
    it = UNITS[k]
    wait, keep = set(), set()
    for tf, fn in sorted(TESTREFS.get((it['name'], it['defined_in']), ())):
        alts = leave(tf, fn)
        if alts is None:
            keep.add(f'{tf}:{fn}'); continue
        # gone at the own module's integration, or once modules already waited for are
        if any(s <= ({own} | before) for s in alts):
            continue
        best = min(alts, key=lambda s: (len(s - before), max(LAYER[m] for m in s), sorted(s)))
        wait |= best - before - {own}
    return wait, keep
def stage(k, objective):
    """('none'|'plan'|'never', freeing modules, C keepers) for one stage of unit k."""
    it = UNITS[k]; own = it['module']
    field, lst = POINT[objective]
    p = it.get(field)
    if p is None:
        return 'none', [], []
    keepers = sorted({mod(u) for u in U.get(k, {}).get(lst, []) if mod(u) and mod(u) != own and OBJ[mod(u)] == 'wrap'})
    freeing = set(m for m in p['closure']['modules'] if m != own)
    if objective == 'port/unexport':
        before = set(freeing) | {a for f in freeing for a in ancestors(f)}
        wait, keep = test_wait(k, own, before)
        keepers += sorted(keep)
        freeing |= set(frontier(wait))
    freeing = sorted(freeing)
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
    nv = [(k, v[2]) for k, d in st.items() for oo, v in d.items() if oo == 'port/unexport' and v[0] == 'never']
    keepf = collections.Counter(x.split(':')[0] for _, kp in nv for x in kp if ':' in x)
    print('port/unexport never:', len(nv), 'test files keeping them:', len(keepf), keepf.most_common(12))
    cm = collections.Counter(x for _, kp in nv for x in kp if ':' not in x)
    print('C keepers:', cm.most_common(5))
