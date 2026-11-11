"""modules.json's derived fields under wavefront 88793d7, over decomp8.py's
decomposition: each module's `tests`, `depends_on` and the `files` and
`function_pointers` of its `counters`.

- tests: ties8.py's (crustify/tmp/planner-run-10/ties.json): run 7's rule, over
  run 8's areas.py and test facts (testfacts8.py).
- depends_on, from the whole-closure schedule (crustify/tmp/planner-run-10/
  schedule-deps.json, no seeds): an in-tree record's outgoing_edges count the
  unit edges from this module's units to the other's, its incoming_edges those
  back, and its counters the distinct entities consumed there, by kind; an
  out-of-tree record counts the distinct entities its units reference from the
  library whose installed headers define them (run 5's deps.lib_of), compiler
  builtins left out. Only the kinds the modules schema counts take part, at both
  ends of an edge: structs, unions, functions, globals and function pointers.
  Enums, macros and typedefs of their own are counted nowhere yet (the schema's
  TODO), so a module that uses only another's enums, macros or typedefs has no
  record of it. Runs 4 to 7 counted a type generator's edges, in no kind; it is
  a macro, and counts in neither now.
- counters.*.function_pointers, which were `callbacks`: the function pointers
  the module owns, declared by a typedef or written inline, in its
  translation_unit and internal_headers (implementation) or its api_headers
  (api). counters.*.files: its file entries. Every other counter is unchanged:
  the new header modules own no struct, union, function or global, and the
  headers decomp8.py adds own none either, so their loc, which counts those
  entities' spans, is 0.

Output: crustify/artifacts/modules.json."""
import json, collections, sys, os
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '../../run-5/compose'))
import deps as D5
R = '/target/libgit2-bindings/'
T = R + 'crustify/tmp/planner-run-10/'
P = R + 'crustify/artifacts/modules.json'
M = json.load(open(P))
S = json.load(open(T + 'schedule-deps.json'))
TIES = json.load(open(T + 'ties.json'))
FND = ' (foundational)'
KINDS = {'struct': 'structs', 'union': 'unions', 'function': 'functions', 'global': 'global_variables',
         'function_pointer': 'function_pointers'}
ORDERK = ['structs', 'functions', 'global_variables', 'unions', 'function_pointers']
units = {}
for w in S['waves']:
    for b in w['batches']:
        for it in b['items']:
            units[(it['name'], it['defined_in'], it['kind'])] = it
out_e = collections.Counter()                         # (a, b) -> edges
cons = collections.defaultdict(lambda: collections.defaultdict(set))
ext = collections.defaultdict(lambda: collections.defaultdict(set))
for k, it in units.items():
    a = it['module']
    if a is None or it['kind'] not in KINDS: continue
    for part in ('scheduled', 'cut'):
        for r in it['depends_on'][part]:
            if r['scope'] == 'scheduled':
                t = units.get((r['name'], r['defined_in'], r['kind']))
                if not t or t['module'] in (None, a) or t['kind'] not in KINDS: continue
                out_e[(a, t['module'])] += 1
                cons[a][t['module']].add((KINDS[t['kind']], t['name'], t['defined_in']))
            elif r['scope'] == 'external':
                lib = D5.lib_of(r['defined_in'])
                if lib and r['kind'] in KINDS:
                    ext[a][lib].add((KINDS[r['kind']], r['name']))
def split(o):
    f = o.endswith(FND); o2 = o[:-len(FND)] if f else o
    lu, name = o2.split('/', 1); return lu, name, f
def records(m):
    recs = []
    for o in sorted({b for (a, b) in out_e if a == m}):
        lu, name, f = split(o)
        recs.append({'link_unit': lu, 'module': name, 'foundational': f, 'outgoing_edges': out_e[(m, o)],
                     'incoming_edges': out_e.get((o, m), 0),
                     'counters': {k: len({x for x in cons[m][o] if x[0] == k}) for k in ORDERK}})
    for lib in sorted(ext[m]):
        recs.append({'library': lib, 'counters': {k: len({x for x in ext[m][lib] if x[0] == k}) for k in ORDERK}})
    return recs
fps = collections.defaultdict(collections.Counter)    # module -> {file: function pointers}
for k, it in units.items():
    if it['kind'] == 'function_pointer' and it['module']:
        fps[it['module']][it['defined_in']] += 1
