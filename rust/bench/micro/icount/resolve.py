import sys, subprocess, collections, bisect
prof, maps = sys.argv[1], sys.argv[2]
top = int(sys.argv[3]) if len(sys.argv) > 3 else 40
m = []
for l in open(maps):
    p = l.split()
    if len(p) < 6: continue
    a, b = [int(x, 16) for x in p[0].split('-')]
    m.append((a, b, int(p[2], 16), p[5]))
base = {}
for a, b, off, f in m:
    if f not in base or a < base[f]: base[f] = a - off if off == 0 else base.get(f, a)
for a, b, off, f in m:
    if off == 0: base[f] = a
syms = {}
def symtab(f):
    if f in syms: return syms[f]
    out = subprocess.run(['nm', '-C', '--defined-only', f], capture_output=True, text=True).stdout
    t = []
    for l in out.splitlines():
        p = l.split(' ', 2)
        if len(p) == 3 and p[1] in 'tTwWiI':
            t.append((int(p[0], 16), p[2]))
    if not t:
        out = subprocess.run(['nm', '-D', '-C', '--defined-only', f], capture_output=True, text=True).stdout
        for l in out.splitlines():
            p = l.split(' ', 2)
            if len(p) == 3 and p[1] in 'tTwWiI': t.append((int(p[0], 16), p[2]))
    t.sort(); syms[f] = ([x[0] for x in t], [x[1] for x in t]); return syms[f]
agg = collections.Counter(); tot = 0
for l in open(prof):
    a, c = l.split(); a = int(a, 16); c = int(c); tot += c
    name = '?'
    for s, e, off, f in m:
        if s <= a < e:
            v = a - base.get(f, s)
            ks, ns = symtab(f)
            i = bisect.bisect_right(ks, v) - 1
            name = (ns[i] if i >= 0 else '?') + '  [' + f.split('/')[-1] + ']'
            break
    agg[name] += c
print('total', tot)
for n, c in agg.most_common(top):
    print(f'{c:10d} {100*c/tot:5.1f}%  {n[:150]}')
