"""Planner run 6's composition: schedule.json v6 from wavefront's plan schedule.

Run 5's composition (../../run-5/compose/), with three changes:

- each type's first stage, and its later stages, follow wavefront's release
  points (stages.py) rather than run 4's own layout query: `port` where the
  order places it when no layout user of another module remains, `wrap`
  otherwise, then `port/layout` after its opaque_after and `port/memory` after
  its memory_unmanaged_after; a ported function's or global's `port/unexport`
  after its unreferenced_after;
- the later stages an entity calls for share a wave of its module with the
  stages the same integrations free: one wave per module and set of freeing
  modules, after the module's translation waves, depending on the last
  translation wave of each freeing module, the run that integrates its C;
- every planned item is v6: `field_users`, and its entity's release points,
  their waves renumbered to schedule.json's and their modules written as
  {link_unit, module, foundational}.

The started waves keep their records; their planned items and their batches'
items are converted to v6 (field_anchors renamed field_users; planned items
gain their release points), nothing else. A todo wave whose items the
composition reproduces keeps its index; a v5 second `port` item of a type,
which the type's wrap precedes, is read as `port/layout` for that match.

Inputs: crustify/tmp/planner-run-6/{schedule-port.json, users.json}, and
crustify/artifacts/modules.json, and the schedule before run 6 (22a6e9493's). Output:
crustify/tmp/planner-run-6/{schedule.v6.json, wave-map.json}."""
import json, collections, re, sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import stages as ST

R = '/target/libgit2-bindings/'
T = R + 'crustify/tmp/planner-run-6/'
S = ST.S
M = ST.M
import subprocess
# the schedule before run 6 (its parent commit), or CURSCHED
CUR = (json.load(open(os.environ['CURSCHED'])) if 'CURSCHED' in os.environ else
       json.loads(subprocess.run(['git', '-C', R, 'show', '22a6e9493:crustify/artifacts/schedule.json'],
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

# ---- keep the indices of the todo waves the composition reproduces
second_port = set()
wrapped = {ukey(i) for w in CUR['waves'] for i in w['items'] if i['objective'] == 'wrap'}
for w in oldtodo:
    for i in w['items']:
        if (i['kind'] == 'type' and i['objective'] == 'port' and ukey(i) in wrapped
                and STAGES.get(ukey(i), {}).get('port/layout', ('none',))[0] == 'plan'):
            second_port.add((w['index'], ukey(i)))
def mkey(w, old=False):
    def obj(i):
        if old and (w['index'], ukey(i)) in second_port: return 'port/layout'
        return i['objective']
    return (w['link_unit'], w['module'], w['foundational'],
            frozenset((i['name'], i['defined_in'], i['kind'], obj(i), tuple(i['field_anchors' if old else 'field_users']), i['home']) for i in w['items']))
oldby = {}
for w in oldtodo:
    oldby.setdefault(mkey(w, True), w['index'])
nxt = base
final = {}; kept = []; appended = []
for w in out:
    k = mkey(w)
    if k in oldby and oldby[k] not in kept:
        final[w['index']] = oldby[k]; kept.append(oldby[k])
    else:
        final[w['index']] = nxt; appended.append(nxt); nxt += 1
removed = sorted(set(w['index'] for w in oldtodo) - set(kept))
fmap = lambda i: final.get(i, i)
for w in out:
    w['index'] = fmap(w['index'])
    w['depends_on'] = {p: sorted(fmap(d) for d in w['depends_on'][p]) for p in ('scheduled', 'cut')}
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
COMMENT = ["The libgit2 port campaign's order of modules and its waves, schema 6, as planner run 4 planned them", 'over the whole target, the libgit2.so link unit and the git2 CLI executable, run 5 corrected them and', 'run 6 converted them to schema 6 with the later stages of a port campaign. modules and the todo waves', 'come from one wavefront schedule of crustify/wavefront/configs/wavefront-config.json in its default,', "body-deep mode, every module's files seeded, enums scheduled (--with-enums), no batch caps; modules", "are listed by layer. One departure from wavefront's order: each foundational module's uses of other", "modules (src/util/errors.c's git_error_set_oom and git_error_set on alloc.c's, strlist.c's and", "win32_leakcheck.c's failure paths) are cut, so the foundational modules come first, and every layer", "is the longest path over the scheduled dependencies that remain. Waves 0-168 are the wrap campaign's", 'that had started (88 done, 1 wip): kept as they ran, their items those of their batches, each with', "its batch's kind as objective; the 17 wip waves of port modules were closed with the batches that", "had landed, at the user's request. The todo waves have no batches. Each item's objective follows its", "module's: in the seven foundational modules, which are wrap in every campaign, the items other", 'modules can reach (exported and header-inline functions, extern globals, callbacks, and the types a', 'header declares) are wrap, their statics and enums planned for none; in every other module, which', 'the port campaign ports, every function, global, callback, enum and type generator is port where the', 'order places it, and a struct or union is port there when no layout user of another module remains', "(its opaque_after names no other module), wrap otherwise. The later stages follow each item's release", 'points, which every planned item carries as wavefront 2a95e3c records them, their waves renumbered to', "this schedule's and their modules written {link_unit, module, foundational}: port/layout after a", "type's opaque_after, port/memory after its memory_unmanaged_after, port/unexport after a ported", "function's or global's unreferenced_after, each when the point names a module other than the item's", 'own, and none when a user stays C in a foundational module. The C tests count as referrers too,', "though wavefront's closure leaves them out: a test file tied to modules (modules.json tests) leaves", 'with the first of them to be integrated, so a stage the test still calls also waits for one of them,', 'and a test tied to no port module keeps the export for good. The stages the same integrations free', "share a wave of the item's module, after its translation waves, which depends on the last", "translation wave of each freeing module, the run that integrates its C. An item a started wave's", 'batches hold, or that a batch landed, with that objective is not planned again, and a dependency on', "it is one on that wave. A translation wave's depends_on is recomputed from its items' unit edges:", "scheduled, its module's previous wave and the earlier waves holding their dependencies; cut, the", 'later ones. Out of scope: the deprecated git_url_resolve_cb and the GIT_DEPRECATE_HARD-only field', "users (commit_id, signing_cb, resolve_url, update_tips). A module's external records count its units'", 'references to out-of-tree entities, macros and compiler builtins left out, by the library whose', 'installed headers define them. Homes follow the conventions: src/<link-unit>/<TU path>.rs, a header', "module's <stem>_h.rs. Planner run 6's composition is crustify/artifacts/planner/run-6/compose/."]
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

json.dump(doc, open(T + 'schedule.v6.json', 'w'), indent=2)
json.dump({'kept': sorted(kept), 'appended': appended, 'removed': removed,
           'never': {o: [(k, kp) for k, kp in v] for o, v in never.items()}}, open(T + 'wave-map.json', 'w'), indent=1)
print('waves', len(waves), 'todo', len(out), 'kept', len(kept), 'appended', len(appended),
      appended[:1], appended[-1:], 'removed', len(removed))
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
open(T + 'schedule.v6.json', 'w').write(enc(doc) + '\n')
assert json.load(open(T + 'schedule.v6.json')) == doc
