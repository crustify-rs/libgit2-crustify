#!/usr/bin/env python3
"""dashmerge.py <ours> <base> <theirs> [later=ours|theirs]: three-way merge of a dashboard, as the
`accounting` skill's Landing cells section settles an integration merge: every row both sides added
is kept; a row both sides changed takes, cell by cell, ours plus theirs less the base's for additive
figures, and the later landing's for percentages; prose both sides added is kept, ours first.
Writes the result over <ours>; prints the hunks it could not settle by rule (exit 1)."""
import re, subprocess, sys

ours, base, theirs = sys.argv[1:4]
later = sys.argv[4] if len(sys.argv) > 4 else 'ours'
txt = subprocess.run(['git', 'merge-file', '-p', '--diff3', '-L', 'O', '-L', 'B', '-L', 'T', ours, base, theirs],
                     text=True, capture_output=True).stdout
NUM = re.compile(r'(?P<pre>[+\-±]?\$?)(?P<n>\d+(?:\.\d+)?)(?P<suf>M?)')


def key(line):
    if line.startswith('|'):
        cells = [c.strip() for c in line.strip().strip('|').split('|')]
        if cells and cells[0].startswith('`') and len(cells) > 3:      # a module's row: module, wave, batch (+ pass)
            return ('row', cells[0], cells[1], cells[2])
        return ('row', cells[0])
    m = re.match(r'^(- \*\*[^*]+\*\*)', line)
    return ('line', m.group(1)) if m else None


def cellmerge(o, b, t):
    """Merge one line present on all three sides, number by number."""
    if o == b: return t, True
    if t == b or o == t: return o, True
    po, pb, pt = NUM.split(o), NUM.split(b), NUM.split(t)
    mo, mb, mt = list(NUM.finditer(o)), list(NUM.finditer(b)), list(NUM.finditer(t))
    if not (len(mo) == len(mb) == len(mt)):
        return (o if later == 'ours' else t), False
    out, pos = [], 0
    for x, y, z in zip(mo, mb, mt):
        out.append(o[pos:x.start()]); pos = x.end()
        vo, vb, vt = (float((m.group('pre').replace('$', '').replace('±', '') or '') + m.group('n')) for m in (x, y, z))
        tail = o[x.end():x.end() + 3]
        if tail.startswith('`%') or tail.startswith('%'):          # a percentage: the changed side's, else the later's
            tok = x.group(0) if vt == vb else z.group(0) if vo == vb else (x.group(0) if later == 'ours' else z.group(0))
        else:
            v = vo + vt - vb
            dec = len(x.group('n').split('.')[1]) if '.' in x.group('n') else 0
            pre = x.group('pre'); sign = ''
            if pre[:1] and pre[:1] in '+-±':
                sign = '+' if v > 0 or (v == 0 and pre[0] == '+') else '-' if v < 0 else '±'
                if v == 0 and pre[0] == '-': sign = '-'
            tok = f"{sign}{'$' if '$' in pre else ''}{abs(v):.{dec}f}{x.group('suf')}"
        out.append(tok)
    out.append(o[pos:])
    return ''.join(out), True


res, unsettled = [], []
lines = txt.split('\n')
i = 0
while i < len(lines):
    if lines[i].startswith('<<<<<<< O'):
        j = lines.index('||||||| B', i); k = lines.index('=======', j); e = next(n for n in range(k, len(lines)) if lines[n].startswith('>>>>>>> T'))
        O, B, T = lines[i + 1:j], lines[j + 1:k], lines[k + 1:e]
        kb = {key(l): l for l in B if key(l)}; kt = {key(l): l for l in T if key(l)}; ko = {key(l) for l in O if key(l)}
        if all(key(l) for l in O + T if l.strip()):
            merged, ok = [], True
            for l in O:
                kk = key(l)
                if kk and kk in kt:
                    m, good = cellmerge(l, kb.get(kk, l if kk not in kb else kb[kk]), kt[kk]) if kk in kb else ((l, l == kt[kk]))
                    ok &= good; merged.append(m)
                elif kk and kk in kb and kk not in kt:
                    continue                                           # theirs removed it
                else:
                    merged.append(l)
            # theirs' new rows: after the row that precedes them there, else before the first Σ row
            for n, l in enumerate(T):
                kk = key(l)
                if not kk or kk in ko or kk in kb: continue
                prev = next((key(T[m]) for m in range(n - 1, -1, -1) if key(T[m]) and any(key(x) == key(T[m]) for x in merged)), None)
                at = next((idx + 1 for idx, x in enumerate(merged) if key(x) == prev), None) if prev else None
                if at is None:
                    at = next((idx for idx, x in enumerate(merged) if x.startswith('| **Σ')), len(merged))
                merged.insert(at, l)
            if not ok: unsettled.append('\n'.join(['<<< ours'] + O + ['=== theirs'] + T))
            res += merged
        else:                                                           # prose: keep both, ours first
            res += O + ([''] if O and O[-1].strip() and T and T[0].strip() else []) + [l for l in T if l not in O or not l.strip()]
        i = e + 1
    else:
        res.append(lines[i]); i += 1
open(ours, 'w').write('\n'.join(res))
if unsettled:
    print('UNSETTLED HUNKS:'); print('\n\n'.join(unsettled)); sys.exit(1)
