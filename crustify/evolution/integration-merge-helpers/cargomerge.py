#!/usr/bin/env python3
"""cargomerge.py <Cargo.toml>: resolve conflict hunks in the export-features list: theirs' comment
lines, then every `ffi-export-…` feature line ours declares (theirs deleted switched modules')."""
import re, sys
p = sys.argv[1]; t = open(p).read(); ok = True
def fix(m):
    global ok
    o, th = m.group(1).split('\n'), m.group(2).split('\n')
    if not all(l.startswith('#') or l.startswith('ffi-export-') or not l.strip() for l in o + th): ok = False; return m.group(0)
    feats = [l for l in o if l.startswith('ffi-export-')] + [l for l in th if l.startswith('ffi-export-') and l not in o]
    return '\n'.join([l for l in th if l.startswith('#')] + feats) + '\n'
t = re.sub(r'<<<<<<< [^\n]*\n(.*?)=======\n(.*?)>>>>>>> [^\n]*\n', fix, t, flags=re.S)
if ok: open(p, 'w').write(t)
sys.exit(0 if ok else 1)
