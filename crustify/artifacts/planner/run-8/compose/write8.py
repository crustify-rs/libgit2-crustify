"""Planner run 8's composition: schedule.json from wavefront 88793d7's plan schedule.

Run 7's write7.py, brought to wavefront's schema v6 and Crustify's granular item
kinds (crustify b47995e):

- every item names its C kind (struct, union, enum, function, global, macro,
  function_pointer, typedef) and carries the typedefs that fold into it;
- the macros, the function pointers written inline and the typedefs of their
  own are items where the order places them: port in a port module; in a
  foundational module, wrap when other modules can reach them (a header defines
  them), as for its other items. The macros that are not API (not_api.json) are
  neither homed nor translated;
- an enum is an item of its module, port in a port module and wrap when a
  foundational header defines it, and a closed enum (enums8.py) takes a
  port/layout after its opaque_after, as a struct's does;
- a function pointer takes a port/unexport after its unreferenced_after, as a
  function's does (stages8.py); a macro's or typedef's release point places none;
- the order of the modules is order8.py's: wavefront's, the foundational
  modules' dependencies on other modules cut first;
- the started waves (88 done, wave 12 wip) take their items' C kinds and
  typedefs, in place, at the user's request (task.md): nothing else of them
  changes, their release points included;
- the todo waves are composed anew, numbered after the highest started index,
  in one topological order: the translation waves in the order of the modules,
  each later-stage wave right after the last of the waves it depends on.

Inputs: crustify/tmp/planner-run-8/{schedule-port.json, users.json, order.json,
enums.json, testfacts.json}, crustify/artifacts/modules.json, the committed
schedule (HEAD). Outputs: crustify/tmp/planner-run-8/{schedule.v8.json, wave-map.json}."""
import json, collections, re, sys, os, subprocess, csv
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, '../../run-5/compose'))
import stages8 as ST
import deps as D5                      # run 5's lib_of: the library whose installed headers define an entity

R = '/target/libgit2-bindings/'
T = R + 'crustify/tmp/planner-run-8/'
S = ST.S
M = ST.M
CUR = json.loads(subprocess.run(['git', '-C', R, 'show', 'HEAD:crustify/artifacts/schedule.json'],
                                capture_output=True, check=True).stdout)
ORDER = ST.ORDER
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
NA = json.load(open(os.path.join(HERE, 'not_api.json')))
NOT_API = {tuple(x) for k in ('attribute', 'configuration') for x in NA[k]}
TYPE = ('struct', 'union', 'enum')
def header(it): return any(f.endswith('.h') for f in it['declared_in'] + [it['defined_in']])
def visible(it):
    """What other modules can reach of a foundational module, which binds it."""
    if it['kind'] == 'function':
        return not it['stats'].get('static') or it['defined_in'].endswith('.h')
    if it['kind'] == 'global':
        return not it['stats'].get('static')
    if it['kind'] == 'macro':
        return it['defined_in'].endswith('.h')
    return header(it)
def ukey(it): return '|'.join((it['name'], it['defined_in'], it['kind']))
UNITS = {}; WF_WAVE = {}
for w in S['waves']:
    for b in w['batches']:
        for it in b['items']:
            UNITS[ukey(it)] = it; WF_WAVE[ukey(it)] = w['index']
STAGES = ST.stages()
ENUMS = ST.ENUMS

# ---- the started waves' items in their C kinds
ENT = collections.defaultdict(list)
for line in open(T + 'entities.txt'):
    n, k, d, decl = line.rstrip('\n').split('\t')
    for f in ([d] if d else decl.split(',')):
        ENT[(n, f)].append(k)
ROUTE = {'type': set(TYPE), 'symbol': {'function', 'global', 'macro', 'typedef', 'function_pointer'},
         'callback': {'function_pointer'}}
def ckind(name, f, route):
    if route not in ROUTE:
        return route
    ks = sorted({k for k in [u.split('|')[2] for u in UNITS if u.startswith(f'{name}|{f}|')] if k in ROUTE[route]}
                or {k for k in ENT.get((name, f), ()) if k in ROUTE[route]})
    assert len(ks) == 1, (name, f, route, ks)
    return ks[0]
def typedefs_of(name, f, kind):
    u = UNITS.get('|'.join((name, f, kind)))
    return list(u['typedefs']) if u else []
def migrate(i):
    """A started item, its kind and typedefs taken in place; nothing else changes."""
    k = ckind(i['name'], i['defined_in'], i['kind'])
    out = {}
    for f, v in i.items():
        out[f] = k if f == 'kind' else v
        if f == 'field_users':
            out['typedefs'] = typedefs_of(i['name'], i['defined_in'], k)
    return out
