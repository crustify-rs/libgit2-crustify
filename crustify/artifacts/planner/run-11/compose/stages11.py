"""Which later stages each scheduled entity of a port module calls for: the
scheduling spec's Later stages section, from wavefront's release points
(crustify/tmp/planner-run-11/schedule-port.json) and the whole user lists
(users.json, from users8.py). Run 10's stages10.py, re-derived under Crustify
596b74e's rule for the C tests (scheduling, Later stages 5.2; porting-workflow,
Tests follow their modules; testing, C tests carried over):

- the C tests hold no stage back. A stage waits for the modules of its point's
  `closure.modules`, the remaining C that wavefront's closure counts, less the
  item's own and less the modules ordered before the item's own (order8.py's
  order), which are integrated before it: a stage whose release point the item's
  own port already passes is not planned (`none`). A user in a module that stays
  C, a foundational module, keeps it from being planned (`never`). port/layout
  and port/memory each wait for their own point alone (as run 10);
- a port/unexport carries over the in-scope C test cases that still reference
  its export when it runs (carryover11.py records them per wave). A test case
  references what its own code does (its body, the static helpers of its file it
  calls, its suite's __initialize and __cleanup and its file's file-scope uses)
  and what the helpers of other test files and of deps/clar it calls do,
  transitively: calls, addresses, global accesses, and the function pointers it
  names or calls through (run 10's facts, unchanged). The harness's own code,
  deps/clar/main.c's main and what it reaches, is no test case's: its references
  are recorded apart (HARNESS), as the harness's;
- an export whose `closure` point the item's own port passes, but which C tests
  or the harness still reference after the item's module is integrated, is
  exported for them at that integration (conventions, Exports), so its
  port/unexport is planned right after the integration, freed by it alone, and
  carries them over: no export is kept for good because of a test. A test case
  is gone by then when it reaches the internals of the item's module or of one
  ordered before it, which it leaves with (porting-workflow, Tests follow their
  modules);
- the test cases whose body the configured build compiles empty (empty11.py)
  are out of scope, as a configured-out TU is: they reference nothing.

`stages()` returns {unit key: {objective: (verdict, freeing modules, keepers)}}."""
import json, collections, csv, re
R = '/target/libgit2-bindings/'
T = R + 'crustify/tmp/planner-run-11/'
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
            UNITS['|'.join((it['name'], it['defined_in'], it['kind']))] = it
ENUMS = json.load(open(T + 'enums.json'))
def mod(k): return UNITS[k]['module'] if k in UNITS else None
POINT = {'port/layout': ('opaque_after', 'layout'), 'port/memory': ('memory_unmanaged_after', 'memory'),
         'port/unexport': ('unreferenced_after', 'referrers')}

# ---- the order of the modules: order8.py's
ORDER = json.load(open(T + 'order.json'))
LAYER = {k: v['layer'] for k, v in ORDER.items()}
MDEPS = {k: v['scheduled'] for k, v in ORDER.items()}
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

# ---- the test cases in scope and the internals they reach
def is_tc(n): return bool(re.match(r'test_.+__.+', n)) and not n.endswith(('__initialize', '__cleanup'))
def is_fixture(n): return bool(re.match(r'test_.+__(initialize|cleanup)$', n))
EMPTY = set(json.load(open(T + 'empty.json')))
CASES = sorted(f'{f}:{n}' for f, fns in TF.items() for n in fns if is_tc(n) and f'{f}:{n}' not in EMPTY)
WHY = json.load(open(T + 'ties-why.json'))     # 'file:fn' -> {'internals': [...], 'results': [...]}
def internals(case):
    """The port modules whose internals the test case reaches: it leaves at the
    first of their integrations."""
    return {m for m in WHY.get(case, {}).get('internals', ()) if OBJ.get(m) == 'port'}

