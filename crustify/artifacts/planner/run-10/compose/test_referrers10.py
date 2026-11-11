"""Run 9's test_referrers8.py over schedule.v10.json, under run 10's rule for
the tests (stages10.py): a test tied to the item's own module through its public
API alone checks the export and holds nothing back, so it is left out.

Every ported function or global whose export a C test still reaches is
retired only once the test has left: for each port/unexport item, every test
referrer leaves (stages7.leave) at the integration of its own module or of a
module the stage's wave waits for, or one ordered before them; and a function
with no port/unexport planned is reached by no test that outlives its module,
unless it is kept for good. Prints the violations; there must be none."""
import json, collections, sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import stages10 as ST
R = '/target/libgit2-bindings/'
D = json.load(open(R + 'crustify/tmp/planner-run-10/schedule.v10.json'))
W = {w['index']: w for w in D['waves']}
mk = lambda r: ST.keyof(r['link_unit'], r['module'], r['foundational'])
st = ST.stages()
def ok(name, df, own, before):
    bad = []
    for tf, fn in ST.TESTREFS.get((name, df), ()):
        if tf.startswith('tests/') and ST.public_only(tf, fn, own):
            continue
        alts = ST.leave(tf, fn)
        if alts is None or not any(s <= ({own} | before) for s in alts):
            bad.append(f'{tf}:{fn}')
    return bad
viol = []; planned = set()
for w in D['waves']:
    for i in w['items']:
        if i['objective'] != 'port/unexport': continue
        own = mk(w); planned.add((i['name'], i['defined_in']))
        fr = {mk(W[d]) for d in w['depends_on']['scheduled']} - {own}
        before = fr | {a for f in fr for a in ST.ancestors(f)} | ST.ancestors(own)
        b = ok(i['name'], i['defined_in'], own, before)
        if b: viol.append(('stage', i['name'], b[:2]))
for k, d in st.items():
    v = d.get('port/unexport')
    if not v or v[0] != 'none': continue
    name, df, _ = k.split('|')
    b = ok(name, df, ST.UNITS[k]["module"], ST.ancestors(ST.UNITS[k]["module"]))
    if b: viol.append(('none', name, b[:2]))
print('port/unexport items checked:', len(planned), 'violations:', len(viol))
for x in viol[:10]: print(' ', x)