started = []
for w in CUR['waves']:
    if w['status'] == 'todo':
        continue
    w = dict(w)
    w['items'] = [migrate(i) for i in w['items']]
    w['batches'] = [dict(b, items=[migrate(i) for i in b['items']]) for b in w['batches']]
    started.append(w)
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

# ---- translation items per wavefront wave
def field_users(it):
    if it['kind'] not in ('struct', 'union'):
        return []
    return [f for f in it.get('used_fields', []) if (it['name'], f) not in DEPRECATED_ANCHORS]
def planned(k, objective):
    it = UNITS[k]
    return {'name': it['name'], 'defined_in': it['defined_in'], 'kind': it['kind'], 'field_users': field_users(it),
            'typedefs': list(it['typedefs']), 'home': home(it['module']), 'objective': objective}
regular = []
placed = {k: ('old', oi) for k, (_, oi) in held.items()}
stage_items = collections.defaultdict(list)    # (module, frozenset freeing) -> [(objective, k)]
never = collections.defaultdict(list)          # objective -> [(k, keepers)]
skipped = collections.Counter()
for w in S['waves']:
    items = []
    for b in w['batches']:
        for it in b['items']:
            k = ukey(it); m = it['module']
            if m is None or (it['name'], it['defined_in']) in OUT_OF_SCOPE:
                skipped['no module'] += 1; continue
            if it['kind'] == 'macro' and (it['name'], it['defined_in']) in NOT_API:
                skipped['not API'] += 1; continue
            if OBJ[m] == 'wrap':
                if not visible(it):
                    skipped['foundational, unreachable'] += 1; continue
                if is_held(k, 'wrap'):
                    continue
                items.append(planned(k, 'wrap')); placed[k] = ('new', w['index']); continue
            st = STAGES.get(k, {})
            if it['kind'] in ('struct', 'union') and st['port/layout'][0] != 'none':
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
MIDX = {m['name']: i for i, m in enumerate(S['modules'])}
def mlayer(m): return ORDER[m]['layer']
regular.sort(key=lambda p: (mlayer(p[0]['module']), MIDX[p[0]['module']], p[0]['sublayer'], p[0]['index']))

# ---- indices: provisional, then numbered compactly
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
    return pos.get(last_regular.get(m), -1)
smods = collections.defaultdict(list)
for (m, fr), lst in stage_items.items():
    smods[m].append((fr, lst))
SORDER = ['port/layout', 'port/memory', 'port/unexport']
stagew = []
for m in sorted(smods, key=lambda m: (mlayer(m), MIDX[m])):
    for fr, lst in sorted(smods[m], key=lambda p: (max(lastpos(x) for x in p[0]), sorted(p[0]))):
        idx = base + len(new) + len(stagew)
        items = [planned(k, o) for o, k in sorted(lst, key=lambda p: (SORDER.index(p[0]), p[1]))]
        stagew.append({'_m': m, '_S': None, '_after': sorted(fr), 'index': idx, 'items': items})
allnew = new + stagew
for i, nw in enumerate(allnew): pos[nw['index']] = i

# ---- wave records
prev = {m: l[-1] for m, l in started_mod.items()}
sub = collections.defaultdict(lambda: -1)
for w in started:
    k = keyof(w['link_unit'], w['module'], w['foundational']); sub[k] = max(sub[k], w['sublayer'])
out = []
for nw in allnew:
    m = nw['_m']; lu, name, f = split(m)
    sched = set(); cut = set()
    if m in prev: sched.add(prev[m])
    if nw['_S'] is not None:
        for it in nw['items']:
            u = UNITS[ukey(it)]
            for part in ('scheduled', 'cut'):
                for r in u['depends_on'][part]:
                    if r['scope'] != 'scheduled': continue
                    tk = '|'.join((r['name'], r['defined_in'], r['kind']))
                    wi = unit_wave(tk)
                    if wi is None or wi == nw['index']: continue
                    # a started wave has run; across modules, the order of the modules decides,
                    # within one, the waves' order
                    if wi < base: sched.add(wi)
                    elif UNITS[tk]['module'] != m:
                        if UNITS[tk]['module'] in ORDER[m]['cut']: cut.add(wi)
                        else:
                            assert pos[wi] < pos[nw['index']], (nw['index'], tk)
                            sched.add(wi)
                    elif pos[wi] < pos[nw['index']]: sched.add(wi)
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
    if w['items'][0]['objective'] in SORDER:
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
def with_points(i):
    u = UNITS[ukey(i)]
    r = dict(i)
    r.update({f: point(u.get(f)) for f in ('opaque_after', 'memory_unmanaged_after', 'unreferenced_after')})
    return r
