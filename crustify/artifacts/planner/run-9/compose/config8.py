"""Planner run 8's wavefront configuration, authored from modules.json.

- closure: run 7's, with the headers modules.json now lists that it lacked
  (decomp8.py), the installed ones under api_headers, the others under
  impl_files.
- modules: one record per module of modules.json, named <link-unit>/<name>,
  with ' (foundational)' after a foundational module's name; its files are its
  translation_unit, internal_headers and api_headers entries, in that order, a
  path when both lists are empty, else {path, items}, whose items are the
  entities of each listed name that the file is home to (its definition, or
  with none its declaration), each as {name, kind} with what the entity is in C
  (`entities.txt`, from `wavefront ... query --closure-only` over the closure
  with no modules). A name the file is home to no entity of is dropped: a file
  entry may also name what the file only declares. Run 7's items took the route
  (`type`, `symbol`) as their kind, which wavefront 88793d7 refuses.
- schedule_seeds: the files of every module, each module's entries in module
  order, identical entries once. Run 4 left api.c's seed of
  deps/llhttp/llhttp.h naming the enum llhttp_finish its module entry no longer
  lists (crustify/reviews/planner/modules/llhttp-seed-stale-finish/).

Run on the committed modules.json (1aa1ee98e), it reproduces the committed
configuration's modules but for the kinds, which `--check` verifies.

Output: crustify/wavefront/configs/wavefront-config.json."""
import collections, json, os, subprocess, sys
R = '/target/libgit2-bindings/'
T = R + 'crustify/tmp/planner-run-9/'
CFG = R + 'crustify/wavefront/configs/wavefront-config.json'
def git_json(path, rev='1aa1ee98e'):
    return json.loads(subprocess.run(['git', '-C', R, 'show', f'{rev}:{path}'], capture_output=True, check=True).stdout)
CHECK = '--check' in sys.argv
c = git_json('crustify/wavefront/configs/wavefront-config.json')
M = git_json('crustify/artifacts/modules.json') if CHECK else json.load(open(R + 'crustify/artifacts/modules.json'))
FND = ' (foundational)'
# ---- closure
impl, api = c['closure']['impl_files']['included'], c['closure']['api_headers']['included']
for lu in M['link_units']:
    for x in lu['modules']:
        for sec in ('translation_unit', 'internal_headers', 'api_headers'):
            for f in ([x[sec]] if sec == 'translation_unit' else x[sec]):
                if f and f['path'] not in impl and f['path'] not in api:
                    (api if f['path'].startswith('include/') else impl).append(f['path'])
impl.sort(); api.sort()
# ---- entities the closure's files are home to
bare = dict(c); bare.pop('modules'); bare.pop('schedule_seeds')
json.dump(bare, open(T + 'config-bare.json', 'w'), indent=1)
ent_path = T + ('entities-check.txt' if CHECK else 'entities.txt')
with open(ent_path, 'w') as out:
    subprocess.run(['wavefront', R + 'c', '--config', T + 'config-bare.json', 'query', '--closure-only'],
                   stdout=out, stderr=subprocess.DEVNULL, check=True)
home = collections.defaultdict(list)          # (file, name) -> [kind]
for line in open(ent_path):
    n, k, d, decl = line.rstrip('\n').split('\t')
    for f in ([d] if d else decl.split(',')[:1] if k in ('typedef', 'function_pointer', 'struct', 'union', 'enum') else []):
        home[(f, n)].append(k)
    if not d and k not in ('typedef', 'function_pointer', 'struct', 'union', 'enum'):
        for f in decl.split(','):
            home[(f, n)].append(k)
def items(path, names):
    return [{'name': n, 'kind': k} for n in names for k in sorted(set(home.get((path, n), ())))]
def file_entry(f):
    inc, exc = f['items']['included'], f['items']['excluded']
    if not inc and not exc:
        return f['path']
    lst, names = ('included', inc) if inc else ('excluded', exc)
    sel = items(f['path'], names)
    if not sel and lst == 'excluded':
        return f['path']
    return {'path': f['path'], 'items': {lst: sel}}
mods = []
for lu in M['link_units']:
    for x in lu['modules']:
        files = [file_entry(f) for f in ([x['translation_unit']] if x['translation_unit'] else [])
                 + x['internal_headers'] + x['api_headers']]
        files = [f for f in files if isinstance(f, str) or f['items'].get('included', True)]
        mods.append({'name': f"{lu['name']}/{x['name']}" + (FND if x['foundational'] else ''), 'files': files})
if CHECK:
    def norm(fs):
        return [f if isinstance(f, str) else {'path': f['path'], 'items': {l: sorted(i['name'] for i in v)
                                                                            for l, v in f['items'].items()}} for f in fs]
    old = {m['name']: norm(m['files']) for m in c['modules']}
    new = {m['name']: norm(m['files']) for m in mods}
    bad = [n for n in old if old[n] != new.get(n)]
    print('modules', len(old), len(new), 'differing', len(bad))
    for n in bad[:10]:
        print(n, old[n], new.get(n), sep='\n  ')
    raise SystemExit(1 if bad or set(old) != set(new) else 0)
c['modules'] = mods
seeds, seen = [], set()
for m in mods:
    for f in m['files']:
        k = json.dumps(f, sort_keys=True)
        if k not in seen:
            seen.add(k); seeds.append(f)
c['schedule_seeds'] = seeds
c['_comment'][2] = ("modules: one record per module of modules.json, named <link-unit>/<name>, with ' (foundational)' after a "
                    "foundational module's name; its files are the module's file entries with their items, as {name, kind}, "
                    "each item an entity its file is home to and each kind what the entity is in C (struct, union, enum, "
                    "function, global, macro, function_pointer or typedef), a function pointer written inline named by its "
                    "signature. A typedef follows the struct, union, enum or function pointer it names, so a shared header's "
                    "typedef of a struct another file defines is claimed there. A module owns every macro of the files it lists "
                    "whole or with excluded, and the API macros of a shared header it lists with included are named there, "
                    "as modules.json splits them.")
c['_comment'][3] = ("schedule_seeds: the files of every module of the campaign, each module's file entries in module order, "
                    "identical entries once, so that the out-of-scope entities no module owns (deprecated items, and "
                    "declarations the configured build never defines, which modules.json's file entries exclude) are not seeded.")
open(CFG, 'w').write(json.dumps(c, indent=2, ensure_ascii=False) + '\n')
print('modules', len(mods), 'seeds', len(seeds), 'impl_files', len(impl), 'api_headers', len(api))
