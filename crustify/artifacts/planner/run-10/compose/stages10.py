"""Which later stages each scheduled entity of a port module calls for: the
scheduling spec's Later stages section, from wavefront's release points
(crustify/tmp/planner-run-10/schedule-port.json) and the whole user lists
(users.json, from users8.py). Run 9's stages8.py, re-derived under Crustify
6f1e04c's rules for the C tests (porting-workflow, Tests follow their modules;
conventions, Exports) and its Later stages section:

- a clar test case's subjects are those modules.json ties it to (ties8.py), each
  either through the module's internals (ties-why.json's `internals`: the fields
  or layout of its types, its static functions and globals) or through its public
  API alone (its `results`). Only foundational modules are wrap; they are never
  integrated, so no test leaves through them;
- a test case leaves at the first point at which it would leave as a test of one
  of its port subjects alone: at the integration of a subject whose internals it
  reaches; for a subject whose public API alone it uses, with that subject's
  export it calls, at the export's port/unexport, which waits for the export's own
  release point and the subject's integration. Its code, here, is its own and that
  of the helpers of other test files it calls, as ties8.py walks it for the
  results it checks. A subject whose public API alone it uses, and none of whose
  functions, globals or function pointers that code references, gives it no
  point to leave at;
- a test file's helpers, __initialize, __cleanup and file-scope uses leave with
  its last test case, and a helper file without test cases with the last test
  function of another file that reaches it (run 9's rule, unchanged);
- the C tests count as referrers of what they call, take the address of or
  access, and of the function pointers they name or call through (run 9's
  facts, unchanged), but for a test tied to the item's own module through its
  public API alone: such a test checks the export while other C needs it and
  leaves with it, so it holds no port/unexport back. A test tied to the item's
  module through its internals leaves at that module's integration at the
  latest, and holds nothing back either;
- deps/clar, the harness every remaining test links, never leaves, so an export
  it reaches is kept for good;
- a stage's freeing modules are its point's closure.modules less the item's own
  and less the modules ordered before the item's own (order8.py's order), which
  are integrated before it: a stage whose release point the item's own port
  already passes is not planned. port/layout and port/memory each wait for their
  own point alone.

A stage waits for the modules of its point's closure.modules so reduced, and for
one point of each test referrer that outlives those; with none left it is not
planned (`none`). A user in a module that stays C, a foundational module, or a
test that never leaves, keeps it from being planned (`never`).

A test leaving with an export, and that export's stage, may wait on each other
through other tests: the recursion is cut where it meets a stage being computed,
which then counts only its C referrers, so that the tests in such a cycle leave
together once the C that holds any of them has gone."""
import json, collections, csv, re, subprocess
R = '/target/libgit2-bindings/'
T = R + 'crustify/tmp/planner-run-10/'
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
WHY = json.load(open(T + 'ties-why.json'))     # 'file:fn' -> {'internals': [...], 'results': [...]}
def internal_subjects(f, n):
    return set(WHY.get(f'{f}:{n}', {}).get('internals', ()))
def own_code(f, n):
    """The test case, the static helpers of its file it calls and its suite's
    __initialize and __cleanup: the code whose references are its own."""
    fns = TF.get(f, {})
    seen, st = set(), [n] + [x for x in fns if re.match(r'test_.+__(initialize|cleanup)$', x)]
    while st:
        x = st.pop()
        if x in seen or x not in fns: continue
        seen.add(x)
        st += [l for l in fns[x]['local'] if not is_tc(l)]
    return seen
EXPORTABLE = ('function', 'global', 'function_pointer')
def subject_refs(f, n, m):
    """The functions, globals and function pointers of module m the test case's
    code references: its own code's, and that of the helpers of other test files
    it calls, transitively, as ties8.py's reached_modules walks them for its
    results; as unit keys."""
    out, seen, st = set(), set(), [(f, x) for x in own_code(f, n)]
    while st:
        ff, x = st.pop()
        if (ff, x) in seen or x not in TF.get(ff, {}): continue
        seen.add((ff, x))
        d = TF[ff][x]
        for r in list(d['calls']) + list(d['refs']):
            mm, name, df = r.split('|')
            if mm != m: continue
            for kind in EXPORTABLE:
                if f'{name}|{df}|{kind}' in UNITS: out.add(f'{name}|{df}|{kind}')
        st += [(ff, l) for l in d['local'] if not is_tc(l)]
        st += [tuple(tc.split('|', 1)) for tc in d['testcalls']]
    return out
FREE = {}        # unit key -> ('none'|'plan'|'never', freeing modules): the current port/unexport verdicts
def tc_leave(f, n):
    """The points at which a test case leaves, each a set of port modules all of
    which must be integrated: one per subject whose internals it reaches, and,
    for a subject whose public API alone it uses, one per export of it the case
    references, which it leaves with. None when it never leaves."""
    subj = sorted(m for m in subjects(f, n) if OBJ.get(m) == 'port')
    inner = internal_subjects(f, n)
    alts = []
    for m in subj:
        if m in inner:
            alts.append({m}); continue
        for e in sorted(subject_refs(f, n, m)):
            v, fr = FREE.get(e, ('none', []))
            if v == 'never': continue
            alts.append({m} | set(fr))
    # the earliest points only: an alternative that contains another is never first
    alts = [a for a in alts if not any(b < a for b in alts)]
    uniq = []
    for a in sorted(alts, key=lambda s: (len(s), sorted(s))):
        if a not in uniq: uniq.append(a)
    return uniq or None
