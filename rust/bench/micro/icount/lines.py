# lines.py prof maps libsubstr funcsubstr: per source line counts inside matching functions
import sys, subprocess, collections, bisect
prof, maps, libsub, fsub = sys.argv[1:5]
m=[]
for l in open(maps):
    p=l.split()
    if len(p)>=6 and libsub in p[5]:
        a,b=[int(x,16) for x in p[0].split('-')]; m.append((a,b,int(p[2],16),p[5]))
base=[a for a,b,o,f in m if o==0][0]; lib=m[0][3]
addrs={}
for l in open(prof):
    a,c=l.split(); a=int(a,16)
    if any(s<=a<e for s,e,o,f in m): addrs[a-base]=int(c)
ks=sorted(addrs)
out=subprocess.run(['addr2line','-f','-C','-i','-e',lib]+['0x%x'%k for k in ks],capture_output=True,text=True).stdout.splitlines()
# with -i, variable lines per address; rerun without -i for simplicity
out=subprocess.run(['addr2line','-f','-C','-e',lib]+['0x%x'%k for k in ks],capture_output=True,text=True).stdout.splitlines()
agg=collections.Counter()
for i,k in enumerate(ks):
    fn,loc=out[2*i],out[2*i+1]
    if fsub in fn: agg[loc.split('/')[-1]+'  '+fn[:60]]+=addrs[k]
for n,c in sorted(agg.items(), key=lambda x:-x[1])[:25]: print(c,n)
