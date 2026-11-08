import json, csv, collections, re, sys
sys.path.insert(0,'/target/libgit2-bindings/crustify/artifacts/planner/run-5/compose')
import deps as D
R='/target/libgit2-bindings/'
S=json.load(open(R+'crustify/tmp/planner-run-5/schedule-port.json'))
import os
M=json.load(open(os.environ.get('MODULES',R+'crustify/artifacts/modules.json')))
OLD=json.load(open(os.environ.get('OLDSCHED',R+'crustify/tmp/planner-run-5/schedule.started.json')))
FND=' (foundational)'
def split(key):
    f=key.endswith(FND); k=key[:-len(FND)] if f else key
    lu,name=k.split('/',1); return lu,name,f
def keyof(lu,name,f): return f"{lu}/{name}"+(FND if f else '')
MODS={}
for lu in M['link_units']:
    for x in lu['modules']: MODS[keyof(lu['name'],x['name'],x['foundational'])]=x
OBJ={k:x['objective']['objective'] for k,x in MODS.items()}
def home(key):
    lu,name,f=split(key)
    x=MODS[key]
    p=name
    if x['translation_unit'] is None:
        d,_,stem=p.rpartition('/'); stem=stem.rsplit('.',1)[0]+'_h'; p=(d+'/' if d else '')+stem
    else: p=p.rsplit('.',1)[0]
    parts=[re.sub(r'[^A-Za-z0-9_]','_',s) for s in p.split('/')]
    KW={'as','break','const','continue','crate','else','enum','extern','false','fn','for','if','impl','in','let','loop','match','mod','move','mut','pub','ref','return','self','static','struct','super','trait','true','type','unsafe','use','where','while','async','await','dyn','abstract','become','box','do','final','macro','override','priv','typeof','unsized','virtual','yield','try','gen'}
    parts=[s+'_' if s in KW else s for s in parts]
    return f"src/{lu}/"+'/'.join(parts)+'.rs'
# ---- units of the plan schedule
UNITS={}; UWAVE={}
for w in S['waves']:
    for b in w['batches']:
        for it in b['items']:
            k=(it['name'],it['defined_in'],it['kind']); UNITS[k]=it; UWAVE[k]=w['index']
NF=collections.defaultdict(list)
for k,it in UNITS.items(): NF[(k[0],k[1])].append(k)
def find(name,file,route):
    ks=NF.get((name,file),[])
    if route=='types': ks=[k for k in ks if k[2]=='type']
    else: ks=[k for k in ks if k[2]!='type']
    return ks[0] if ks else None
DEPRECATED_ANCHORS={('git_attr_options','commit_id'),('git_blob_filter_options','commit_id'),('git_filter_options','commit_id'),('git_rebase_options','signing_cb'),('git_remote_callbacks','resolve_url'),('git_remote_callbacks','update_tips')}
OUT_OF_SCOPE={('git_url_resolve_cb','include/git2/remote.h')}
# ---- module order: wavefront's, with each foundational module's uses of other modules cut
SM={m['name']:m for m in S['modules']}
def unit_edges(src_mod, dst_mod):
    out=[]
    for k,it in UNITS.items():
        if it['module']!=src_mod: continue
        for part in('scheduled','cut'):
            for route in('types','symbols'):
                for r in it['depends_on'][part][route]:
                    if r['scope']!='scheduled': continue
                    t=find(r['name'],r['defined_in'],route)
                    if t and UNITS[t]['module']==dst_mod:
                        out.append({'from':{'name':it['name'],'defined_in':it['defined_in']},'to':{'name':t[0],'defined_in':t[1]},'kind':'type' if t[2]=='type' else 'symbol'})
    return out
sched=collections.defaultdict(list); cut=collections.defaultdict(list)
for m in S['modules']:
    for d in m['depends_on']['scheduled']:
        if MODS[m['name']]['foundational'] and not MODS[d['module']]['foundational']:
            cut[m['name']].append(dict(d, items=unit_edges(m['name'],d['module'])))
        else: sched[m['name']].append(d)
    for d in m['depends_on']['cut']: cut[m['name']].append(d)
