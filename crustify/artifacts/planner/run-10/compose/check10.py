"""Checks of schedule.v10.json (run 9's check8.py, whose started-kind check
failed on run 9's output, a C kind looked up as a route: it now takes a C kind as
itself; and with run 10's checks: every stage wave also waits for its own
module's last translation wave, and the foundational modules' todo waves come
first. Run 9's was run 7's check7.py, with Crustify's granular item
kinds) against the schedule schema's v6, the task schema's items and the
scheduling spec's Later stages: objectives by kind, one first stage per
entity, stage waves after the last translation waves of their freeing
modules, release points on every planned item and only on the kinds that carry
them, typedefs on every item, the compact numbering of the todo waves, and the
started waves unchanged but for their items' kind and typedefs."""
import json, collections, sys
KINDS = {'struct', 'union', 'enum', 'function', 'global', 'macro', 'function_pointer', 'typedef'}
R = '/target/libgit2-bindings/'
P = sys.argv[1] if len(sys.argv) > 1 else R + 'crustify/tmp/planner-run-10/schedule.v10.json'
import subprocess, os
D = json.load(open(P))
C = json.loads(subprocess.run(['git', '-C', R, 'show', os.environ.get('BASE', 'HEAD') + ':crustify/artifacts/schedule.json'], capture_output=True, check=True).stdout)
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
        for f in ('field_users', 'typedefs', 'opaque_after', 'memory_unmanaged_after', 'unreferenced_after'):
            if f not in i: fail.append(('missing', f, w['index'], i['name']))
        if i['kind'] not in KINDS: fail.append(('kind', w['index'], i['name'], i['kind']))
        if i['field_users'] and i['kind'] not in ('struct', 'union', 'enum'): fail.append(('field_users', w['index'], i['name']))
        if i['typedefs'] and i['kind'] not in ('struct', 'union', 'enum', 'function_pointer'): fail.append(('typedefs', w['index'], i['name']))
        o = i['objective']
        if o == 'port/layout' and (i['kind'] not in ('struct', 'union', 'enum') or i['opaque_after'] is None):
            fail.append(('stage kind', w['index'], i['name'], o))
        if o == 'port/memory' and (i['kind'] not in ('struct', 'union') or i['memory_unmanaged_after'] is None):
            fail.append(('stage kind', w['index'], i['name'], o))
        if o == 'port/unexport' and (i['kind'] not in ('function', 'global', 'function_pointer') or i['unreferenced_after'] is None):
            fail.append(('stage kind', w['index'], i['name'], o))
        if w['status'] == 'todo':
            if (i['opaque_after'] is not None) != (i['kind'] in ('struct', 'union', 'enum')): fail.append(('opaque_after', w['index'], i['name']))
            if i['memory_unmanaged_after'] is not None and i['kind'] not in ('struct', 'union'): fail.append(('memory', w['index'], i['name']))
            if (i['unreferenced_after'] is not None) != (i['kind'] in ('function', 'global', 'macro', 'function_pointer', 'typedef')):
                fail.append(('unreferenced_after', w['index'], i['name']))
        objs[(uk(i), o)] += 1
        if o in ('wrap', 'port'): first[uk(i)] += 1
    for b in w['batches']:
        for i in b['items']:
            if set(i) != {'name', 'defined_in', 'kind', 'field_users', 'typedefs', 'home'} or i['kind'] not in KINDS:
                fail.append(('batch item', w['index'], i['name']))
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
# started waves: unchanged but for their items' kind and typedefs (run 8's task.md), and, in
# run 9, at the user's request, their items' release points
old = {w['index']: w for w in C['waves'] if w['status'] != 'todo'}
ROUTE = {'type': {'struct', 'union', 'enum'}, 'symbol': {'function', 'global', 'macro', 'typedef', 'function_pointer'},
         'callback': {'function_pointer'}}
for n, ow in old.items():
    for a, b in zip(W[n]['items'] + [i for bb in W[n]['batches'] for i in bb['items']],
                    ow['items'] + [i for bb in ow['batches'] for i in bb['items']]):
        if a['kind'] not in ROUTE.get(b['kind'], {b['kind']}): fail.append(('started kind', n, a['name'], b['kind'], a['kind']))
        if list(a) != ([k for k in b for k in ([k, 'typedefs'] if k == 'field_users' else [k])] if 'typedefs' not in b else list(b)): fail.append(('started fields', n, a['name']))
strip = lambda i: {k: v for k, v in i.items() if k not in ('kind', 'typedefs', 'opaque_after', 'memory_unmanaged_after', 'unreferenced_after')}
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
# run 10: a stage wave runs after its own module's integration, directly or through
# an earlier stage wave of its module
_anc = {}
def anc(i):
    if i not in _anc:
        _anc[i] = set()
        s = set()
        for d in W[i]['depends_on']['scheduled']: s |= {d} | anc(d)
        _anc[i] = s
    return _anc[i]
for w in sorted(D['waves'], key=lambda w: w['index']):
    if w['status'] == 'todo' and w['items'] and all(i['objective'] in STAGE for i in w['items']):
        if regular_last[mk(w)] not in anc(w['index']): fail.append(('stage after own', w['index']))
# run 10: the foundational modules' todo waves first, the string one last among them
ft = [i for i in todo if W[i]['foundational']]
if ft != todo[:len(ft)]: fail.append(('foundational first', ft[:3]))
if ft and W[ft[-1]]['module'] != 'src/util/strlist.c': fail.append(('string foundational last', ft[-1]))
# modules
mw = {i for m in D['modules'] for i in m['waves']}
if mw != set(W): fail.append(('module waves', len(mw ^ set(W))))
print('waves', len(W), 'wrap items planned twice (wrap held + ahead):', len(wrapdup))
print('FAIL' if fail else 'OK', fail[:20])
