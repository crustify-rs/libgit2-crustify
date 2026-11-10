"""Closed and flags enums. A closed enum's port/layout removes its integer form
once no remaining C uses it; a flags enum, whose constants C combines, has no
later stage (porting, Enums). Wavefront 88793d7 does not tell the two apart
(its skill's TODO), so this reads each scheduled enum's definition from its
source: an enum is flags when its name says so (`flag`), when a constant's value
is written as a shift (`1 << n`) or combines other constants (`A | B`), or when
it has at least three explicit nonzero values and all of them are distinct powers
of two; it is closed otherwise.

Input: crustify/tmp/planner-run-9/schedule-deps.json. Output: enums.json,
{"name|defined_in|enum": "closed" | "flags"}, and the evidence per enum."""
import json, re, collections
R = '/target/libgit2-bindings/'
T = R + 'crustify/tmp/planner-run-9/'
S = json.load(open(T + 'schedule-deps.json'))
def strip(src):
    src = re.sub(r'/\*.*?\*/', ' ', src, flags=re.S)
    return re.sub(r'//[^\n]*', ' ', src)
SRC = {}
def body(name, typedefs, path):
    if path not in SRC:
        SRC[path] = strip(open(R + 'c/' + path, errors='replace').read())
    s = SRC[path]
    for pat in [rf'\benum\s+{re.escape(name)}\s*\{{', ] + [rf'typedef\s+enum\s*\w*\s*\{{(?=[^{{}}]*\}}\s*{re.escape(t)}\s*;)'
                                                           for t in [name] + typedefs]:
        m = re.search(pat, s)
        if m:
            i = m.end(); depth = 1; j = i
            while depth and j < len(s):
                depth += {'{': 1, '}': -1}.get(s[j], 0); j += 1
            return s[i:j - 1]
    return None
def value(expr, known):
    e = expr.strip()
    try:
        return int(eval(re.sub(r'(?<=\d)[uUlL]+\b', '', e), {'__builtins__': {}}, dict(known)))
    except Exception:
        return None
# read against their definitions: git_dir_flag is a tristate (TRUE, FALSE, UNKNOWN = -1)
# and ConversionFlags a mode (strict, lenient), whatever their names say; the
# capabilities of git_index_capability_t are bits, with FROM_OWNER = -1 asking for all.
OVERRIDE = {'git_dir_flag': 'closed', 'ConversionFlags': 'closed', 'git_index_capability_t': 'flags'}
out, why = {}, {}
for w in S['waves']:
    for b in w['batches']:
        for it in b['items']:
            if it['kind'] != 'enum':
                continue
            k = f"{it['name']}|{it['defined_in']}|enum"
            txt = body(it['name'], it['typedefs'], it['defined_in'])
            if txt is None:
                out[k] = 'closed'; why[k] = 'definition not found'; continue
            known, vals, shift, combo = {}, [], False, False
            nxt = 0
            for part in [p.strip() for p in re.split(r',(?![^()]*\))', txt) if p.strip()]:
                if '=' in part:
                    n, e = [x.strip() for x in part.split('=', 1)]
                    if '<<' in e: shift = True
                    if '|' in e: combo = True
                    v = value(e, known)
                    if v is not None: vals.append(v)
                else:
                    n, v = part, nxt
                if v is not None:
                    known[n] = v; nxt = v + 1
            pow2 = [v for v in vals if v]
            flags = ('flag' in it['name'].lower() or any('flag' in t.lower() for t in it['typedefs'])
                     or shift or combo
                     or (len(pow2) >= 3 and len(set(pow2)) == len(pow2) and all(v > 0 and v & (v - 1) == 0 for v in pow2)))
            out[k] = OVERRIDE.get(it['name'], 'flags' if flags else 'closed')
            why[k] = {'shift': shift, 'combine': combo, 'explicit': vals[:12]}
json.dump(out, open(T + 'enums.json', 'w'), indent=1, sort_keys=True)
json.dump(why, open(T + 'enums-why.json', 'w'), indent=1, sort_keys=True)
print(collections.Counter(out.values()))