changed = collections.Counter()
for lu in M['link_units']:
    for x in lu['modules']:
        key = f"{lu['name']}/{x['name']}" + (FND if x['foundational'] else '')
        tests = sorted(TIES.get(key, []), key=lambda e: e['path'])
        if tests != x['tests']: changed['tests'] += 1
        x['tests'] = tests
        recs = records(key)
        if recs != x['depends_on']: changed['depends_on'] += 1
        x['depends_on'] = recs
        impl = ([x['translation_unit']['path']] if x['translation_unit'] else []) + [f['path'] for f in x['internal_headers']]
        api = [f['path'] for f in x['api_headers']]
        c = x['counters']
        new_i = dict(c['implementation'], files=len(impl), function_pointers=sum(fps[key][p] for p in impl))
        new_a = dict(c['api'], files=len(api), function_pointers=sum(fps[key][p] for p in api))
        for blk, nb in (('implementation', new_i), ('api', new_a)):
            for f in ('files', 'function_pointers'):
                if c[blk][f] != nb[f]: changed[f'{blk}.{f}'] += 1
        x['counters'] = {'implementation': new_i, 'api': new_a}
owned = sum(fps[k][p] for k in fps for p in fps[k])
assert owned == sum(1 for it in units.values() if it['kind'] == 'function_pointer' and it['module']), owned
# ---- the comment, for run 8's placements, tests and counts
REPL = [
    ("and a header module for each header implementing what no TU does (src/util/integer.h, array.h, bitvec.h, "
     "hashmap_str.h, staticstr.h, src/libgit2/stream.h, src/libgit2/common.h, deps/ntlmclient/utf8.h, src/cli/error.h).",
     "and a header module for each header implementing what no TU does (src/util/integer.h, array.h, bitvec.h, "
     "hashmap.h, hashmap_str.h, staticstr.h, cc-compat.h, ctype_compat.h, strnlen.h, src/libgit2/stream.h, "
     "src/libgit2/common.h, deps/ntlmclient/utf8.h, compat.h, deps/xdiff/xmacros.h, src/cli/error.h). Every entity is "
     "placed under its C kind, as wavefront 88793d7 schedules it: a shared header's API macro goes to the module whose "
     "logic it implements, as the functions it calls and the types it accesses show, a constant or a family of flags "
     "to the module owning the type that holds its values or, holding none, whose functions consume it; a function "
     "pointer written inline, named by its signature, goes with the struct or function whose declaration holds it; a "
     "typedef of its own to the module whose functions use it most; a header holding only macros and typedefs is "
     "bundled with the TU it is named for or implements (assert_safe.h and the error-checking macros of git2_util.h "
     "with src/util/errors.c, version.h with src/libgit2/libgit2.c, reftable-system.h with deps/reftable/system.c, "
     "auth_ntlm.h with auth_ntlmclient.c, src/util/hash/sha.h with src/util/hash.c, unix/pthread.h with "
     "src/util/thread.c). Attribute, linkage and configuration macros are not API, and no module owns them "
     "(crustify/artifacts/planner/run-8/compose/not_api.json)."),
    ("the subject area its suite is named for (crustify/artifacts/planner/run-7/compose/areas.py) that its code reaches.",
     "the subject area its suite is named for (crustify/artifacts/planner/run-8/compose/areas.py) that its code "
     "reaches, the macros it expands included."),
    ("counted from a wavefront schedule of the whole closure (crustify/wavefront/configs/wavefront-config.json without "
     "its schedule_seeds, the body-deep graph, enums left out): an in-tree record's outgoing_edges are the unit edges "
     "from this module to the other, its incoming_edges those back, non-zero on either side of a cycle, and its "
     "counters the distinct entities it consumes there, by kind (a type generator macro is counted in the edges but in "
     "no kind); an out-of-tree record counts the distinct entities its units reference from the library whose installed "
     "headers define them, as dependencies.json names it, macros and compiler builtins left out. A module that uses "
     "only another's enums has no record of it.",
     "counted from a wavefront 88793d7 schedule of the whole closure (crustify/wavefront/configs/wavefront-config.json "
     "without its schedule_seeds, the body-deep graph) over the kinds the modules schema counts, structs, unions, "
     "functions, globals and function pointers, at both ends of an edge: an in-tree record's outgoing_edges are the "
     "unit edges from this module to the other, its incoming_edges those back, non-zero on either side of a cycle, and "
     "its counters the distinct entities it consumes there, by kind; an out-of-tree record counts the distinct entities "
     "its units reference from the library whose installed headers define them, as dependencies.json names it, macros "
     "and compiler builtins left out. Enums, macros and typedefs of their own are counted nowhere yet, so a module that "
     "uses only another's enums, macros or typedefs has no record of it. counters.*.function_pointers counts the "
     "function pointers a module owns, declared by a typedef or written inline."),
]
for a, b in REPL:
    assert a in M['_comment'], a[:60]
    M['_comment'] = M['_comment'].replace(a, b)
open(P, 'w').write(json.dumps(M, indent=2, ensure_ascii=False) + '\n')
print('modules changed:', dict(changed), 'function pointers owned:', owned)
