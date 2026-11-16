# annot.py prof maps lib func : objdump of func with per-instruction counts
import sys, subprocess
prof, maps, libsub, func = sys.argv[1:5]
m=[]
for l in open(maps):
    p=l.split()
    if len(p)>=6 and libsub in p[5]:
        a,b=[int(x,16) for x in p[0].split('-')]; m.append((a,b,int(p[2],16),p[5]))
base=[a for a,b,o,f in m if o==0][0]; lib=m[0][3]
cnt={}
for l in open(prof):
    a,c=l.split(); a=int(a,16)
    if any(s<=a<e for s,e,o,f in m): cnt[a-base]=int(c)
out=subprocess.run(['objdump','-d','--no-show-raw-insn','-C',lib],capture_output=True,text=True).stdout
on=False
for l in out.splitlines():
    if l.endswith('<'+func+'>:'): on=True; continue
    if on and l.strip()=='' : break
    if on:
        try: a=int(l.split(':')[0],16)
        except: continue
        print('%7d %s'%(cnt.get(a,0),l))
