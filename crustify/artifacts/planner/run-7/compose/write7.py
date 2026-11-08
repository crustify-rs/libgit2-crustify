"""Planner run 7's composition: schedule.json from wavefront's plan schedule.

Run 6's composition (../../run-6/compose/write6.py), with two changes:

- the later stages follow stages7.py: the C tests count as referrers of every
  function and global, tied as modules.json now ties them (ties.py), a test case
  leaving with its first subject module to be integrated and a helper with its
  file, and deps/clar, the harness, never;
- the todo waves are numbered after the highest index of a started wave, from
  the next integer, in one topological order of their depends_on.scheduled, with
  no gaps (the schedule schema's Indices section): the translation waves in
  wavefront's order of modules, layers and sublayers, and each later-stage wave
  right after the last of the waves it depends on.

The started waves keep their records; their items' release points are
recomputed, so that the waves they name follow the renumbering.

Inputs: crustify/tmp/planner-run-7/{schedule-port.json, users.json,
testfacts.json}, crustify/artifacts/modules.json, and the committed schedule
(547ab887f's, the commit this run forked from). Output: crustify/tmp/planner-run-7/{schedule.v7.json, wave-map.json}."""
import json, collections, re, sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import stages7 as ST

R = '/target/libgit2-bindings/'
T = R + 'crustify/tmp/planner-run-7/'
S = ST.S
M = ST.M
import subprocess
# the schedule this run forked from (547ab887f), or CURSCHED
CUR = (json.load(open(os.environ['CURSCHED'])) if 'CURSCHED' in os.environ else
       json.loads(subprocess.run(['git', '-C', R, 'show', '547ab887f:crustify/artifacts/schedule.json'],
                                 capture_output=True, check=True).stdout))
FND = ' (foundational)'
keyof = ST.keyof
def split(key):
    f = key.endswith(FND); k = key[:-len(FND)] if f else key
    lu, name = k.split('/', 1); return lu, name, f
MODS = {}
for lu in M['link_units']:
    for x in lu['modules']:
        MODS[keyof(lu['name'], x['name'], x['foundational'])] = x
OBJ = ST.OBJ
KW = {'as', 'break', 'const', 'continue', 'crate', 'else', 'enum', 'extern', 'false', 'fn', 'for', 'if', 'impl', 'in',
      'let', 'loop', 'match', 'mod', 'move', 'mut', 'pub', 'ref', 'return', 'self', 'static', 'struct', 'super', 'trait',
      'true', 'type', 'unsafe', 'use', 'where', 'while', 'async', 'await', 'dyn', 'abstract', 'become', 'box', 'do',
      'final', 'macro', 'override', 'priv', 'typeof', 'unsized', 'virtual', 'yield', 'try', 'gen'}
def home(key):
    lu, name, f = split(key)
    x = MODS[key]; p = name
    if x['translation_unit'] is None:
        d, _, stem = p.rpartition('/'); stem = stem.rsplit('.', 1)[0] + '_h'; p = (d + '/' if d else '') + stem
    else:
        p = p.rsplit('.', 1)[0]
    parts = [re.sub(r'[^A-Za-z0-9_]', '_', s) for s in p.split('/')]
    parts = [s + '_' if s in KW else s for s in parts]
    return f"src/{lu}/" + '/'.join(parts) + '.rs'
DEPRECATED_ANCHORS = {('git_attr_options', 'commit_id'), ('git_blob_filter_options', 'commit_id'),
                      ('git_filter_options', 'commit_id'), ('git_rebase_options', 'signing_cb'),
                      ('git_remote_callbacks', 'resolve_url'), ('git_remote_callbacks', 'update_tips')}
OUT_OF_SCOPE = {('git_url_resolve_cb', 'include/git2/remote.h')}
VISIBLE_SYM = {'function_exported', 'function_inline_header', 'global_extern', 'callback'}
def visible(it):
    if it['kind'] == 'type':
        if it['source_kind'] == 'enum': return False
        return any(f.endswith('.h') for f in it['declared_in'] + [it['defined_in']])
    return it['source_kind'] in VISIBLE_SYM or it['kind'] == 'callback'
