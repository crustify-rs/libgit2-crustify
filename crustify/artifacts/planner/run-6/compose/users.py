"""Every unit's whole user lists, as wavefront's schedule finds them for its
release points (src/wavefront/schedule.py): the layout users of each struct or
union closed over embedding, its raw sites' functions, and each function's or
global's referrers. Written to crustify/tmp/planner-run-6/users.json, keyed
"name|defined_in|kind". Run with wavefront's interpreter (/opt/venv/bin/python)."""
import json, sys
from collections import defaultdict
from pathlib import Path
R = Path('/target/libgit2-bindings')
from wavefront.layout import Layout, set_config_path, set_repo_root
from wavefront import opacity
set_repo_root(R / 'c'); set_config_path(R / 'crustify/wavefront/configs/wavefront-config.json')
layout = Layout.discover(R / 'c')
S = json.load(open(R / 'crustify/tmp/planner-run-6/schedule-port.json'))
units = {}
for w in S['waves']:
    for b in w['batches']:
        for it in b['items']:
            units[(it['name'], it['defined_in'], it['kind'] if it['kind'] == 'type' else 'symbol')] = it
key = lambda k: '|'.join(k)
out = {}
lay = opacity.close_over_embedding(opacity.load_layout_uses(layout.t2))
sites = opacity.load_raw_sites(layout) or []
by_tag = defaultdict(list)
for k, it in units.items():
    if k[2] == 'type' and it['source_kind'] in ('struct', 'union'):
        by_tag[k[0]].append(k)
managed = defaultdict(set)
for s in sites:
    u = (s.user[0], s.user[1], 'symbol')
    if u not in units:
        continue
    for t in s.types:
        for k in ([(t[1], t[2], 'type')] if t[0] == 'exact' else by_tag.get(t[1], ())):
            if k in units:
                managed[k].add(u)
refs = defaultdict(set)
names = defaultdict(list)
for k in units:
    names[(k[0], k[1])].append(k)
for k, it in units.items():
    for part in ('scheduled', 'cut'):
        for r in it['depends_on'][part]['symbols']:
            if r['scope'] != 'scheduled':
                continue
            for d in names.get((r['name'], r['defined_in']), ()):
                if d[2] == 'symbol' and d != k:
                    refs[d].add(k)
for k, it in units.items():
    rec = {}
    if k[2] == 'type' and it['source_kind'] in ('struct', 'union'):
        rec['layout'] = sorted(key(u) for u in lay.get(k, {}) if u in units)
        rec['memory'] = sorted(key(u) for u in managed.get(k, ()))
    elif it['source_kind'].startswith(('function', 'global')):
        rec['referrers'] = sorted(key(u) for u in refs.get(k, ()) if units[u]['source_kind'] != 'macro')
    if rec:
        out[key(k)] = rec
json.dump(out, open(R / 'crustify/tmp/planner-run-6/users.json', 'w'))
print(len(out), 'units with user lists')