# ---- the test-side call graph: tests/ and deps/clar functions
TEST_FILE = lambda f: f.startswith(('tests/', 'deps/clar/'))
T2 = R + 'crustify/wavefront/codeql/t2/'
CALLS = collections.defaultdict(set)         # (file, fn) -> {(file, fn)} of test-side functions
for path, cn, cf, tn, tf in (('function_calls.csv', 'caller_name', 'caller_file', 'callee_name', 'callee_def_file'),
                             ('function_addresses.csv', 'enclosing_name', 'access_file', 'target_name', 'target_def_file')):
    for r in csv.DictReader(open(T2 + path)):
        if TEST_FILE(r[cf]) and r[tf] and TEST_FILE(r[tf]):
            # a test case or fixture is reached only as its own: clar's suite tables take every one's address
            if is_tc(r[tn]) or is_fixture(r[tn]): continue
            CALLS[(r[cf], r[cn] or '<file>')].add((r[tf], r[tn]))
def closure(roots):
    seen, st = set(), list(roots)
    while st:
        x = st.pop()
        if x in seen: continue
        seen.add(x)
        st += CALLS.get(x, ())
    return seen
HARNESS_ROOT = ('deps/clar/main.c', 'main')
HARNESS = closure([HARNESS_ROOT])            # the harness's own code: no test case's
def reach(case):
    """The test-side functions whose references are the test case's."""
    f, n = case.split(':')
    roots = [(f, n), (f, '<file>')] + [(f, x) for x in TF.get(f, {}) if is_fixture(x)]
    return closure(roots)

# ---- what test-side code references: run 10's facts
TESTREFS = collections.defaultdict(set)      # (name, def_file) -> {(test file, test fn)}
for path, cn, cf, tn, tf in (('function_calls.csv', 'caller_name', 'caller_file', 'callee_name', 'callee_def_file'),
                             ('function_addresses.csv', 'enclosing_name', 'access_file', 'target_name', 'target_def_file'),
                             ('global_accesses.csv', 'enclosing_name', 'access_file', 'global_name', 'global_def_file')):
    for r in csv.DictReader(open(T2 + path)):
        if TEST_FILE(r[cf]) and r[tf] and not TEST_FILE(r[tf]):
            TESTREFS[(r[tn], r[tf])].add((r[cf], r[cn] or '<file>'))
# function pointers a typedef declares: the tests' code that names the typedef
# or an alias, or calls through it
FPNAME = {}                                  # typedef name -> (name, defined_in) of its function pointer
for k, it in UNITS.items():
    if it['kind'] == 'function_pointer' and '(*)' not in it['name']:
        for n in [it['name']] + it['typedefs']:
            FPNAME[n] = (it['name'], it['defined_in'])
for path, cn, cf, tn in (('signature_type_uses.csv', 'function_name', 'function_def_file', 'type_name'),
                         ('local_type_uses.csv', 'function_name', 'function_def_file', 'type_name'),
                         ('global_type_uses.csv', None, 'global_def_file', 'type_name'),
                         ('field_type_uses.csv', None, 'struct_def_file', 'type_name')):
    for r in csv.DictReader(open(T2 + path)):
        if TEST_FILE(r[cf]) and r[tn] in FPNAME:
            TESTREFS[FPNAME[r[tn]]].add((r[cf], r[cn] if cn else '<file>'))
for r in csv.DictReader(open(T2 + 'callback_call_sites.csv')):
    if TEST_FILE(r['callsite_def_file']) and r['callback_name'] in FPNAME:
        TESTREFS[FPNAME[r['callback_name']]].add((r['callsite_def_file'], r['callsite_name']))
# function pointers written inline: the tests that refer to the function or
# global holding a parameter, return or value site
for k, it in UNITS.items():
    if it['kind'] == 'function_pointer' and '(*)' in it['name']:
        for site in U.get(k, {}).get('sites', []):
            sn, sf, use = site.split('|')
            if use in ('param', 'return', 'global'):
                TESTREFS[(it['name'], it['defined_in'])] |= TESTREFS.get((sn, sf), set())

