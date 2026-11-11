"""Every unit's whole user lists, as wavefront 88793d7's schedule finds them for
its release points (src/wavefront/schedule.py, _release_points): a struct's,
union's or enum's layout users closed over embedding, a struct's or union's raw
sites' functions, a function's, global's or typedef-declared function pointer's
referrers, and an inline function pointer's sites with what reaches them. Run 7's
users.py over the v6 schedule, whose items name their C kind, with the function
pointers added. Written to crustify/tmp/planner-run-10/users.json, keyed
"name|defined_in|kind". Run with wavefront's interpreter (/opt/venv/bin/python)."""
import json
from collections import defaultdict
from pathlib import Path
R = Path('/target/libgit2-bindings')
from wavefront.layout import Layout, set_config_path, set_repo_root
from wavefront import opacity
set_repo_root(R / 'c'); set_config_path(R / 'crustify/wavefront/configs/wavefront-config.json')
layout = Layout.discover(R / 'c')
S = json.load(open(R / 'crustify/tmp/planner-run-10/schedule-port.json'))
TYPE = ('struct', 'union', 'enum')
def ikey(name, f, kind):
    return (name, f, 'type' if kind in TYPE else 'symbol')
units, ckey = {}, {}
for w in S['waves']:
    for b in w['batches']:
        for it in b['items']:
            k = ikey(it['name'], it['defined_in'], it['kind'])
            units[k] = it; ckey[k] = '|'.join((it['name'], it['defined_in'], it['kind']))
lay = opacity.close_over_embedding(opacity.load_layout_uses(layout.t2))
sites = opacity.load_raw_sites(layout) or []
by_tag = defaultdict(list)
for k, it in units.items():
    if it['kind'] in ('struct', 'union'):
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
# referrers: every unit whose edges reach a function, global, macro, function
# pointer or typedef, the body-deep graph's edges as the schedule records them
refs = defaultdict(set)
SYM = ('function', 'global', 'macro', 'function_pointer', 'typedef')
for k, it in units.items():
    for part in ('scheduled', 'cut'):
        for r in it['depends_on'][part]:
            if r['scope'] != 'scheduled' or r['kind'] not in SYM:
                continue
            d = ikey(r['name'], r['defined_in'], r['kind'])
            if d in units and d != k:
                refs[d].add(k)
fp_sites = opacity.load_function_pointer_sites(layout.t2)
out = {}
for k, it in units.items():
    rec = {}
    if it['kind'] in TYPE:
        rec['layout'] = sorted(ckey[u] for u in lay.get(k, {}) if u in units)
        if it['kind'] != 'enum':
            rec['memory'] = sorted(ckey[u] for u in managed.get(k, ()))
    elif k in fp_sites:
        found = opacity.inline_users(fp_sites[k], lay, refs) - {k}
        rec['referrers'] = sorted(ckey[u] for u in found if u in units)
        rec['sites'] = sorted(f'{s[0]}|{s[1]}|{use}' for s, use in fp_sites[k])
    elif it['kind'] in ('function', 'global', 'function_pointer'):
        rec['referrers'] = sorted(ckey[u] for u in refs.get(k, ()))
    if rec:
        out[ckey[k]] = rec
# each list's length is its release point's `users`
bad = 0
for k, it in units.items():
    rec = out.get(ckey[k], {})
    for field, lst in (('opaque_after', 'layout'), ('memory_unmanaged_after', 'memory'), ('unreferenced_after', 'referrers')):
        p = it.get(field)
        if p is not None and lst in rec and p['closure']['users'] != len(rec[lst]):
            bad += 1
json.dump(out, open(R / 'crustify/tmp/planner-run-10/users.json', 'w'))
print(len(out), 'units with user lists;', bad, 'lists disagreeing with their release point')