def public_only(f, n, m):
    """Whether test function n of file f is tied to module m through its public
    API alone; for a helper, whether every test case of its file is."""
    if is_tc(n):
        return m in subjects(f, n) and m not in internal_subjects(f, n)
    tcs = TCS.get(f, [])
    return bool(tcs) and all(m in subjects(f, t) and m not in internal_subjects(f, t) for t in tcs)
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

def test_wait(k, own, before):
    """(modules the stage also waits for, test referrers that never leave)."""
    it = UNITS[k]
    wait, keep = set(), set()
    for tf, fn in sorted(TESTREFS.get((it['name'], it['defined_in']), ())):
        if TEST_FILE(tf) and tf.startswith('tests/') and public_only(tf, fn, own):
            continue                          # it checks the export and leaves with it
        alts = leave(tf, fn)
        if alts is None:
            keep.add(f'{tf}:{fn}'); continue
        # gone at the own module's integration, or once modules already waited for are
        if any(s <= ({own} | before) for s in alts):
            continue
        best = min(alts, key=lambda s: (len(s - before), max(LAYER[m] for m in s), sorted(s)))
        wait |= best - before - {own}
    return wait, keep
def stage(k, objective, with_tests=True):
    """('none'|'plan'|'never', freeing modules, C keepers) for one stage of unit k."""
    it = UNITS[k]; own = it['module']
    field, lst = POINT[objective]
    p = it.get(field)
    if p is None:
        return 'none', [], []
    keepers = sorted({mod(u) for u in U.get(k, {}).get(lst, []) if mod(u) and mod(u) != own and OBJ[mod(u)] == 'wrap'})
    # the modules ordered before the item's own are integrated before it: its port passes them
    freeing = set(m for m in p['closure']['modules'] if m != own) - ancestors(own)
    if objective == 'port/unexport' and with_tests:
        before = set(freeing) | {a for f in freeing for a in ancestors(f)} | ancestors(own)
        wait, keep = test_wait(k, own, before)
        keepers += sorted(keep)
        freeing |= set(frontier(wait))
    freeing = sorted(freeing)
    if keepers:
        return 'never', freeing, keepers
    if not freeing:
        return 'none', [], []
    return 'plan', freeing, []
UNEXPORT = sorted(k for k, it in UNITS.items()
                  if it['module'] and OBJ.get(it['module']) == 'port' and it['kind'] in EXPORTABLE)
ITERATIONS = []
def settle(limit=12):
    """The port/unexport verdicts, from the C referrers alone, then again with the
    tests' points from the verdicts before, until they no longer move."""
    FREE.clear()
    for k in UNEXPORT:
        v, fr, _ = stage(k, 'port/unexport', with_tests=False)
        FREE[k] = (v, fr)
    for i in range(limit):
        _fleave.clear()
        nxt = {}
        for k in UNEXPORT:
            v, fr, _ = stage(k, 'port/unexport')
            nxt[k] = (v, fr)
        moved = sum(1 for k in UNEXPORT if nxt[k] != FREE[k])
        ITERATIONS.append(moved)
        FREE.clear(); FREE.update(nxt)
        if not moved:
            return
    raise SystemExit(f'port/unexport verdicts still moving after {limit} rounds: {ITERATIONS}')
def stages():
    """{unit key: {objective: (verdict, freeing, keepers)}} over the units of port
    modules: a struct's or union's port/layout and port/memory, a closed enum's
    port/layout, a function's, global's or function pointer's port/unexport."""
    settle()
    out = {}
    for k, it in UNITS.items():
        m = it['module']
        if m is None or OBJ[m] != 'port':
            continue
        if it['kind'] in ('struct', 'union'):
            out[k] = {o: stage(k, o) for o in ('port/layout', 'port/memory')}
        elif it['kind'] == 'enum' and ENUMS.get(k) == 'closed':
            out[k] = {'port/layout': stage(k, 'port/layout')}
        elif it['kind'] in ('function', 'global', 'function_pointer'):
            out[k] = {'port/unexport': stage(k, 'port/unexport')}
    return out
if __name__ == '__main__':
    st = stages()
    c = collections.Counter((o, v[0]) for d in st.values() for o, v in d.items())
    print('rounds (verdicts moved):', ITERATIONS)
    print(sorted(c.items()))
    nv = [(k, v[2]) for k, d in st.items() for oo, v in d.items() if oo == 'port/unexport' and v[0] == 'never']
    keepf = collections.Counter(x.split(':')[0] for _, kp in nv for x in kp if ':' in x)
    print('port/unexport never:', len(nv), 'test files keeping them:', len(keepf), keepf.most_common(12))
    cm = collections.Counter(x for _, kp in nv for x in kp if ':' not in x)
    print('C keepers:', cm.most_common(5))
