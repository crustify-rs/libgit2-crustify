"""The C test cases out of scope: those whose body the configured build compiles
empty (porting-workflow, Tests follow their modules; scheduling, Later stages
5.2), as a configured-out translation unit is.

Every clar test case (`test_<suite>__<name>`, not `__initialize`/`__cleanup`)
of the test files the traced build compiles is preprocessed with the `plain`
build's own compile command (ninja's, for libgit2_tests and util_tests), and
its body taken from the preprocessed text by brace matching. A body that holds
nothing but whitespace and empty statements is empty: a `GIT_WIN32`-only body,
one whose `#if` the configured build drops (`LIBGIT2_VERSION_CHECK`, a builtin
`git__multiply_int64_overflow`), a deprecated API's under `GIT_DEPRECATE_HARD`,
or a comment alone.

Cross-check: every empty case has no row in the T2 tables (no call, macro
expansion, address, global access, local type use or field access), and the
script reports the cases with no row whose body is not empty.

Writes crustify/tmp/planner-run-11/empty.json: sorted ["file:test case", ...]."""
import csv, json, re, shlex, subprocess, collections, sys
R = '/target/libgit2-bindings/'
B = R + 'crustify/builds/plain'
T1 = R + 'crustify/wavefront/codeql/t1/'
T2 = R + 'crustify/wavefront/codeql/t2/'
OUT = R + 'crustify/tmp/planner-run-11/'
def rd(p): return csv.DictReader(open(p, newline=''))
def is_tc(n): return bool(re.match(r'test_.+__.+', n)) and not n.endswith(('__initialize', '__cleanup'))
TCS = collections.defaultdict(set)
for r in rd(T1 + 'functions.csv'):
    if r['def_file'].startswith('tests/') and is_tc(r['name']):
        TCS[r['def_file']].add(r['name'])
CMD = {}
for target in ('libgit2_tests', 'util_tests'):
    out = subprocess.run(['ninja', '-C', B, '-t', 'commands', target], capture_output=True, text=True, check=True).stdout
    for line in out.splitlines():
        m = re.search(r' -c (\S+/c/(tests/\S+\.c))$', line)
        if m: CMD[m.group(2)] = line
def preprocess(f):
    a = shlex.split(CMD[f])
    keep, skip = [], False
    for x in a:
        if skip: skip = False; continue
        if x in ('-o', '-MF', '-MT'): skip = True; continue
        if x in ('-c', '-MD'): continue
        keep.append(x)
    return subprocess.run(keep[:-1] + ['-E', '-P', keep[-1]], capture_output=True, text=True, cwd=B, check=True).stdout
def body(text, name):
    m = re.search(r'\bvoid\s+' + re.escape(name) + r'\s*\(\s*void\s*\)\s*\{', text)
    if not m: return None
    i, depth = m.end(), 1
    while depth:
        c = text[i]
        if c in '"\'':                    # skip a string or character literal
            j = i + 1
            while text[j] != c:
                j += 2 if text[j] == '\\' else 1
            i = j
        elif c == '{': depth += 1
        elif c == '}': depth -= 1
        i += 1
    return text[m.end():i - 1]
empty, missing = [], []
for f in sorted(TCS):
    if f not in CMD: missing.append(f); continue
    text = preprocess(f)
    for n in sorted(TCS[f]):
        b = body(text, n)
        if b is None: missing.append(f'{f}:{n}'); continue
        if not re.sub(r'[\s;]', '', b): empty.append(f'{f}:{n}')
# cross-check against the T2 tables
has = collections.Counter()
for p, fc, nc in (('function_calls.csv', 'caller_file', 'caller_name'), ('macro_expansions.csv', 'invocation_file', 'enclosing_name'),
                  ('function_addresses.csv', 'access_file', 'enclosing_name'), ('global_accesses.csv', 'access_file', 'enclosing_name'),
                  ('local_type_uses.csv', 'function_def_file', 'function_name'), ('field_accesses.csv', 'access_file', 'enclosing_name')):
    for r in rd(T2 + p):
        if r[fc].startswith('tests/'): has[f'{r[fc]}:{r[nc]}'] += 1
nofacts = {f'{f}:{n}' for f in TCS for n in TCS[f] if not has[f'{f}:{n}']}
print('test cases:', sum(len(v) for v in TCS.values()), 'empty:', len(empty), 'not found:', missing)
print('empty with T2 rows:', sorted(set(empty) - nofacts))
print('no T2 rows, not empty:', sorted(nofacts - set(empty)))
json.dump(sorted(empty), open(OUT + 'empty.json', 'w'), indent=0)
