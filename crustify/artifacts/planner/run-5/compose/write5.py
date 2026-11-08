import sys; sys.path.insert(0,'/target/libgit2-bindings/crustify/artifacts/planner/run-5/compose')
from compose2 import *
CUR=json.load(open(os.environ.get('CURSCHED',R+'crustify/artifacts/schedule.json')))
started=[w for w in CUR['waves'] if w['status']!='todo']
oldtodo=[w for w in CUR['waves'] if w['status']=='todo']
def mkey(w):
    return (w['link_unit'],w['module'],w['foundational'],frozenset((i['name'],i['defined_in'],i['kind'],i['objective'],tuple(i['field_anchors']),i['home']) for i in w['items']))
oldby={}
for w in oldtodo:
    k=mkey(w); assert k not in oldby; oldby[k]=w['index']
nxt=max(w['index'] for w in CUR['waves'])+1
final={}; kept=[]; appended=[]
for w in out:
    k=mkey(w)
    if k in oldby and oldby[k] not in final.values():
        final[w['index']]=oldby[k]; kept.append(oldby[k])
    else:
        final[w['index']]=nxt; appended.append(nxt); nxt+=1
removed=sorted(set(w['index'] for w in oldtodo)-set(kept))
fmap=lambda i: final.get(i,i)
neww=[]
for w in out:
    w=dict(w); w['index']=final[w['index']]
    w['depends_on']={p:sorted(fmap(d) for d in w['depends_on'][p]) for p in('scheduled','cut')}
    neww.append(w)
waves=sorted(started+neww,key=lambda w:w['index'])
for m in modwaves: modwaves[m]=[fmap(i) for i in modwaves[m]]
def rec(d, with_items):
    lu,name,f=split(d['module'])
    r={'link_unit':lu,'module':name,'foundational':f,'unique_edges':d['unique_edges'],'total_edges':d['total_edges']}
    if with_items: r['items']=d['items']
    return r
mods=[]
for m in sorted(SM, key=lambda m:(layer[m], list(SM).index(m))):
    if not modwaves[m]: continue
    lu,name,f=split(m)
    mods.append({'link_unit':lu,'module':name,'foundational':f,'layer':layer[m],'waves':sorted(modwaves[m]),
      'depends_on':{'scheduled':[rec(d,False) for d in sched[m]],'cut':[rec(d,True) for d in cut[m]],'external':extrecs(m)}})
doc={'_comment':CUR['_comment'],'schema_version':5,'modules':mods,'waves':waves}
idx={w['index']:w for w in waves}
for w in waves:
    for d in w['depends_on']['scheduled']+w['depends_on']['cut']: assert d in idx, (w['index'],d)
state={}
def visit(i):
    if state.get(i)==1: raise SystemExit(f'cycle at {i}')
    if state.get(i)==2: return
    state[i]=1
    for d in idx[i]['depends_on']['scheduled']: visit(d)
    state[i]=2
sys.setrecursionlimit(100000)
for i in idx: visit(i)
json.dump(doc,open(R+'crustify/tmp/planner-run-5/schedule.v5.json','w'),indent=2)
json.dump({'kept':sorted(kept),'appended':appended,'removed':removed},open(R+'crustify/tmp/planner-run-5/wave-map.json','w'))
print('waves',len(waves),'new plan',len(out),'kept',len(kept),'appended',len(appended),appended[:1],appended[-1:],'removed',len(removed))
from collections import Counter
print('new items',sum(len(w['items']) for w in out), Counter(i['objective'] for w in out for i in w['items']))
print('cats',Counter(cat.values()))
print('wrap-only',{k:v for k,v in keepers.items()})
print('old modules order equal:', [ (m['link_unit'],m['module'],m['foundational'],m['layer'],m['depends_on']) for m in mods]==[(m['link_unit'],m['module'],m['foundational'],m['layer'],m['depends_on']) for m in CUR['modules']])