layer={}
def L(m):
    if m in layer: return layer[m]
    layer[m]=0
    layer[m]=max([L(d['module'])+1 for d in sched[m]],default=0); return layer[m]
for m in SM: L(m)
# ---- external records
ext=collections.defaultdict(lambda: collections.defaultdict(list))
for k,it in UNITS.items():
    if it['module'] is None: continue
    for part in('scheduled','cut'):
        for route in('types','symbols'):
            for r in it['depends_on'][part][route]:
                if r['scope']!='external': continue
                lib=D.lib_of(r['defined_in'])
                if not lib: continue
                kd=D.ref_kind(route,r,{})
                if kd is None and route=='symbols': continue   # macros, builtins
                if kd is None: continue
                ext[it['module']][lib].append({'from':{'name':it['name'],'defined_in':it['defined_in']},'to':{'name':r['name'],'defined_in':r['defined_in']},'kind':'type' if route=='types' else 'symbol'})
def extrecs(m):
    out=[]
    for lib in sorted(ext[m]):
        items=sorted({json.dumps(e,sort_keys=True) for e in ext[m][lib]})
        items=[json.loads(e) for e in items]
        out.append({'library':lib,'unique_edges':len({(e['to']['name'],e['to']['defined_in']) for e in items}),'total_edges':len(items),'items':items})
    return out
# ---- layout users (field accesses, and the layout-use query's sizeof/value/pointer-arithmetic/embedding rows)
users=collections.defaultdict(lambda: collections.defaultdict(set)); embeds=collections.defaultdict(set)
def ty(name,file): return find(name,file,'types')
def fn(name,file): return find(name,file,'symbols')
for r in csv.DictReader(open(R+'crustify/wavefront/codeql/t2/fa_with_root.csv')):
    t=ty(r['root_struct_name'],r['root_struct_def_file'])
    u=fn(r['enclosing_name'],r['access_file']) if (t and r['enclosing_name']) else None
    if t and u: users[t][UNITS[u]['module']].add(('field',u))
for r in csv.DictReader(open(R+'crustify/artifacts/planner/run-4/layout-uses/layout_uses.csv')):
    t=ty(r['tn'],r['tf'])
    if not t: continue
    u=ty(r['un'],r['uf']) if r['uk']=='struct' else fn(r['un'],r['uf'])
    if not u: continue
    if r['uk']=='struct' and u!=t: embeds[t].add(u)
    users[t][UNITS[u]['module']].add((r['use'],u))
# scheduled-ancestor relation over the module order
anc={}
def ancestors(m):
    if m in anc: return anc[m]
    anc[m]=set()
    a=set()
    for d in sched[m]: a|={d['module']}|ancestors(d['module'])
    anc[m]=a; return a
for m in SM: ancestors(m)
TYPES=[k for k,it in UNITS.items() if it['kind']=='type' and it['source_kind'] in('struct','union') and it['module']]
cat={}; lastusers={}; keepers={}
def foreign(t):
    mt=UNITS[t]['module']
    return {m for m in users[t] if m and m!=mt}
# propagate: the users of an embedding struct use the layout it carries
eff={t:set(foreign(t)) for t in TYPES}
changed=True
while changed:
    changed=False
    for t in TYPES:
        for s in embeds[t]:
            if s in eff:
                add=(eff[s]|({UNITS[s]['module']} if UNITS[s]['module']!=UNITS[t]['module'] else set()))-{UNITS[t]['module']}-eff[t]
                if add: eff[t]|=add; changed=True
for t in TYPES:
    mt=UNITS[t]['module']
    if OBJ[mt]=='wrap': cat[t]='wrap'; continue
    stay=[m for m in eff[t] if OBJ[m]=='wrap']
    if stay: cat[t]='wrap-only'; keepers[t]=sorted(stay); continue
    later={m for m in eff[t] if mt not in ancestors(m) and False} # placeholder
    later={m for m in eff[t] if m not in ancestors(mt)}
    if not later: cat[t]='port'; continue
    last={m for m in later if not any(m in ancestors(o) for o in later if o!=m)}
    cat[t]='wrap+port'; lastusers[t]=sorted(last)
