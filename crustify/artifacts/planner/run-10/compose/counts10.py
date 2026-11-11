"""The counts run 10's commit reports: how many todo waves and items each change
moved, added or removed. Change 2 (the later stages under the current rule for
the C tests) is HEAD's schedule against schedule.v10-run9numbering.json, run
10's stages numbered as run 9 numbered (FOUNDATIONAL_FIRST=0); change 3 (the
foundational modules first) is that against schedule.v10.json.

A wave is identified by its module and its content: a translation wave by its
items, a later-stage wave by its module and the modules whose integration it
waits for. An item is identified by (name, defined_in, kind, objective); it moved
when its wave's identity changed. A wave is renumbered when its index changed."""
import json, subprocess, collections, sys
R = '/target/libgit2-bindings/'
T = R + 'crustify/tmp/planner-run-10/'
HEAD = json.loads(subprocess.run(['git', '-C', R, 'show', 'HEAD:crustify/artifacts/schedule.json'],
                                 capture_output=True, check=True).stdout)
MID = json.load(open(T + 'schedule.v10-run9numbering.json'))
NEW = json.load(open(T + 'schedule.v10.json'))
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
compare(HEAD, MID, 'change 2 (later stages, current C-test rule):')
compare(MID, NEW, 'change 3 (foundational modules first):')
compare(HEAD, NEW, 'both:')