def ukey(it): return '|'.join((it['name'], it['defined_in'], 'type' if it['kind'] == 'type' else 'symbol'))
UNITS = {}; WF_WAVE = {}
for w in S['waves']:
    for b in w['batches']:
        for it in b['items']:
            UNITS[ukey(it)] = it; WF_WAVE[ukey(it)] = w['index']
STAGES = ST.stages()

# ---- what has started
started = [w for w in CUR['waves'] if w['status'] != 'todo']
oldtodo = [w for w in CUR['waves'] if w['status'] == 'todo']
held = {}
started_mod = collections.defaultdict(list)
for w in started:
    started_mod[keyof(w['link_unit'], w['module'], w['foundational'])].append(w['index'])
    for b in w['batches']:
        for it in b['items']:
            held.setdefault(ukey(it), (b['kind'], w['index']))
def is_held(k, objective):
    return k in held and held[k][0] == objective

# ---- regular (translation) items per wavefront wave
def field_users(it):
    return [f for f in it.get('used_fields', []) if (it['name'], f) not in DEPRECATED_ANCHORS]
def planned(k, objective):
    it = UNITS[k]
    return {'name': it['name'], 'defined_in': it['defined_in'], 'kind': it['kind'], 'field_users': field_users(it),
            'home': home(it['module']), 'objective': objective}
regular = []
placed = {k: ('old', oi) for k, (_, oi) in held.items()}
stage_items = collections.defaultdict(list)    # (module, frozenset freeing) -> [(objective, k)]
never = collections.defaultdict(list)          # objective -> [(k, keepers)]
for w in S['waves']:
    items = []
    for b in w['batches']:
        for it in b['items']:
            k = ukey(it); m = it['module']
            if m is None or (it['name'], it['defined_in']) in OUT_OF_SCOPE:
                continue
            if OBJ[m] == 'wrap':
                if not visible(it) or is_held(k, 'wrap'):
                    continue
                items.append(planned(k, 'wrap')); placed[k] = ('new', w['index']); continue
            st = STAGES.get(k, {})
            if 'port/layout' in st and st['port/layout'][0] != 'none':
                if not is_held(k, 'wrap'):
                    items.append(planned(k, 'wrap')); placed[k] = ('new', w['index'])
            elif not is_held(k, 'port'):
                items.append(planned(k, 'port')); placed[k] = ('new', w['index'])
            for o, (verdict, freeing, keepers) in st.items():
                if verdict == 'plan':
                    stage_items[(m, frozenset(freeing))].append((o, k))
                elif verdict == 'never':
                    never[o].append((k, keepers))
    if items:
        regular.append((w, items))
layer = {(m['link_unit'], m['module'], m['foundational']): m['layer'] for m in CUR['modules']}
def mlayer(m): return layer[split(m)]
regular.sort(key=lambda p: (mlayer(p[0]['module']), p[0]['sublayer'], p[0]['index']))

# ---- indices: provisional, then matched against the todo waves
base = max(w['index'] for w in CUR['waves']) + 1
new = []
s2n = {}
for w, items in regular:
    idx = base + len(new); s2n[w['index']] = idx
    new.append({'_m': w['module'], '_S': w['index'], 'index': idx, 'items': items})
def unit_wave(k):
    p = placed.get(k)
    if not p: return None
    return s2n.get(p[1]) if p[0] == 'new' else p[1]
last_regular = {}
for m, l in started_mod.items(): last_regular[m] = l[-1]
for nw in new: last_regular[nw['_m']] = nw['index']
pos = {nw['index']: i for i, nw in enumerate(new)}
def lastpos(m):
    i = last_regular.get(m)
    return pos.get(i, -1)
# stage waves, each module's ordered by when the last of its freeing modules is integrated
smods = collections.defaultdict(list)
for (m, fr), lst in stage_items.items():
    smods[m].append((fr, lst))
