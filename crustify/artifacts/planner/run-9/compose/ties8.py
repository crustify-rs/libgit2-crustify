"""Planner run 7's ties.py, run on run 8's test facts and areas.

Each module's `tests` under the decomposition's current Bundling rule: a test's
subject is each module whose internals it uses (the fields or the layout of the
types it owns, its static functions and static globals) and each module whose own
results or effects its assertions check (areas.py). A module it only calls to set
up or tear down its fixtures, or whose enums it only uses, is no subject.

- A test case is a clar test, `test_<suite>__<name>`. Its code is its own body
  and the static helpers of its file it calls; its suite's `__initialize` and
  `__cleanup` run around it, so their internals uses are its own too.
- Results: an area module of its file that its code, or a test helper of another
  test file it calls, reaches; and the file's ALWAYS modules.
- A helper, initialize or cleanup function is tied by its own internals uses.
- A file entry names the functions tied to the module, or none when every test
  case of the file, or for a file without test cases every function, is.

Reads crustify/tmp/planner-run-9/testfacts.json (testfacts.py) and writes
crustify/tmp/planner-run-9/ties.json: {module key: [file entry, ...]}."""
import json, re, collections, sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from areas import area
R = '/target/libgit2-bindings/'
OUT = R + 'crustify/tmp/planner-run-9/'
D = json.load(open(OUT + 'testfacts.json'))
CFG = json.load(open(R + 'crustify/wavefront/configs/wavefront-config.json'))
MODS = {m['name'] for m in CFG['modules']}
FND = ' (foundational)'
def mkey(name):
    """A module path's key, as wavefront's configuration names it; the
    foundational module of a split TU is never an area (its routines are the
    ones every test calls)."""
    k = 'libgit2/' + name
    if k in MODS: return k
    if k + FND in MODS: return k + FND
    raise SystemExit(f'areas.py names {name}, which is no module')
def is_tc(n): return bool(re.match(r'test_.+__.+', n)) and not n.endswith(('__initialize', '__cleanup'))
def is_fixture(n): return bool(re.match(r'test_.+__(initialize|cleanup)$', n))
def reached_modules(f, n):
    """Modules the function's code, its file's static helpers and other test
    files' helpers reach, transitively: calls, addresses and global accesses."""
    seen, st, mods = set(), [(f, n)], set()
    while st:
        ff, nn = st.pop()
        if (ff, nn) in seen or nn not in D.get(ff, {}): continue
        seen.add((ff, nn))
        d = D[ff][nn]
        for k in list(d['calls']) + list(d['refs']):
            mods.add(k.split('|')[0])
        for l in d['local']:
            if not is_tc(l) or l == n: st.append((ff, l))
        for tc in d['testcalls']:
            tf, tn = tc.split('|', 1)
            st.append((tf, tn))
    return mods
def own_helpers(f, n):
    """The function and the static helpers of its own file it reaches."""
    seen, st = set(), [n]
    while st:
        x = st.pop()
        if x in seen or x not in D[f]: continue
        seen.add(x)
        st += [l for l in D[f][x]['local'] if not is_tc(l)]
    return seen
ties = collections.defaultdict(list)
why = {}
unarea = []
for f in sorted(D):
    fns = D[f]
    ar, always = area(f)
    tcs = sorted(n for n in fns if is_tc(n))
    if tcs and ar is None: unarea.append(f)
    ar = {mkey(m) for m in (ar or [])}
    always = {mkey(m) for m in always}
    fixture_int = set()
    for n in fns:
        if is_fixture(n):
            fixture_int |= set(fns[n]['internals'])
    subj = {}
    for n in fns:
        if is_tc(n):
            internals = set(fixture_int)
            for h in own_helpers(f, n):
                internals |= set(fns[h]['internals'])
            results = (reached_modules(f, n) & ar) | always
            subj[n] = internals | results
            why[(f, n)] = {'internals': sorted(internals), 'results': sorted(results)}
        else:
            subj[n] = set(fns[n]['internals'])
    # a test case with no subject tests its suite's through its fixtures
    filed = set().union(*(set(why[(f, n)]['results']) for n in tcs)) if tcs else set()
    for n in tcs:
        if not subj[n] and ar and (filed or reached_modules(f, n)):
            subj[n] = set(filed or ar)
            why[(f, n)]['results'] = sorted(subj[n]); why[(f, n)]['fallback'] = True
    allmods = set().union(*subj.values()) if subj else set()
    for m in sorted(allmods):
        tied = sorted(n for n in fns if m in subj[n] and n != '<file>')
        whole = (all(m in subj[n] for n in tcs) if tcs else
                 all(m in subj[n] for n in fns))
        if '<file>' in subj and m in subj['<file>']:
            whole = True                  # a file-scope use: the whole file depends on it
        ties[m].append({'path': f, 'items': {'included': [] if whole else tied, 'excluded': []}})
json.dump(ties, open(OUT + 'ties.json', 'w'), indent=1, sort_keys=True)
json.dump({f'{f}:{n}': v for (f, n), v in why.items()}, open(OUT + 'ties-why.json', 'w'), indent=0, sort_keys=True)
tied_files = {e['path'] for es in ties.values() for e in es}
tc_files = {f for f in D if any(is_tc(n) for n in D[f])}
print('modules with tests:', len(ties), 'file entries:', sum(len(v) for v in ties.values()))
print('test files with test cases:', len(tc_files), 'tied to no module:', len(tc_files - tied_files))
for f in sorted(tc_files - tied_files): print('  untied:', f)
if unarea: print('files with test cases and no area:', unarea)
