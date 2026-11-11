"""Planner run 7's testfacts.py over wavefront 88793d7's schedule, whose units
carry their C kinds, and with the macros a test's code expands among what it
reaches (the Bundling rule counts a test's macro expansions as its own uses):
t2/macro_expansions.csv's rows in test files add to `refs`.

The C tests' facts, from the campaign's CodeQL tables (T1/T2 trace the tests
as well as the closure): for every function of a test file, the closure units
it calls, takes the address of or accesses, its calls to other functions of its
own file, and its uses of other modules' internals as the Bundling rule names
them (fields and layout of the types a module owns, its static functions and
static globals). Writes crustify/tmp/planner-run-11/testfacts.json."""
import csv, json, collections, re, sys
R = '/target/libgit2-bindings/'
T1 = R + 'crustify/wavefront/codeql/t1/'
T2 = R + 'crustify/wavefront/codeql/t2/'
OUT = R + 'crustify/tmp/planner-run-11/'
SCHED = sys.argv[1] if len(sys.argv) > 1 else OUT + 'schedule-deps.json'
def rd(p): return csv.DictReader(open(p, newline=''))
def is_test(f): return f.startswith('tests/')
S = json.load(open(SCHED))
FND = ' (foundational)'
UNIT = {}                                  # (name, def_file, 'type'|'symbol') -> module key
TYPE_KINDS = ('struct', 'union', 'enum')
def static(it):
    """run 7's v5 `function_static`: a static function of a .c file. A header's
    `static inline` function is its header's API, no internal."""
    if it['kind'] == 'function' and it['stats'].get('static') and not it['defined_in'].endswith('.h'):
        return 'function_static'
    return it['kind']
for w in S['waves']:
    for b in w['batches']:
        for it in b['items']:
            k = 'type' if it['kind'] in TYPE_KINDS else 'symbol'
            UNIT[(it['name'], it['defined_in'], k)] = (it['module'], static(it))
TYPES_BY_NAME = collections.defaultdict(list)
for (n, f, k), v in UNIT.items():
    if k == 'type': TYPES_BY_NAME[n].append((n, f, k))
def type_unit(n, f):
    """The struct or union unit of a field or layout use. An enum's constants and
    values are no internals: a test that only uses a module's enums is not tied to
    it, so enum units, which wavefront 88793d7 schedules, are left out here."""
    if (n, f, 'type') in UNIT: k = (n, f, 'type')
    else:
        ks = TYPES_BY_NAME.get(n, [])
        k = ks[0] if len(ks) == 1 else None
    return k if k and UNIT[k][1] in ('struct', 'union') else None
FUN = {}                                    # (def_file, name) -> linkage, for test files
for r in rd(T1 + 'functions.csv'):
    if is_test(r['def_file']): FUN[(r['def_file'], r['name'])] = r['linkage']
fn = lambda: {'calls': collections.Counter(), 'refs': collections.Counter(), 'local': set(),
              'testcalls': set(), 'internals': collections.defaultdict(set)}
F = collections.defaultdict(fn)             # (test file, fn) -> facts
def add_ref(f, c, name, df, kind, bucket):
    u = (name, df, kind)
    if u in UNIT:
        F[(f, c)][bucket][f'{UNIT[u][0]}|{name}|{df}'] += 1
        return UNIT[u]
for r in rd(T2 + 'function_calls.csv'):
    f, c = r['caller_file'], r['caller_name']
    if not is_test(f) or not c: continue
    if r['callee_def_file'] == f: F[(f, c)]['local'].add(r['callee_name']); continue
    if is_test(r['callee_def_file']):
        F[(f, c)]['testcalls'].add(r['callee_def_file'] + '|' + r['callee_name']); continue
    m = add_ref(f, c, r['callee_name'], r['callee_def_file'], 'symbol', 'calls')
    if m and m[1] == 'function_static':
        F[(f, c)]['internals'][m[0]].add('static_fn:' + r['callee_name'])
for r in rd(T2 + 'function_addresses.csv'):
    f, c = r['access_file'], r['enclosing_name']
    if not is_test(f) or not c: continue
    if r['target_def_file'] == f: F[(f, c)]['local'].add(r['target_name']); continue
    if is_test(r['target_def_file']):
        F[(f, c)]['testcalls'].add(r['target_def_file'] + '|' + r['target_name']); continue
    m = add_ref(f, c, r['target_name'], r['target_def_file'], 'symbol', 'refs')
    if m and m[1] == 'function_static':
        F[(f, c)]['internals'][m[0]].add('static_fn:' + r['target_name'])
for r in rd(T2 + 'global_accesses.csv'):
    f, c = r['access_file'], r['enclosing_name']
    if not is_test(f) or not c: continue
    m = add_ref(f, c, r['global_name'], r['global_def_file'], 'symbol', 'refs')
    if m and r['global_linkage'] == 'global_static':
        F[(f, c)]['internals'][m[0]].add('static_gl:' + r['global_name'])
for r in rd(T2 + 'macro_expansions.csv'):
    f, c = r['invocation_file'], r['enclosing_name']
    if not is_test(f) or not r['macro_name'] or not c: continue     # a function's own code only
    add_ref(f, c, r['macro_name'], r['macro_def_file'], 'symbol', 'refs')
for r in rd(T2 + 'field_accesses.csv'):
    f, c = r['access_file'], r['enclosing_name'] or '<file>'
    if not is_test(f): continue
    u = type_unit(r['struct_name'], r['struct_def_file'])
    if u: F[(f, c)]['internals'][UNIT[u][0]].add('field:' + r['struct_name'] + '.' + r['field_name'])
for r in rd(T2 + 'layout_uses.csv'):
    f, c = r['use_file'], r['user_name'] or '<file>'
    if not is_test(f) or not is_test(r['user_def_file'] or f): continue
    u = type_unit(r['type_name'], r['type_def_file'])
    if u: F[(f, c)]['internals'][UNIT[u][0]].add(f"layout:{r['type_name']}:{r['use_kind']}")
out = {}
for (f, c), d in F.items():
    out.setdefault(f, {})[c] = {'linkage': FUN.get((f, c)), 'calls': dict(d['calls']), 'refs': dict(d['refs']),
                                'local': sorted(d['local']), 'testcalls': sorted(d['testcalls']),
                                'internals': {m: sorted(v) for m, v in d['internals'].items()}}
for (f, c), l in FUN.items():                # functions with no facts at all
    out.setdefault(f, {}).setdefault(c, {'linkage': l, 'calls': {}, 'refs': {}, 'local': [], 'testcalls': [], 'internals': {}})
json.dump(out, open(OUT + 'testfacts.json', 'w'), indent=0, sort_keys=True)
print(len(out), 'test files', sum(len(v) for v in out.values()), 'functions')
