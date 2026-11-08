import sys; sys.path.insert(0,'/target/libgit2-bindings/crustify/artifacts/planner/run-5/compose')
from compose import *
# ---- what has started or landed
started_mod=collections.defaultdict(list)   # module key -> old started waves (index order)
held={}   # (name, defined_in, kind) -> (objective, old wave index) for items a started wave holds
for w in OLD['waves']:
    if w['status']=='todo': continue
    k=keyof(w['link_unit'],w['module'],w['foundational'])
    started_mod[k].append(w['index'])
    for b in w['batches']:
        for it in b['items']:
            held.setdefault((it['name'],it['defined_in'],it['kind']),(b['kind'],w['index']))
def is_held(k,objective):
    h=held.get(k)
    return h is not None and h[0]==objective
# ---- items per wavefront wave
VISIBLE_SYM={'function_exported','function_inline_header','global_extern','callback'}
def visible(it):
    if it['kind']=='type':
        if it['source_kind']=='enum': return False
        return any(f.endswith('.h') for f in it['declared_in']+[it['defined_in']])
    return it['source_kind'] in VISIBLE_SYM or it['kind']=='callback'
def anchors(it):
    return [f for f in it.get('used_fields',[]) if (it['name'],f) not in DEPRECATED_ANCHORS]
def planned(k,objective):
    it=UNITS[k]
    return {'name':it['name'],'defined_in':it['defined_in'],'kind':it['kind'],'field_anchors':anchors(it),'home':home(it['module']),'objective':objective}
regular=[]     # (S wave, [items])
deferred=collections.defaultdict(list)  # module -> [(sublayer, k)]
placed={}      # unit key -> ('new', S wave idx) or ('old', old idx)
for k,(obj,oi) in held.items(): pass
for w in S['waves']:
    items=[]
    for b in w['batches']:
        for it in b['items']:
            k=(it['name'],it['defined_in'],it['kind'])
            m=it['module']
            if m is None or (it['name'],it['defined_in']) in OUT_OF_SCOPE: continue
            if OBJ[m]=='wrap':
                if not visible(it): continue
                if is_held(k,'wrap'): placed[k]=('old',held[k][1]); continue
                items.append(planned(k,'wrap')); placed[k]=('new',w['index']); continue
            c=cat.get(k,'port')
            if it['kind']=='type' and it['source_kind'] in('struct','union'):
                if c=='port':
                    items.append(planned(k,'port')); placed[k]=('new',w['index'])
                else:
                    if is_held(k,'wrap'): placed[k]=('old',held[k][1])
                    else: items.append(planned(k,'wrap')); placed[k]=('new',w['index'])
                    if c=='wrap+port': deferred[m].append((w['sublayer'],k))
            else:
                items.append(planned(k,'port')); placed[k]=('new',w['index'])
    if items: regular.append((w,items))
# ---- deferred port waves: one per sublayer of the deferred types, after the module's regular waves
mod_last_S={}
for w,items in regular: mod_last_S[w['module']]=w['index']
dwaves=[]   # (module, sublayer rank, [k], last-user modules)
for m,lst in deferred.items():
    by=collections.defaultdict(list)
    for sl,k in lst: by[sl].append(k)
    for sl in sorted(by):
        lu=sorted({x for k in by[sl] for x in lastusers[k]})
        dwaves.append((m,sl,sorted(by[sl]),lu))
# ---- order and index
def wkey_regular(w): return (layer[w['module']], w['sublayer'], w['index'])
regular.sort(key=lambda p: wkey_regular(p[0]))
base=max(w['index'] for w in OLD['waves'])+1
new=[]   # dicts
s2n={}
for w,items in regular:
    idx=base+len(new); s2n[w['index']]=idx
    new.append({'_m':w['module'],'_S':w['index'],'index':idx,'items':items})
mod_last_new={}
for nw in new: mod_last_new[nw['_m']]=nw['index']
# deferred waves: ordered so that every wave they wait for comes first
pending=list(dwaves); done_mod_last=dict(mod_last_new)
while pending:
    pending.sort(key=lambda d: (max([done_mod_last.get(u,0) for u in d[3]]+[done_mod_last[d[0]]]), layer[d[0]], d[1]))
    m,sl,ks,lu=pending.pop(0)
    idx=base+len(new)
    items=[planned(k,'port') for k in ks]
    for k in ks: placed[('port',)+k]=('new',idx)
    new.append({'_m':m,'_S':None,'_after':lu,'index':idx,'items':items})
    done_mod_last[m]=idx
pos={nw['index']:i for i,nw in enumerate(new)}
# ---- wave records
def unit_wave(k):
    p=placed.get(k)
    if not p: return None
    return s2n.get(p[1]) if p[0]=='new' else p[1]
prev_in_mod={}
for m,l in started_mod.items(): prev_in_mod[m]=l[-1]
modwaves=collections.defaultdict(list)
for m,l in started_mod.items(): modwaves[m].extend(l)
oldsub=collections.defaultdict(lambda:-1)
for w in OLD['waves']:
    if w['status']!='todo':
        k=keyof(w['link_unit'],w['module'],w['foundational']); oldsub[k]=max(oldsub[k],w['sublayer'])
out=[]
for nw in new:
    m=nw['_m']; lu,name,f=split(m)
    schedd=set(); cutd=set()
    if m in prev_in_mod: schedd.add(prev_in_mod[m])
    if nw['_S'] is not None:
        for it in nw['items']:
            k=(it['name'],it['defined_in'],it['kind']); u=UNITS[k]
            for part in('scheduled','cut'):
                for route in('types','symbols'):
                    for r in u['depends_on'][part][route]:
                        if r['scope']!='scheduled': continue
                        t=find(r['name'],r['defined_in'],route)
                        if not t: continue
                        wi=unit_wave(t)
                        if wi is None or wi==nw['index']: continue
                        if wi<base or pos[wi]<pos[nw['index']]: schedd.add(wi)
                        else: cutd.add(wi)
    else:
        # the last wave of each last user module, its deferred ones included, after which it is integrated
        for u in nw['_after']:
            lastw=max([x['index'] for x in new if x['_m']==u and pos[x['index']]<pos[nw['index']]],default=None)
            if lastw is None and started_mod.get(u): lastw=started_mod[u][-1]
            if lastw is not None: schedd.add(lastw)
    sub=oldsub[m]+1+len([x for x in modwaves[m] if x>=base])
    rec={'index':nw['index'],'link_unit':lu,'module':name,'foundational':f,'layer':layer[m],'sublayer':sub,'status':'todo',
         'depends_on':{'scheduled':sorted(schedd),'cut':sorted(cutd-schedd)},'items':nw['items'],'packed_with':None,'batches':[]}
    out.append(rec); modwaves[m].append(nw['index']); prev_in_mod[m]=nw['index']
