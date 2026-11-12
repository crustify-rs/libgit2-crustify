"""dependencies.json under run 12's schedule: each entity's `used_by` from the
external records of schedule.v12.json's modules, as deps11.py derives them
(the modules whose units reference the entity, in the order of the modules), and
the comment brought up to date. Nothing else changes: the candidates, their
quality_assessment and the entities' matches and recommendations are run 11's,
and each library's decision and crustify_campaign are the user's
(fb010a58e).

Wavefront e529c26 gives macros the external references their replacement lists
make, so modules whose macros reference an entity, and no unit of which did
before, now use it. No library gains or loses an entity.

Writes crustify/artifacts/dependencies.json (indent=1, ensure_ascii=False)."""
import json, collections, os, sys
HERE = os.path.dirname(os.path.abspath(__file__))
R = '/target/libgit2-crustify/crustify/.worktrees/planner/run-12/'
T = R + 'crustify/tmp/planner-run-12/'
P = R + 'crustify/artifacts/dependencies.json'
D = json.load(open(P))
SCHED = json.load(open(T + 'schedule.v12.json'))
# deps11.py's header(): the public header that declares an internal glibc header's entity
src = open(os.path.join(HERE, 'deps11.py')).read()
ns = {}
exec(src[src.index('PUBLIC = ['):src.index('FND = ')], ns)
header = ns['header']
USED = collections.defaultdict(lambda: collections.defaultdict(list))
for m in SCHED['modules']:
    for e in m['depends_on']['external']:
        for it in e['items']:
            k = (it['to']['name'], it['kind'], header(it['to']['defined_in']))
            rec = {'link_unit': m['link_unit'], 'module': m['module'], 'foundational': m['foundational']}
            if rec not in USED[e['library']][k]: USED[e['library']][k].append(rec)
stats = collections.Counter()
for lib in D['libraries']:
    used = USED[lib['name']]
    have = {(e['c_name'], e['kind'], e['header']) for e in lib['c_entities']}
    assert have == set(used), (lib['name'], sorted(have ^ set(used))[:10])
    for e in lib['c_entities']:
        new = used[(e['c_name'], e['kind'], e['header'])]
        key = lambda l: {json.dumps(x, sort_keys=True) for x in l}
        if key(new) != key(e['used_by']):
            stats['used_by gained modules'] += 1
            stats['module uses added'] += len(key(new) - key(e['used_by']))
            assert not key(e['used_by']) - key(new), (lib['name'], e['c_name'])
        elif new != e['used_by']:
            stats['used_by reordered'] += 1
        e['used_by'] = new
D['_comment'] = (
    "The libgit2 port campaign's out-of-tree libraries, in the schema of Crustify 043a8ae, as planner run 11 brought them "
    "to it from run 10's assessments: each library's candidate crates, each with one quality_assessment composed from run "
    "10's reading of it against the Adopted crates criteria (coverage, shared state, soundness, advisories, maintenance) "
    "and its verdict; every entity the campaign uses of the library (c_entities), from the external records of "
    "schedule.json's modules, named by the public header that declares it, with the modules that use it, what each "
    "candidate and std offers for it and the plan's recommended route, by the area run 10 assessed it in; and the "
    "recommendation that follows. Each library's decision is the user's, as recorded after run 11 (fb010a58e): the crates "
    "it adopts, the route of what they leave uncovered, and the overrides of the entities whose recommended route it does "
    "not take. libz-rs-sys, assessed by run 10, is no candidate: it has no safe API. Planner run 12 took each entity's "
    "used_by from its schedule, whose macros reference out-of-tree entities too (wavefront e529c26). Planner run 11's "
    "composition is crustify/artifacts/planner/run-11/compose/ (deps11.py), run 12's "
    "crustify/artifacts/planner/run-12/compose/ (deps12.py).")
open(P, 'w').write(json.dumps(D, indent=1, ensure_ascii=False) + '\n')
print(dict(stats))