EXPORTABLE = ('function', 'global', 'function_pointer')
UNEXPORT = sorted(k for k, it in UNITS.items()
                  if it['module'] and OBJ.get(it['module']) == 'port' and it['kind'] in EXPORTABLE)
REFS_OF = collections.defaultdict(set)       # (test file, fn) -> {unit key of an export}
for k in UNEXPORT:
    it = UNITS[k]
    for x in TESTREFS.get((it['name'], it['defined_in']), ()):
        REFS_OF[x].add(k)
CASE_REFS = {}                               # test case -> {unit key}
REFERRING = collections.defaultdict(set)     # unit key -> {test case}
for c in CASES:
    ks = set().union(*(REFS_OF.get(x, set()) for x in reach(c)))
    CASE_REFS[c] = ks
    for k in ks: REFERRING[k].add(c)
HARNESS_REFS = collections.defaultdict(set)  # unit key -> {harness function}
for x in HARNESS:
    for k in REFS_OF.get(x, ()): HARNESS_REFS[k].add(f'{x[0]}:{x[1]}')

def tests_left(k):
    """The test cases, and the harness functions, that still reference unit k's
    export once its module is integrated: a case that reaches the internals of
    the module, or of one ordered before it, has left with it."""
    own = UNITS[k]['module']
    gone = {own} | ancestors(own)
    return sorted(c for c in REFERRING.get(k, ()) if not internals(c) & gone), sorted(HARNESS_REFS.get(k, ()))

def stage(k, objective):
    """('none'|'plan'|'never', freeing modules, C keepers) for one stage of unit k."""
    it = UNITS[k]; own = it['module']
    field, lst = POINT[objective]
    p = it.get(field)
    if p is None:
        return 'none', [], []
    keepers = sorted({mod(u) for u in U.get(k, {}).get(lst, []) if mod(u) and mod(u) != own and OBJ[mod(u)] == 'wrap'})
    # the modules ordered before the item's own are integrated before it: its port passes them
    freeing = sorted(set(m for m in p['closure']['modules'] if m != own) - ancestors(own))
    if keepers:
        return 'never', freeing, keepers
    if freeing:
        return 'plan', freeing, []
    if objective == 'port/unexport' and any(tests_left(k)):
        return 'plan', [], []                 # exported for the C tests alone: removed right after the integration
    return 'none', [], []

def stages():
    """{unit key: {objective: (verdict, freeing, keepers)}} over the units of port
    modules: a struct's or union's port/layout and port/memory, a closed enum's
    port/layout, a function's, global's or function pointer's port/unexport."""
    out = {}
    for k, it in UNITS.items():
        m = it['module']
        if m is None or OBJ[m] != 'port':
            continue
        if it['kind'] in ('struct', 'union'):
            out[k] = {o: stage(k, o) for o in ('port/layout', 'port/memory')}
        elif it['kind'] == 'enum' and ENUMS.get(k) == 'closed':
            out[k] = {'port/layout': stage(k, 'port/layout')}
        elif it['kind'] in EXPORTABLE:
            out[k] = {'port/unexport': stage(k, 'port/unexport')}
    return out
if __name__ == '__main__':
    st = stages()
    c = collections.Counter((o, v[0]) for d in st.values() for o, v in d.items())
    print(sorted(c.items()))
    tonly = [k for k, d in st.items() if d.get('port/unexport', ('',))[0] == 'plan' and not d['port/unexport'][1]]
    print('port/unexport planned for the C tests alone:', len(tonly),
          'of them for the harness alone:', sum(1 for k in tonly if not tests_left(k)[0]))
    print('in-scope test cases:', len(CASES), 'referencing an export:', sum(1 for c in CASES if CASE_REFS[c]))
    print('harness references:', sorted(k.split('|')[0] for k in HARNESS_REFS))
    nv = [(k, v[2]) for k, d in st.items() for oo, v in d.items() if oo == 'port/unexport' and v[0] == 'never']
    print('port/unexport never:', len(nv), collections.Counter(x for _, kp in nv for x in kp).most_common(5))
