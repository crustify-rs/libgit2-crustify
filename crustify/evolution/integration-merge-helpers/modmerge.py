#!/usr/bin/env python3
"""modmerge.py <file>: resolve conflict hunks made only of `mod` declarations by keeping both sides,
sorted. Exit 1 if a hunk holds anything else."""
import re, sys
p = sys.argv[1]; t = open(p).read()
pat = re.compile(r'<<<<<<< [^\n]*\n(.*?)=======\n(.*?)>>>>>>> [^\n]*\n', re.S)
ok = True
def fix(m):
    global ok
    ls = [l for l in (m.group(1) + m.group(2)).split('\n') if l.strip()]
    if not all(re.match(r'^\s*(pub(\([a-z]+\))? )?mod \w+;$', l) for l in ls): ok = False; return m.group(0)
    return '\n'.join(sorted(set(ls), key=lambda l: l.split('mod ')[1])) + '\n'
t = pat.sub(fix, t)
if ok: open(p, 'w').write(t)
sys.exit(0 if ok else 1)
