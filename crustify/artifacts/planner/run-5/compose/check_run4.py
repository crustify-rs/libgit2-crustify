import sys,os; sys.path.insert(0,'/target/libgit2-bindings/crustify/artifacts/planner/run-5/compose')
from compose2 import *
CUR=json.load(open(os.environ.get('CURSCHED',R+'crustify/artifacts/schedule.json')))
oldtodo=[w for w in CUR['waves'] if w['status']=='todo']
# map run-4 order: provisional ids in order should correspond to 204.. in order
print(len(out),len(oldtodo))
diff=0
for a,b in zip(out,oldtodo):
    a=dict(a); off=b['index']-a['index']
    if a['items']!=b['items'] or (a['link_unit'],a['module'],a['sublayer'],a['layer'])!=(b['link_unit'],b['module'],b['sublayer'],b['layer']): diff+=1; continue
    da={p:sorted(d+off if d>=169 else d for d in a['depends_on'][p]) for p in ('scheduled','cut')}
    if da!=b['depends_on']: diff+=1; 
print('diff waves',diff)
n=0
for a,b in zip(out,oldtodo):
    off=b['index']-a['index']
    if a['items']!=b['items']:
        n+=1
        if n<4:
            sa={json.dumps(i,sort_keys=True) for i in a['items']}; sb={json.dumps(i,sort_keys=True) for i in b['items']}
            print('items',b['index'],b['module'],list(sa-sb)[:2],list(sb-sa)[:2])
        continue
    da={p:sorted(d+off if d>=169 else d for d in a['depends_on'][p]) for p in ('scheduled','cut')}
    if da!=b['depends_on']:
        n+=1
        if n<8: print('deps',b['index'],b['module'],da,b['depends_on'])
