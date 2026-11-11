"""The function pointers written inline that no module claimed, each with the
module of the struct or function whose declaration holds it (the decomposition's
Bundling rule), from t2/function_pointer_uses.csv and the whole-closure schedule
of the committed configuration in wavefront 88793d7's kinds
(crustify/tmp/planner-run-11/schedule-deps.json, before decomp8.py's edits).
Output: compose/fp_owners.json, {module name: {header: [signatures]}}."""
import json, csv, collections, os
R = '/target/libgit2-bindings/crustify/'
HERE = os.path.dirname(os.path.abspath(__file__))
S = json.load(open(R + 'tmp/planner-run-11/schedule-deps.json'))
units = [it for w in S['waves'] for b in w['batches'] for it in b['items']]
owner = collections.defaultdict(set)
for it in units:
    if it['module']:
        owner[(it['name'], it['defined_in'])].add(it['module'])
rows = list(csv.DictReader(open(R + 'wavefront/codeql/t2/function_pointer_uses.csv')))
out = collections.defaultdict(lambda: collections.defaultdict(list))
for it in units:
    if it['module'] is not None or it['kind'] != 'function_pointer' or it['typedefs'] or not it['name'].count('(*)'):
        continue
    hold = {m for r in rows if r['signature'] == it['name'] and r['def_file'] == it['defined_in']
            for m in owner.get((r['user_name'], r['user_def_file']), ())}
    assert len(hold) == 1, (it['name'], it['defined_in'], hold)
    m = hold.pop().split('/', 1)[1]
    out[m][it['defined_in']].append(it['name'])
json.dump({m: {p: sorted(v) for p, v in d.items()} for m, d in sorted(out.items())},
          open(os.path.join(HERE, 'fp_owners.json'), 'w'), indent=1)
print({m: {p: len(v) for p, v in d.items()} for m, d in out.items()})