stagew = []
for m in sorted(smods, key=lambda m: (mlayer(m), m)):
    for fr, lst in sorted(smods[m], key=lambda p: (max(lastpos(x) for x in p[0]), sorted(p[0]))):
        idx = base + len(new) + len(stagew)
        items = [planned(k, o) for o, k in sorted(lst, key=lambda p: (['port/layout', 'port/memory', 'port/unexport'].index(p[0]), p[1]))]
        stagew.append({'_m': m, '_S': None, '_after': sorted(fr), 'index': idx, 'items': items})
allnew = new + stagew
for i, nw in enumerate(allnew): pos[nw['index']] = i

# ---- wave records
prev = {m: l[-1] for m, l in started_mod.items()}
sub = collections.defaultdict(lambda: -1)
for w in started:
    k = keyof(w['link_unit'], w['module'], w['foundational']); sub[k] = max(sub[k], w['sublayer'])
names = collections.defaultdict(list)
for k in UNITS: names[tuple(k.split('|')[:2])].append(k)
out = []
for nw in allnew:
    m = nw['_m']; lu, name, f = split(m)
    sched = set(); cut = set()
    if m in prev: sched.add(prev[m])
    if nw['_S'] is not None:
        for it in nw['items']:
            u = UNITS[ukey(it)]
            for part in ('scheduled', 'cut'):
                for route in ('types', 'symbols'):
                    for r in u['depends_on'][part][route]:
                        if r['scope'] != 'scheduled': continue
                        for t in names.get((r['name'], r['defined_in']), ()):
                            if (t.endswith('|type')) != (route == 'types'): continue
                            wi = unit_wave(t)
                            if wi is None or wi == nw['index']: continue
                            if wi < base or pos[wi] < pos[nw['index']]: sched.add(wi)
                            else: cut.add(wi)
    else:
        for x in nw['_after']:
            sched.add(last_regular[x])
    sub[m] += 1
    out.append({'index': nw['index'], 'link_unit': lu, 'module': name, 'foundational': f, 'layer': mlayer(m),
                'sublayer': sub[m], 'status': 'todo',
                'depends_on': {'scheduled': sorted(sched), 'cut': sorted(cut - sched)},
                'items': nw['items'], 'packed_with': None, 'batches': []})
    prev[m] = nw['index']

# ---- the todo waves numbered compactly, in one topological order
first = max(w['index'] for w in started) + 1
byidx = {w['index']: w for w in out}
provpos = {w['index']: i for i, w in enumerate(out)}
key = {}
for w in out:                                   # `out` is topological: translation waves, then stage waves
    deps = [d for d in w['depends_on']['scheduled'] if d in byidx]
    if w['items'] and w['items'][0]['objective'] in ('port/layout', 'port/memory', 'port/unexport'):
        key[w['index']] = (max(key[d][0] for d in deps) if deps else -1, 1, provpos[w['index']])
    else:
        key[w['index']] = (provpos[w['index']], 0, provpos[w['index']])
order = sorted(out, key=lambda w: key[w['index']])
final = {w['index']: first + i for i, w in enumerate(order)}
oldtodo_idx = sorted(w['index'] for w in oldtodo)
fmap = lambda i: final.get(i, i)
for w in out:
    w['index'] = fmap(w['index'])
    w['depends_on'] = {p: sorted(fmap(d) for d in w['depends_on'][p]) for p in ('scheduled', 'cut')}
out.sort(key=lambda w: w['index'])
pos2 = {w['index']: w for w in out}
for w in out:
    for d in w['depends_on']['scheduled']:
        assert d < w['index'], ('not topological', w['index'], d)
placed_final = {k: fmap(unit_wave(k)) if unit_wave(k) is not None else None for k in UNITS}

# ---- release points, renumbered
def modrec(s):
    lu, name, f = split(s); return {'link_unit': lu, 'module': name, 'foundational': f}
def entry(e):
    r = {'name': e['name'], 'defined_in': e['defined_in'], 'kind': e['kind'],
         'module': modrec(e['module']) if e.get('module') else None}
    w = placed_final.get('|'.join((e['name'], e['defined_in'], e['kind'])))
    if w is not None: r['wave'] = w
    return r
