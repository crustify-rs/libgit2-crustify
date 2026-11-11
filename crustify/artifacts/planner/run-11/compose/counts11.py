"""The counts run 11's commit reports: how many todo waves and items each change
moved, added or removed. Change 2 (the C tests hold no export back) is HEAD's
schedule against schedule.v11.json; change 3 (empty test cases out of scope)
moves no wave or item, since such a case references nothing (empty11.py), and
changes only modules.json's tests, counted here against HEAD's.

A wave is identified by its module and its content: a translation wave by its
items, a later-stage wave by its module and the modules whose integration it
waits for. An item is identified by (name, defined_in, kind, objective); it moved
when its wave's identity changed. A wave is renumbered when its index changed."""
import json, subprocess, collections, sys
R = '/target/libgit2-bindings/'
T = R + 'crustify/tmp/planner-run-11/'
HEAD = json.loads(subprocess.run(['git', '-C', R, 'show', 'HEAD:crustify/artifacts/schedule.json'],
                                 capture_output=True, check=True).stdout)
NEW = json.load(open(T + 'schedule.v11.json'))
STAGE = ('port/layout', 'port/memory', 'port/unexport')
mk = lambda w: (w['link_unit'], w['module'], w['foundational'])
def ident(D):
    W = {w['index']: w for w in D['waves']}
    out = {}
    for w in D['waves']:
        if w['status'] != 'todo': continue
        if all(i['objective'] in STAGE for i in w['items']):
            frees = frozenset(mk(W[d]) for d in w['depends_on']['scheduled'] if mk(W[d]) != mk(w))
            out[w['index']] = ('stage', mk(w), frees)
        else:
            out[w['index']] = ('translation', mk(w), frozenset((i['name'], i['defined_in'], i['kind'], i['objective']) for i in w['items']))
    return W, out
def compare(A, B, label):
    WA, IA = ident(A); WB, IB = ident(B)
    ia = {v: k for k, v in IA.items()}; ib = {v: k for k, v in IB.items()}
    assert len(ia) == len(IA) and len(ib) == len(IB)
    items = lambda W, I: {(i['name'], i['defined_in'], i['kind'], i['objective']): I[w] for w in I for i in W[w]['items']}
    xa, xb = items(WA, IA), items(WB, IB)
    r = collections.OrderedDict()
    r['waves removed'] = len(set(ia) - set(ib)); r['waves added'] = len(set(ib) - set(ia))
    r['waves kept'] = len(set(ia) & set(ib))
    r['waves renumbered'] = sum(1 for v in set(ia) & set(ib) if ia[v] != ib[v])
    r['items removed'] = len(set(xa) - set(xb)); r['items added'] = len(set(xb) - set(xa))
    r['items moved'] = sum(1 for k in set(xa) & set(xb) if xa[k] != xb[k])
    by = collections.Counter()
    for k in set(xa) - set(xb): by[('removed', k[3], k[2])] += 1
    for k in set(xb) - set(xa): by[('added', k[3], k[2])] += 1
    for k in set(xa) & set(xb):
        if xa[k] != xb[k]: by[('moved', k[3], k[2])] += 1
    r['by objective and kind'] = sorted(by.items())
    print(label, json.dumps(r, default=str))
    return r
compare(HEAD, NEW, 'change 2 (the C tests hold no export back):')
HM = json.loads(subprocess.run(['git', '-C', R, 'show', 'HEAD:crustify/artifacts/modules.json'],
                               capture_output=True, check=True).stdout)
NM = json.load(open(R + 'crustify/artifacts/modules.json'))
tests = lambda M: {(lu['name'], x['name'], x['foundational']): x['tests'] for lu in M['link_units'] for x in lu['modules']}
a, b = tests(HM), tests(NM)
changed = [k for k in a if a[k] != b[k]]
ents = lambda t: {e['path'] for e in t}
print('change 3 (empty test cases out of scope): modules whose tests changed', len(changed),
      'file entries removed', sum(len(ents(a[k]) - ents(b[k])) for k in changed),
      'entries changed', sum(1 for k in changed for x in a[k] for y in b[k] if x['path'] == y['path'] and x != y))
diff_other = sum(1 for lu0, lu1 in zip(HM['link_units'], NM['link_units']) for x0, x1 in zip(lu0['modules'], lu1['modules'])
                 if dict(x0, tests=None) != dict(x1, tests=None))
print('modules changed in any other field:', diff_other)
