"""The C test cases each port/unexport item carries over, as its `carries_over`
(schedule schema, waves[*] items; scheduling, Later stages 5.2; testing, C
tests carried over). Run 11's carryover11.py, which wrote them per wave to
compose/carryover.json, with what the assistant did after run 11 made part of
the composition:

- `9237f3a18` split each wave's record per item: an item's `cases` are the
  cases its wave carries over that reference its export; its `if_still_in_c`
  are, for an export planned for the C tests alone (stages11.py), the cases
  still referencing it at its module's switch, as the dependencies have it,
  that the plan's order removes before the wave, less the wave's own cases;
  its `harness` the harness functions that reference its export. Every todo
  port/unexport item carries the field, empty lists where it carries nothing.
- `5d23cc8e9` (crustify/evolution/test-only-exports-and-the-harness/, option
  1, which the scheduling skill's Later stages 5.2 now states): the harness
  adapts to every export removed before it, but it cannot run without
  `git_libgit2_init` and `git_libgit2_shutdown` (SUITE_END): the first
  port/unexport wave that removes one of them ends the C suite. It carries over
  every in-scope case still in c/ when it runs, recorded among the `cases` of
  its `git_libgit2_init` item, and removes the harness: the later waves carry
  over nothing and adapt no harness.

Run 12 corrects one thing (crustify/reviews/planner/modules/
if-still-in-c-for-test-only-exports-alone/): an item's `if_still_in_c` is the
schema's whole set, every case referencing its export that leaves c/ before the
wave, at a switch, another wave's carry-over or the end of the C suite, that the
wave does not depend on through depends_on.scheduled, and not only, as run 11
had it, the cases of an export planned for the C tests alone (which it keeps).
IF_STILL_FULL=0 gives run 11's sets, with which `--check-head` reproduces every
carries_over of crustify/devel 6ae0bd6f9 exactly.

The plan is one total order on disk (schedule schema, Indices): a wave runs at
its index, and a module switches after its last translation wave. In that
order, an in-scope test case leaves at the first of the switch of a port module
whose internals it reaches (ties-why.json's `internals`), the first
port/unexport wave that removes an export its code references (stages11.py's
CASE_REFS), and the end of the C suite.

`apply(D)` sets `carries_over` on every todo port/unexport item of the schedule
document D, in place, and returns the per-wave record and every case's fate.
Run alone, it reads crustify/tmp/planner-run-12/schedule.v12.json (or argv[1])
and prints the counts; `--check-head` instead strips HEAD's carries_over,
recomputes them over HEAD's own waves and reports where they differ."""
import json, collections, os, sys, subprocess
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import stages11 as ST
FND = ST.FND
SUITE_END = {'git_libgit2_init|src/libgit2/libgit2.c|function', 'git_libgit2_shutdown|src/libgit2/libgit2.c|function'}
SUITE_END_HOLDER = 'git_libgit2_init|src/libgit2/libgit2.c|function'
mk = lambda r: ST.keyof(r['link_unit'], r['module'], r['foundational'])
uk = lambda i: '|'.join((i['name'], i['defined_in'], i['kind']))
def shaped(cases):
    by = collections.defaultdict(list)
    for c in sorted(cases):
        f, n = c.split(':'); by[f].append(n)
    return [{'path': f, 'tests': ts} for f, ts in sorted(by.items())]

FULL = os.environ.get('IF_STILL_FULL', '1') != '0'
def ancestors(D):
    W = {w['index']: w for w in D['waves']}
    pos = {i: n for n, i in enumerate(sorted(W))}
    anc = {}
    for i in sorted(W):
        a = 0
        for d in W[i]['depends_on']['scheduled']:
            a |= anc[d] | (1 << pos[d])
        anc[i] = a
    return lambda i, j: bool(anc[i] >> pos[j] & 1)

