"""How many planned port/unexport items the remaining C tests still call after
the integrations their wave waits for: a test file tied to modules (modules.json `tests`)
leaves with the first of them to be integrated, so a call from it outlives the
export's removal unless one of those modules is integrated before the stage's
wave runs (a freeing module, or a module ordered before one). wavefront's
closure holds no tests, so its unreferenced_after counts none of these calls."""
import json, csv, collections
R = '/target/libgit2-bindings/'
D = json.load(open(R + 'crustify/tmp/planner-run-6/schedule.v6.json'))
M = json.load(open(R + 'crustify/artifacts/modules.json'))
mk = lambda r: (r['link_unit'], r['module'], r['foundational'])
tied = collections.defaultdict(list)          # test file -> [(module, included fns or None)]
for lu in M['link_units']:
    for m in lu['modules']:
        for t in m['tests']:
            tied[t['path']].append(((lu['name'], m['name'], m['foundational']), t['items']))
anc = collections.defaultdict(set)
order = {mk(m): m for m in D['modules']}
memo = {}
def ancestors(k):
    if k in memo: return memo[k]
    memo[k] = set()
    a = set()
    for d in order[k]['depends_on']['scheduled']:
        a |= {mk(d)} | ancestors(mk(d))
    memo[k] = a; return a
calls = collections.defaultdict(set)          # (callee, def_file) -> {(test file, caller fn)}
for r in csv.DictReader(open(R + 'crustify/wavefront/codeql/t2/function_calls.csv')):
    if r['caller_file'].startswith('tests/'):
        calls[(r['callee_name'], r['callee_def_file'])].add((r['caller_file'], r['caller_name']))
def modules_of(tf, fn):
    out = []
    for m, items in tied.get(tf, ()):
        inc, exc = items['included'], items['excluded']
        if (inc and fn not in inc) or (exc and fn in exc): continue
        out.append(m)
    return out
W = {w['index']: w for w in D['waves']}
hit = []; untied = 0
for w in D['waves']:
    for i in w['items']:
        if i['objective'] != 'port/unexport': continue
        own = mk(w)
        fr = {mk(W[d]) for d in w['depends_on']['scheduled']} - {own}   # the modules the stage's wave waits for
        before = set(fr) | {a for f in fr for a in ancestors(f)}
        late = set()
        for tf, fn in calls.get((i['name'], i['defined_in']), ()):
            ms = modules_of(tf, fn)
            if not ms or own in ms:
                continue            # tied to no module (the stage is never planned), or leaves with its own
            if not any(m in before for m in ms):
                late.add((tf, fn))
        if late: hit.append((i['name'], len(late), sorted(late)[:2]))
print('port/unexport items:', sum(1 for w in D['waves'] for i in w['items'] if i['objective'] == 'port/unexport'))
print('called by remaining C tests after the integrations their wave waits for:', len(hit))
for h in sorted(hit, key=lambda h: -h[1])[:10]: print(' ', h)
