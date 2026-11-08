"""Checks of schedule.v7.json (run 6's check6.py, against the committed schedule, and the
compact numbering of the todo waves) against the schedule schema's v6 and the
scheduling spec's Later stages: objectives by kind, one first stage per
entity, stage waves after the last translation waves of their freeing
modules, release points on every planned item, and the started waves
unchanged but for the v6 renames and release points."""
import json, collections, sys
R = '/target/libgit2-bindings/'
P = sys.argv[1] if len(sys.argv) > 1 else R + 'crustify/tmp/planner-run-7/schedule.v7.json'
import subprocess, os
D = json.load(open(P))
C = json.loads(subprocess.run(['git', '-C', R, 'show', os.environ.get('BASE', '547ab887f') + ':crustify/artifacts/schedule.json'], capture_output=True, check=True).stdout)
fail = []
assert D['schema_version'] == 6
W = {w['index']: w for w in D['waves']}
STAGE = {'port/layout', 'port/memory', 'port/unexport'}
uk = lambda i: (i['name'], i['defined_in'], i['kind'])
mk = lambda r: (r['link_unit'], r['module'], r['foundational'])
first = collections.Counter(); objs = collections.Counter()
regular_last = {}
for w in sorted(D['waves'], key=lambda w: (mk(w), w['sublayer'])):
    its = w['items'] if w['status'] == 'todo' else [dict(i, objective=b['kind']) for b in w['batches'] for i in b['items']]
    if any(i['objective'] in ('wrap', 'port') for i in its) or w['status'] != 'todo':
        regular_last[mk(w)] = w['index']
for w in D['waves']:
    for i in w['items']:
        for f in ('field_users', 'opaque_after', 'memory_unmanaged_after', 'unreferenced_after'):
            if f not in i: fail.append(('missing', f, w['index'], i['name']))
        if 'field_anchors' in i: fail.append(('v5 field', w['index'], i['name']))
        o = i['objective']
        if o in ('port/layout', 'port/memory') and (i['kind'] != 'type' or i['opaque_after'] is None):
            fail.append(('stage kind', w['index'], i['name'], o))
        if o == 'port/unexport' and (i['kind'] != 'symbol' or i['unreferenced_after'] is None):
            fail.append(('stage kind', w['index'], i['name'], o))
        objs[(uk(i), o)] += 1
        if o in ('wrap', 'port'): first[uk(i)] += 1
    for b in w['batches']:
        for i in b['items']:
            if set(i) != {'name', 'defined_in', 'kind', 'field_users', 'home'}: fail.append(('batch item', w['index'], i['name']))
    if w['status'] == 'todo':
        if w['batches'] or w['packed_with'] is not None: fail.append(('todo progress', w['index']))
        st = [i for i in w['items'] if i['objective'] in STAGE]
        if st:
            if len(st) != len(w['items']): fail.append(('mixed stage wave', w['index']))
            need = set()
            for i in st:
                p = i[{'port/layout': 'opaque_after', 'port/memory': 'memory_unmanaged_after', 'port/unexport': 'unreferenced_after'}[i['objective']]]
                need |= {regular_last[mk(m)] for m in p['closure']['modules'] if mk(m) != mk(w)}
            if not need <= set(w['depends_on']['scheduled']): fail.append(('stage deps', w['index'], sorted(need - set(w['depends_on']['scheduled']))))
dups = [k for k, n in objs.items() if n > 1 and k[1] != 'wrap']
fail += [('duplicate', k) for k in dups]
wrapdup = [k for k, n in objs.items() if n > 1 and k[1] == 'wrap']
# started waves: unchanged but for the renames and release points
old = {w['index']: w for w in C['waves'] if w['status'] != 'todo'}
strip = lambda i: {k: v for k, v in i.items() if k not in ('opaque_after', 'memory_unmanaged_after', 'unreferenced_after')}
for n, ow in old.items():
    nw = W[n]
    a = dict(nw, items=[strip(i) for i in nw['items']], batches=[dict(b, items=[strip(i) for i in b['items']]) for b in nw['batches']])
    b = dict(ow, items=[strip(i) for i in ow['items']], batches=[dict(bb, items=[strip(i) for i in bb['items']]) for bb in ow['batches']])
    if a != b: fail.append(('started changed', n))
# the todo waves: numbered compactly after the highest started index, in a
# topological order of depends_on.scheduled, each module's sublayers in index order
started = sorted(i for i, w in W.items() if w['status'] != 'todo')
todo = sorted(i for i, w in W.items() if w['status'] == 'todo')
if todo != list(range(started[-1] + 1, started[-1] + 1 + len(todo))): fail.append(('todo numbering', todo[:3]))
for i in todo:
    if any(d >= i for d in W[i]['depends_on']['scheduled']): fail.append(('not topological', i))
bym = collections.defaultdict(list)
for i in sorted(W): bym[mk(W[i])].append(W[i]['sublayer'])
for m, subs in bym.items():
    if subs != sorted(subs) or len(set(subs)) != len(subs): fail.append(('sublayers', m))
# every release point's wave is a wave of its item's
for w in D['waves']:
    for i in w['items']:
        for f in ('opaque_after', 'memory_unmanaged_after', 'unreferenced_after'):
            p = i[f]
            if p is None: continue
            for e in p['scheduled']['items']:
                if 'wave' in e and e['wave'] not in W: fail.append(('point wave', w['index'], i['name']))
# modules
mw = {i for m in D['modules'] for i in m['waves']}
if mw != set(W): fail.append(('module waves', len(mw ^ set(W))))
print('waves', len(W), 'wrap items planned twice (wrap held + ahead):', len(wrapdup))
print('FAIL' if fail else 'OK', fail[:20])