def point(p):
    if p is None: return None
    c, s = p['closure'], p['scheduled']
    sitems = [entry(e) for e in s['items']]
    return {'closure': {'users': c['users'], 'items': [entry(e) for e in c['items']],
                        'modules': [modrec(x) for x in c['modules']] if c['modules'] is not None else None},
            'scheduled': {'users': s['users'], 'items': sitems,
                          'waves': sorted({e['wave'] for e in sitems if 'wave' in e}),
                          'modules': [modrec(x) for x in s['modules']] if s['modules'] is not None else None}}
RP = {}
def points(i):
    k = ukey(i)
    if k not in RP:
        u = UNITS.get(k, {})
        RP[k] = {f: point(u.get(f)) for f in ('opaque_after', 'memory_unmanaged_after', 'unreferenced_after')}
    return RP[k]
def v6(i, with_points=True):
    i = dict(i)
    if 'field_anchors' in i: i['field_users'] = i.pop('field_anchors')
    order = ['name', 'defined_in', 'kind', 'field_users', 'home', 'objective']
    r = {f: i[f] for f in order if f in i}
    if with_points: r.update(points(i))
    return r
waves = []
for w in started:
    w = dict(w)
    w['items'] = [v6(i) for i in w['items']]
    w['batches'] = [dict(b, items=[v6(i, False) for i in b['items']]) for b in w['batches']]
    waves.append(w)
for w in out:
    w['items'] = [v6(i) for i in w['items']]
    waves.append(w)
waves.sort(key=lambda w: w['index'])

# ---- modules: the order is run 5's, unchanged; each module's waves
modwaves = collections.defaultdict(list)
for w in waves: modwaves[(w['link_unit'], w['module'], w['foundational'])].append(w['index'])
mods = []
for m in CUR['modules']:
    m = dict(m); m['waves'] = sorted(modwaves[(m['link_unit'], m['module'], m['foundational'])]); mods.append(m)
COMMENT = ["The libgit2 port campaign's order of modules and its waves, schema 6, as planner run 4 planned them", 'over the whole target, the libgit2.so link unit and the git2 CLI executable, run 5 corrected them,', 'run 6 converted them to schema 6 with the later stages of a port campaign, and run 7 re-derived the', 'stages the C tests hold back from the tests as modules.json now ties them and numbered the todo', 'waves compactly. modules and the todo waves come from one wavefront schedule of', "crustify/wavefront/configs/wavefront-config.json in its default, body-deep mode, every module's", 'files seeded, enums scheduled (--with-enums), no batch caps; modules are listed by layer. One', "departure from wavefront's order: each foundational module's uses of other modules", "(src/util/errors.c's git_error_set_oom and git_error_set on alloc.c's, strlist.c's and", "win32_leakcheck.c's failure paths) are cut, so the foundational modules come first, and every layer", "is the longest path over the scheduled dependencies that remain. Waves 0-168 are the wrap campaign's", 'that had started (88 done, 1 wip): kept as they ran, their items those of their batches, each with', "its batch's kind as objective; the 17 wip waves of port modules were closed with the batches that", "had landed, at the user's request. The todo waves have no batches, and are numbered from 169, after", 'the highest started index, in one topological order of their scheduled dependencies: the translation', "waves in wavefront's order, each later-stage wave right after the last wave it depends on. Each", "item's objective follows its module's: in the seven foundational modules, which are wrap in every", 'campaign, the items other modules can reach (exported and header-inline functions, extern globals,', 'callbacks, and the types a header declares) are wrap, their statics and enums planned for none; in', 'every other module, which the port campaign ports, every function, global, callback, enum and type', 'generator is port where the order places it, and a struct or union is port there when no layout user', 'of another module remains (its opaque_after names no other module), wrap otherwise. The later stages', "follow each item's release points, which every planned item carries as wavefront 2a95e3c records", "them, their waves renumbered to this schedule's and their modules written {link_unit, module,", "foundational}: port/layout after a type's opaque_after, port/memory after its", "memory_unmanaged_after, port/unexport after a ported function's or global's unreferenced_after, each", "when the point names a module other than the item's own, and none when a user stays C in a", 'foundational module. The C tests count as referrers of every function and global too, though', "wavefront's closure leaves them out: a clar test case leaves with the first of the modules", "modules.json ties it to to be integrated, and a test file's helpers, initialize and cleanup with its", "last test case, a helper file's with the last test that reaches it, so a stage a test still calls", 'also waits for one of those modules; deps/clar, the harness every remaining test links, never', 'leaves, and keeps the exports it reaches for good. The stages the same integrations free share a', "wave of the item's module, after its translation waves, which depends on the last translation wave", "of each freeing module, the run that integrates its C. An item a started wave's batches hold, or", 'that a batch landed, with that objective is not planned again, and a dependency on it is one on that', "wave. A translation wave's depends_on is recomputed from its items' unit edges: scheduled, its", "module's previous wave and the earlier waves holding their dependencies; cut, the later ones. Out of", 'scope: the deprecated git_url_resolve_cb and the GIT_DEPRECATE_HARD-only field users (commit_id,', "signing_cb, resolve_url, update_tips). A module's external records count its units' references to", 'out-of-tree entities, macros and compiler builtins left out, by the library whose installed headers', "define them. Homes follow the conventions: src/<link-unit>/<TU path>.rs, a header module's", "<stem>_h.rs. Planner run 7's composition is crustify/artifacts/planner/run-7/compose/."]
doc = {'_comment': COMMENT, 'schema_version': 6, 'modules': mods, 'waves': waves}

