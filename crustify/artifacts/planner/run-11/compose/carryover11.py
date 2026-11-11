"""The C test cases each port/unexport wave carries over (scheduling, Later
stages 5.2; testing, C tests carried over), recorded here because the schedule
schema has no field for them (crustify/evolution/carried-over-test-cases/).

The plan is one total order on disk (schedule schema, Indices): a wave runs at
its index, and a module is integrated after its last translation wave. In that
order, an in-scope test case leaves at the first of:

- the integration of a port module whose internals it reaches (ties-why.json's
  `internals`), which removes it from c/; and
- the first port/unexport wave that removes an export its code references
  (stages11.py's CASE_REFS: its body, its file's static helpers, fixtures and
  file-scope uses, and the helpers of other test files and of deps/clar it
  calls), which carries it over.

A wave whose dependencies let it run before a lower-indexed one carries over
what is still in c/ when it runs: the record is the plan's prediction, which
the batch checks against c/. Each wave also lists the harness functions
(deps/clar/main.c's main and what it reaches) that reference an export it
removes: the harness is no test case, so the wave adapts it.

Reads crustify/tmp/planner-run-11/schedule.v11.json; writes
compose/carryover.json, {wave index: {module, test_cases: {file: [case]},
harness: [file:fn], if_still_in_c: {file: [case]}}}, and prints the fate of
every in-scope test case. `if_still_in_c` lists, for the exports planned for
the C tests alone (stages11.py), the cases that still reference one at its
module's integration as the dependencies have it, but that the plan's order
removes before the wave, by an integration or another wave's carry-over it does
not depend on: the wave carries them over if they are still in c/."""
import json, collections, os, sys
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import stages11 as ST
R = ST.R
D = json.load(open(sys.argv[1] if len(sys.argv) > 1 else ST.T + 'schedule.v11.json'))
FND = ST.FND
mk = lambda r: ST.keyof(r['link_unit'], r['module'], r['foundational'])
uk = lambda i: '|'.join((i['name'], i['defined_in'], i['kind']))
last = {}                                   # module -> index of its last translation wave
for w in D['waves']:
    its = w['items'] if w['status'] == 'todo' else [dict(i, objective=b['kind']) for b in w['batches'] for i in b['items']]
    if any(i['objective'] in ('wrap', 'port') for i in its) or w['status'] != 'todo':
        last[mk(w)] = max(last.get(mk(w), -1), w['index'])
W = {w['index']: w for w in D['waves']}
UNEXPORT_WAVES = [w for w in sorted(D['waves'], key=lambda w: w['index'])
                  if any(i['objective'] == 'port/unexport' for i in w['items'])]
leave_at = {}                               # case -> ('integration', module, index)
for c in ST.CASES:
    pts = sorted((last[m], m) for m in ST.internals(c) if m in last)
    if pts: leave_at[c] = ('integration', pts[0][1], pts[0][0])
ST_ = ST.stages()
TESTONLY = {k for k, d in ST_.items() if d.get('port/unexport', ('',))[0] == 'plan' and not d['port/unexport'][1]}
fate = {}
out = collections.OrderedDict()
for w in UNEXPORT_WAVES:
    i = w['index']
    keys = {uk(it) for it in w['items'] if it['objective'] == 'port/unexport'}
    cases = set()
    for k in keys:
        for c in ST.REFERRING.get(k, ()):
            if c in fate: continue
            if c in leave_at and leave_at[c][2] < i: continue
            cases.add(c)
    for c in cases: fate[c] = ('carried over', i)
    by = collections.defaultdict(list)
    for c in sorted(cases):
        f, n = c.split(':'); by[f].append(n)
    harness = sorted({h for k in keys for h in ST.HARNESS_REFS.get(k, ())})
    # an export planned for the C tests alone: the cases still referencing it at its
    # module's integration, as the dependencies have it, that the plan's order
    # removes before this wave, by an integration or a carry-over this wave does not
    # depend on: the wave carries them over if they are still in c/ when it runs
    maybe = set()
    for it in w['items']:
        k = uk(it)
        if it['objective'] == 'port/unexport' and k in TESTONLY:
            maybe |= set(ST.tests_left(k)[0]) - cases
    mby = collections.defaultdict(list)
    for c in sorted(maybe):
        f, n = c.split(':'); mby[f].append(n)
    out[str(i)] = {'module': mk(w), 'test_cases': dict(sorted(by.items())), 'harness': harness,
                   'if_still_in_c': dict(sorted(mby.items()))}
for c in ST.CASES:
    if c in fate: continue
    if c in leave_at: fate[c] = leave_at[c][:2] + (leave_at[c][2],)
    elif not ST.CASE_REFS[c]: fate[c] = ('stays', 'references no export of a port module')
    else: fate[c] = ('stays', 'references only exports kept for good by the foundational modules\' C')
json.dump(out, open(os.path.join(HERE, 'carryover.json'), 'w'), indent=1)
json.dump({c: list(v) for c, v in sorted(fate.items())}, open(ST.T + 'fates.json', 'w'), indent=0)
kinds = collections.Counter(v[0] for v in fate.values())
print('in-scope test cases:', len(ST.CASES), dict(kinds))
print('port/unexport waves:', len(UNEXPORT_WAVES), 'carrying over test cases:', sum(1 for v in out.values() if v['test_cases']),
      'adapting the harness:', sum(1 for v in out.values() if v['harness']))
stays = collections.Counter(v[1] for v in fate.values() if v[0] == 'stays')
print('stays:', dict(stays))
# a staying case that references an export: every such export must be one kept for good by C
st = ST_
bad = [c for c, v in fate.items() if v[0] == 'stays' and ST.CASE_REFS[c]
       and any(st[k]['port/unexport'][0] != 'never' for k in ST.CASE_REFS[c])]
print('staying cases that reference an export a port/unexport removes:', len(bad), bad[:5])
big = sorted(((len([n for v in o['test_cases'].values() for n in v]), int(i), o['module']) for i, o in out.items()), reverse=True)[:8]
print('largest carry-overs:', big)
hw = [(int(i), o['module'], o['harness']) for i, o in out.items() if o['harness']]
print('harness adapted by:', hw)
nothing = [i for i, o in out.items() if not o['test_cases'] and not o['harness']
           and any(it['objective'] == 'port/unexport' and uk(it) in TESTONLY for it in W[int(i)]['items'])]
print('port/unexport items planned for the C tests alone:', len(TESTONLY),
      'waves of them whose cases the plan\'s order removes first (if_still_in_c only):', len(nothing))