def apply(D):
    dep = ancestors(D)
    last = {}                               # module -> index of its last translation wave
    for w in D['waves']:
        its = (w['items'] if w['status'] == 'todo'
               else [dict(i, objective=b['kind']) for b in w['batches'] for i in b.get('items', ())])
        if any(i['objective'] in ('wrap', 'port') for i in its) or w['status'] != 'todo':
            last[mk(w)] = max(last.get(mk(w), -1), w['index'])
    waves = [w for w in sorted(D['waves'], key=lambda w: w['index'])
             if w['status'] == 'todo' and any(i['objective'] == 'port/unexport' for i in w['items'])]
    leave_at = {}
    for c in ST.CASES:
        pts = sorted((last[m], m) for m in ST.internals(c) if m in last)
        if pts: leave_at[c] = ('switch', pts[0][1], pts[0][0])
    st = ST.stages()
    testonly = {k for k, d in st.items() if d.get('port/unexport', ('',))[0] == 'plan' and not d['port/unexport'][1]}
    fate, per_wave, ended = {}, collections.OrderedDict(), None
    def left_before(c, i):
        """The wave at whose end, or the switch after whose last wave, the case has left c/ before
        wave i in the plan's order; None when it is still there."""
        pts = [fate[c][1]] if c in fate else []
        if c in leave_at: pts.append(leave_at[c][2])
        pts = [p for p in pts if p < i]
        return min(pts) if pts else None
    for w in waves:
        i = w['index']
        keys = [uk(it) for it in w['items'] if it['objective'] == 'port/unexport']
        if ended is not None:
            for it in w['items']:
                if it['objective'] != 'port/unexport': continue
                k = uk(it); mine = set()
                maybe = set(ST.tests_left(k)[0]) if k in testonly else set()
                maybe = {c for c in maybe if left_before(c, i) is not None}
                if FULL:
                    maybe |= {c for c in ST.REFERRING.get(k, ()) if left_before(c, i) is not None
                              and not dep(i, left_before(c, i))}
                it['carries_over'] = {'cases': [], 'if_still_in_c': shaped(maybe), 'harness': []}
            per_wave[str(i)] = {'module': mk(w), 'cases': 0, 'harness': [], 'after the end of the C suite': ended}
            continue
        still = lambda c: c not in fate and not (c in leave_at and leave_at[c][2] < i)
        cases = {c for k in keys for c in ST.REFERRING.get(k, ()) if still(c)}
        ends = bool(SUITE_END & set(keys))
        everything = {c for c in ST.CASES if still(c)} if ends else set()
        for c in cases | everything:
            fate[c] = ('carried over', i) if c in cases else ('carried over at the end of the C suite', i)
        for it in w['items']:
            if it['objective'] != 'port/unexport': continue
            k = uk(it)
            mine = {c for c in cases if k in ST.CASE_REFS[c]}
            if ends and k == SUITE_END_HOLDER:
                mine |= everything
            maybe = (set(ST.tests_left(k)[0]) - cases - everything) if k in testonly else set()
            if FULL:
                # the schema's whole definition: every case referencing the export that the plan's order
                # removes before the wave, by a switch or a carry-over (the end of the C suite included)
                # the wave does not depend on
                maybe |= {c for c in ST.REFERRING.get(k, ()) if c not in mine and left_before(c, i) is not None
                          and not dep(i, left_before(c, i))}
            it['carries_over'] = {'cases': shaped(mine), 'if_still_in_c': shaped(maybe),
                                  'harness': sorted(ST.HARNESS_REFS.get(k, ()))}
        per_wave[str(i)] = {'module': mk(w), 'cases': len(cases | everything),
                            'harness': sorted({h for k in keys for h in ST.HARNESS_REFS.get(k, ())})}
        if ends:
            ended = i
            per_wave[str(i)]['ends the C suite'] = len(everything - cases)
    for c in ST.CASES:
        if c in fate: continue
        if c in leave_at: fate[c] = leave_at[c][:2] + (leave_at[c][2],)
        elif not ST.CASE_REFS[c]: fate[c] = ('stays', 'references no export of a port module')
        else: fate[c] = ('stays', "references only exports kept for good by the foundational modules' C")
    return per_wave, fate, ended

def report(D, per_wave, fate, ended):
    kinds = collections.Counter(v[0] for v in fate.values())
    print('in-scope test cases:', len(ST.CASES), dict(kinds))
    print('port/unexport waves:', len(per_wave), 'carrying over test cases:', sum(1 for v in per_wave.values() if v['cases']),
          'adapting the harness:', sum(1 for v in per_wave.values() if v['harness']), 'the C suite ends at', ended,
          per_wave.get(str(ended)))
    print('stays:', dict(collections.Counter(v[1] for v in fate.values() if v[0] == 'stays')))
    big = sorted(((v['cases'], int(i), v['module']) for i, v in per_wave.items()), reverse=True)[:8]
    print('largest carry-overs:', big)
    print('harness adapted by:', [(int(i), v['module'], v['harness']) for i, v in per_wave.items() if v['harness']])

if __name__ == '__main__':
    R = ST.R
    if '--check-head' in sys.argv:
        H = json.loads(subprocess.run(['git', '-C', R, 'show', 'HEAD:crustify/artifacts/schedule.json'],
                                      capture_output=True, check=True).stdout)
        want = {}
        for w in H['waves']:
            for it in w['items']:
                if 'carries_over' in it:
                    want[(w['index'], uk(it))] = it.pop('carries_over')
        per_wave, fate, ended = apply(H)
        got = {(w['index'], uk(it)): it['carries_over'] for w in H['waves'] for it in w['items'] if 'carries_over' in it}
        bad = [k for k in set(want) | set(got) if want.get(k) != got.get(k)]
        print('HEAD items with carries_over:', len(want), 'recomputed:', len(got), 'differing:', len(bad))
        for k in sorted(bad)[:12]:
            a, b = want.get(k), got.get(k)
            print(' ', k)
            for f in ('cases', 'if_still_in_c', 'harness'):
                if (a or {}).get(f) != (b or {}).get(f):
                    print('    ', f, 'HEAD', json.dumps((a or {}).get(f))[:300], '\n      now ', json.dumps((b or {}).get(f))[:300])
        report(H, per_wave, fate, ended)
    else:
        D = json.load(open(sys.argv[1] if len(sys.argv) > 1 else ST.T + 'schedule.v12.json'))
        report(D, *apply(D))
