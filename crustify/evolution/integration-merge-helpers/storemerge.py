#!/usr/bin/env python3
"""storemerge.py <ours> <base> <theirs>: keyed three-way merge of wavefront's ownership store: a record
is keyed by (name, defined_in); records only one side added or changed are taken from it; a record
both sides changed differently is an error. Order: ours', then theirs' new ones. Writes <ours>."""
import json, sys
o, b, t = [json.load(open(p)) for p in sys.argv[1:4]]
key = lambda r: (r['name'], r.get('defined_in'))
bad = []
for sec in ('types', 'symbols'):
    B = {key(r): r for r in b[sec]}; T = {key(r): r for r in t[sec]}; O = {key(r): r for r in o[sec]}
    out = []
    for r in o[sec]:
        k = key(r)
        if k in T and T[k] != r:
            if B.get(k) == r: r = T[k]                 # only theirs changed it
            elif B.get(k) != T[k]: bad.append((sec, k))
        elif k not in T and k in B and B[k] == r: continue   # theirs removed it
        out.append(r)
    out += [r for r in t[sec] if key(r) not in O and key(r) not in B]
    o[sec] = out
if bad: print('BOTH CHANGED:', bad); sys.exit(1)
open(sys.argv[1], 'w').write(json.dumps(o, indent=1, ensure_ascii=True) + '\n')
