"""The order of the modules: wavefront's weighted feedback-arc-set over the
module graph (src/wavefront/order.py), with the foundational modules first.

The scheduling spec orders the modules over their units' dependencies, each
dependency weighing the distinct units it reaches, and runs the foundational
modules first, since they depend on no other. With every macro a unit, the
macros of shared headers tie the foundational modules into cycles with the
modules whose macros they expand (alloc.c with errors.c through GIT_ASSERT and
GIT_ERROR_CHECK_ALLOC, with integer.h, strnlen.h and cc-compat.h), and the
feedback-arc-set then puts errors.c ahead of alloc.c. So each foundational
module's dependencies on modules that are not foundational are cut before the
cycles are: the departure runs 4 to 7 took from wavefront's order, applied
first. The rest is wavefront's rule, which `verify()` reproduces exactly on its
own graph: within each strongly connected component, a module is placed the
earlier the more distinct units of the others depend on it, net of those it
depends on, ties by its index in the configuration's modules; every edge to a
module placed after it is cut; layers are the longest path over what is kept.

Input: crustify/tmp/planner-run-9/schedule-port.json (the plan schedule).
`order()` returns {module: {'layer', 'scheduled': [...], 'cut': [...]}}."""
import json, collections, sys
sys.path.insert(0, '/crustify-rs/wavefront/src')
from wavefront.order import _cut, _layers, _sort_key   # noqa: E402
R = '/target/libgit2-bindings/'
T = R + 'crustify/tmp/planner-run-9/'
FND = ' (foundational)'

def graph(S):
    idx = {}
    cfg = json.load(open(R + 'crustify/wavefront/configs/wavefront-config.json'))
    for i, m in enumerate(cfg['modules']):
        idx[m['name']] = i
    adj, w, wavefront = collections.defaultdict(set), {}, {}
    for m in S['modules']:
        for part in ('scheduled', 'cut'):
            for d in m['depends_on'][part]:
                adj[m['name']].add(d['module']); w[(m['name'], d['module'])] = d['unique_edges']
                wavefront[(m['name'], d['module'])] = part
    nodes = sorted({m['name'] for m in S['modules']}, key=lambda n: idx[n])
    return nodes, adj, w, wavefront, idx

def solve(nodes, adj, w, idx, foundational_first):
    a = {u: set(vs) for u, vs in adj.items()}
    pre = set()
    if foundational_first:
        for u in list(a):
            if u.endswith(FND):
                for v in list(a[u]):
                    if not v.endswith(FND):
                        a[u].discard(v); pre.add((u, v))
    # wavefront keys groups by index: map to indices so that ties sort as there
    inv = {idx[n]: n for n in nodes}
    ai = {idx[u]: {idx[v] for v in vs} for u, vs in a.items()}
    cut = {(inv[u], inv[v]) for u, v in _cut([idx[n] for n in nodes], ai, lambda u, v: w[(inv[u], inv[v])])}
    kept = {u: {v for v in vs if (u, v) not in cut} for u, vs in a.items()}
    lay = _layers(nodes, kept)
    return cut | pre, kept, lay

def verify(S):
    nodes, adj, w, wf, idx = graph(S)
    cut, kept, lay = solve(nodes, adj, w, idx, False)
    mine = {e: ('cut' if e in cut else 'scheduled') for e in wf}
    bad = [e for e in wf if mine[e] != wf[e]]
    layers_bad = [m['name'] for m in S['modules'] if m['layer'] != lay[m['name']]]
    return bad, layers_bad

def order(S):
    nodes, adj, w, wf, idx = graph(S)
    cut, kept, lay = solve(nodes, adj, w, idx, True)
    out = {}
    for n in nodes:
        out[n] = {'layer': lay[n], 'scheduled': sorted(kept.get(n, ())), 'cut': sorted(v for v in adj.get(n, ()) if (n, v) in cut)}
    return out

if __name__ == '__main__':
    S = json.load(open(T + 'schedule-port.json'))
    bad, lb = verify(S)
    print('wavefront order reproduced:', not bad and not lb, len(bad), len(lb))
    o = order(S)
    print('layers', max(v['layer'] for v in o.values()) + 1)
    print('foundational layers', sorted((v['layer'], k) for k, v in o.items() if k.endswith(FND)))
    json.dump(o, open(T + 'order.json', 'w'), indent=1)