for w in out:
    w['items'] = [with_points(i) for i in w['items']]
waves = sorted(started + out, key=lambda w: w['index'])

# ---- modules: order8.py's order, each module's waves and dependency records
modwaves = collections.defaultdict(list)
for w in waves: modwaves[keyof(w['link_unit'], w['module'], w['foundational'])].append(w['index'])
WFREC = {m['name']: m for m in S['modules']}
def unit_edges(src, dst):
    """Every unit edge from src's units to dst's, each `to` with its C kind."""
    es = []
    for k, it in UNITS.items():
        if it['module'] != src: continue
        for part in ('scheduled', 'cut'):
            for r in it['depends_on'][part]:
                if r['scope'] != 'scheduled': continue
                t = UNITS.get('|'.join((r['name'], r['defined_in'], r['kind'])))
                if t and t['module'] == dst:
                    es.append({'from': {'name': it['name'], 'defined_in': it['defined_in']},
                               'to': {'name': t['name'], 'defined_in': t['defined_in']}, 'kind': t['kind']})
    return sorted({json.dumps(e, sort_keys=True) for e in es})
def deprec(m, d, with_items):
    lu, name, f = split(d)
    wf = {x['module']: x for p in ('scheduled', 'cut') for x in WFREC[m]['depends_on'][p]}[d]
    r = {'link_unit': lu, 'module': name, 'foundational': f, 'unique_edges': wf['unique_edges'], 'total_edges': wf['total_edges']}
    if with_items:
        r['items'] = [json.loads(e) for e in unit_edges(m, d)]
        assert len(r['items']) == wf['total_edges'], (m, d, len(r['items']), wf['total_edges'])
    return r
ext = collections.defaultdict(lambda: collections.defaultdict(set))
for k, it in UNITS.items():
    if it['module'] is None: continue
    for part in ('scheduled', 'cut'):
        for r in it['depends_on'][part]:
            if r['scope'] != 'external': continue
            lib = D5.lib_of(r['defined_in'])
            if not lib: continue                         # compiler builtins
            ext[it['module']][lib].add(json.dumps({'from': {'name': it['name'], 'defined_in': it['defined_in']},
                                                   'to': {'name': r['name'], 'defined_in': r['defined_in']},
                                                   'kind': r['kind']}, sort_keys=True))
def extrecs(m):
    recs = []
    for lib in sorted(ext[m]):
        items = [json.loads(e) for e in sorted(ext[m][lib])]
        recs.append({'library': lib, 'unique_edges': len({(e['to']['name'], e['to']['defined_in']) for e in items}),
                     'total_edges': len(items), 'items': items})
    return recs
mods = []
for m in sorted(ORDER, key=lambda m: (ORDER[m]['layer'], MIDX[m])):
    if not modwaves.get(m): continue
    lu, name, f = split(m)
    mods.append({'link_unit': lu, 'module': name, 'foundational': f, 'layer': ORDER[m]['layer'],
                 'waves': sorted(modwaves[m]),
                 'depends_on': {'scheduled': [deprec(m, d, False) for d in ORDER[m]['scheduled']],
                                'cut': [deprec(m, d, True) for d in ORDER[m]['cut']],
                                'external': extrecs(m)}})
nowaves = sorted(m for m in ORDER if not modwaves.get(m))