# ---- checks: every dependency resolves, and the scheduled waves are acyclic
idx = {w['index']: w for w in waves}
for w in waves:
    for d in w['depends_on']['scheduled'] + w['depends_on']['cut']:
        assert d in idx, (w['index'], d)
state = {}
sys.setrecursionlimit(100000)
def visit(i):
    if state.get(i) == 1: raise SystemExit(f'cycle at {i}')
    if state.get(i) == 2: return
    state[i] = 1
    for d in idx[i]['depends_on']['scheduled']: visit(d)
    state[i] = 2
for i in idx: visit(i)
assert sum(len(m['waves']) for m in mods) == len(waves)

json.dump(doc, open(T + 'schedule.v7.json', 'w'), indent=2)
json.dump({'todo_before': [oldtodo_idx[0], oldtodo_idx[-1], len(oldtodo_idx)], 'todo_after': [first, first + len(out) - 1, len(out)],
           'never': {o: [(k, kp) for k, kp in v] for o, v in never.items()}}, open(T + 'wave-map.json', 'w'), indent=1)
print('waves', len(waves), 'todo', len(out), 'numbered', first, '-', first + len(out) - 1, 'were', oldtodo_idx[0], '-', oldtodo_idx[-1])
print('todo items', collections.Counter(i['objective'] for w in out for i in w['items']))
print('stage waves', len(stagew))

# ---- written as json.dump(indent=2) would, but for a release point's item and
# module records, one per line
def compact(o):
    return isinstance(o, dict) and (set(o) in ({'link_unit', 'module', 'foundational'},)
                                    or ('defined_in' in o and 'module' in o and 'home' not in o))
def enc(o, level=0):
    pad = '  ' * level
    if compact(o):
        return json.dumps(o, ensure_ascii=False)
    if isinstance(o, dict):
        if not o: return '{}'
        inner = ',\n'.join(f'{pad}  {json.dumps(k)}: {enc(v, level + 1)}' for k, v in o.items())
        return '{\n' + inner + '\n' + pad + '}'
    if isinstance(o, list):
        if not o: return '[]'
        return '[\n' + ',\n'.join(f'{pad}  {enc(v, level + 1)}' for v in o) + '\n' + pad + ']'
    return json.dumps(o, ensure_ascii=False)
open(T + 'schedule.v7.json', 'w').write(enc(doc) + '\n')
assert json.load(open(T + 'schedule.v7.json')) == doc