COMMENT = [
    "The libgit2 port campaign's order of modules and its waves, schema 6, as planner run 4 planned them over the whole "
    "target, the libgit2.so link unit and the git2 CLI executable, runs 5 to 7 corrected them, and run 8 re-planned them "
    "with wavefront 88793d7 and Crustify's granular item kinds. modules and the todo waves come from one wavefront schedule "
    "of crustify/wavefront/configs/wavefront-config.json in its default, body-deep mode, every module's files seeded, no "
    "batch caps; modules are listed by layer. Every entity is scheduled under its C kind (struct, union, enum, function, "
    "global, macro, function_pointer, typedef), the typedefs that name a struct, union, enum or function pointer folded "
    "into its item's typedefs; the macros that are not API (attribute, linkage and configuration macros, "
    "crustify/artifacts/planner/run-8/compose/not_api.json) are not planned. The order of the modules is wavefront's, "
    "but that each foundational module's dependencies on other modules are cut before the cycles are, so that the "
    "foundational modules come first; every layer is the longest path over the scheduled dependencies that remain.",
    "Waves 0-168 are the wrap campaign's that had started (88 done, 1 wip): kept as they ran, their items those of their "
    "batches, each with its batch's kind as objective; run 8 gave their items, and their batches' items, their C kinds "
    "and typedefs in place, at the user's request, and changed nothing else of them, their release points included, "
    "which still name the waves as run 7 numbered them. The 17 wip waves of port modules were closed with the batches "
    "that had landed, at the user's request. The todo waves have no batches, and are numbered from 169, after the "
    "highest started index, in one topological order of their scheduled dependencies: the translation waves in the "
    "order of the modules, each later-stage wave right after the last wave it depends on.",
    "Each item's objective follows its module's: in the seven foundational modules, which are wrap in every campaign, "
    "the items other modules can reach (functions with external linkage or defined in a header, extern globals, and "
    "the types, enums, function pointers, typedefs and macros a header defines) are wrap, the others planned for none; "
    "in every other module, which the port campaign ports, every function, global, enum, function pointer, typedef "
    "and macro is port where the order places it, and a struct or union is port there when no layout user of another "
    "module remains (its opaque_after names no other module), wrap otherwise. The later stages follow each item's "
    "release points, which every todo item carries as wavefront 88793d7 records them, their waves renumbered to this "
    "schedule's and their modules written {link_unit, module, foundational}: port/layout after a struct's, union's or "
    "closed enum's opaque_after (a flags enum has none; compose/enums8.py tells the two apart), port/memory after a "
    "struct's or union's memory_unmanaged_after, port/unexport after a ported function's, global's or function "
    "pointer's unreferenced_after, each when the point names a module other than the item's own, and none when a user "
    "stays C in a foundational module. The C tests count as referrers of every function, global and function pointer "
    "too, though wavefront's closure leaves them out: a clar test case leaves with the first of the modules "
    "modules.json ties it to to be integrated, and a test file's helpers, initialize and cleanup with its last test "
    "case, a helper file's with the last test that reaches it, so a stage a test still reaches also waits for one of "
    "those modules; deps/clar, the harness every remaining test links, never leaves, and keeps the exports it reaches "
    "for good. The stages the same integrations free share a wave of the item's module, after its translation waves, "
    "which depends on the last translation wave of each freeing module, the run that integrates its C.",
    "An item a started wave's batches hold, or that a batch landed, with that objective is not planned again, and a "
    "dependency on it is one on that wave. A translation wave's depends_on is computed from its items' unit edges: "
    "scheduled, its module's previous wave and the earlier waves holding their dependencies; cut, the later ones. A "
    "module's cut records list every unit edge they cut, each to's kind its C kind. Out of scope: the deprecated "
    "git_url_resolve_cb and the GIT_DEPRECATE_HARD-only field users (commit_id, signing_cb, resolve_url, "
    "update_tips). A module's external records count its units' references to out-of-tree entities, macros among "
    "them, compiler builtins left out, by the library whose installed headers define them. An enum's field_users are "
    "empty: wavefront 88793d7 records no enum's constants. Homes follow the conventions: src/<link-unit>/<TU path>.rs, "
    "a header module's <stem>_h.rs. Planner run 8's composition is crustify/artifacts/planner/run-8/compose/."]
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
assert sum(len(m['waves']) for m in mods) == len(waves), (sum(len(m['waves']) for m in mods), len(waves))

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
open(T + 'schedule.v8.json', 'w').write(enc(doc) + '\n')
assert json.load(open(T + 'schedule.v8.json')) == doc
json.dump({'todo_before': [oldtodo_idx[0], oldtodo_idx[-1], len(oldtodo_idx)],
           'todo_after': [first, first + len(out) - 1, len(out)], 'skipped': skipped, 'modules_without_waves': nowaves,
           'never': {o: [(k, kp) for k, kp in v] for o, v in never.items()}}, open(T + 'wave-map.json', 'w'), indent=1)
print('waves', len(waves), 'todo', len(out), 'numbered', first, '-', first + len(out) - 1, 'were', oldtodo_idx[0], '-', oldtodo_idx[-1])
print('todo items', sorted(collections.Counter((i['kind'], i['objective']) for w in out for i in w['items']).items()))
print('stage waves', len(stagew), 'skipped', dict(skipped), 'modules without waves', nowaves)
