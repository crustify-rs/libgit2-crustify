import json,csv,collections
R='/target/libgit2-bindings/'
T=R+'crustify/wavefront/codeql/t1/'
KIND={}
for r in csv.DictReader(open(T+'functions.csv')): KIND.setdefault((r['name'],'f'),'functions')
for r in csv.DictReader(open(T+'globals.csv')): KIND.setdefault((r['name'],'g'),'global_variables')
TK={}
for r in csv.DictReader(open(T+'types.csv')):
    k=r['kind']; u=r['unaliased_kind']
    if k=='struct' or u=='struct_anonymous' or (k=='typedef' and u=='struct'): TK.setdefault(r['name'],'structs')
    elif k=='union' or u=='union_anonymous' or (k=='typedef' and u=='union'): TK.setdefault(r['name'],'unions')
    elif u=='routine': TK.setdefault(r['name'],'callbacks')
    elif k=='enum' or u in('enum','enum_anonymous'): TK.setdefault(r['name'],'enums')
MACROS={r['name'] for r in csv.DictReader(open(T+'macros.csv'))}
def lib_of(path):
    if not path or path.startswith('/usr/lib/llvm'): return None
    if not path.startswith('/usr/include') and not path.startswith('/target/') : return None
    b=path.split('/')
    if 'openssl' in b:
        return 'libssl' if b[-1] in ('ssl.h','tls1.h','ssl2.h','ssl3.h','dtls1.h','sslerr.h','srtp.h') else 'libcrypto'
    if b[-1] in('zlib.h','zconf.h'): return 'libz'
    if b[-1].startswith('pcre2'): return 'libpcre2-8'
    return 'libc'
def ref_kind(route,r,units):
    key=(r['name'],r['defined_in'])
    u=units.get(key)
    if u:
        sk=u['source_kind']
        if u['kind']=='callback' or sk=='callback': return 'callbacks'
        if sk=='struct': return 'structs'
        if sk=='union': return 'unions'
        if sk=='enum': return 'enums'
        if sk.startswith('function'): return 'functions'
        if sk.startswith('global'): return 'global_variables'
        return None  # macro / generator
    if route=='types':
        return TK.get(r['name'])
    if r['name'] in MACROS and (r['name'],'f') not in KIND and (r['name'],'g') not in KIND: return None
    if (r['name'],'f') in KIND: return 'functions'
    if (r['name'],'g') in KIND: return 'global_variables'
    if TK.get(r['name'])=='callbacks': return 'callbacks'
    return None
def compute(path):
    s=json.load(open(path))
    units={}
    for w in s['waves']:
        for b in w['batches']:
            for it in b['items']: units[(it['name'],it['defined_in'])]=it
    total={}  # (m, other) -> total edges
    for m in s['modules']:
        for part in('scheduled','cut'):
            for d in m['depends_on'][part]:
                total[(m['name'],d['module'])]=d['total_edges']
    cons=collections.defaultdict(lambda: collections.defaultdict(set))
    ext=collections.defaultdict(lambda: collections.defaultdict(set))
    for it in units.values():
        mm=it['module']
        if mm is None: continue
        for part in('scheduled','cut'):
            for route in('types','symbols'):
                for r in it['depends_on'][part][route]:
                    if r['scope']=='scheduled':
                        u=units.get((r['name'],r['defined_in']))
                        if not u or u['module']==mm or u['module'] is None: continue
                        k=ref_kind(route,r,units)
                        cons[mm][u['module']].add((k,r['name'],r['defined_in']))
                    elif r['scope']=='external':
                        lib=lib_of(r['defined_in'])
                        if not lib: continue
                        k=ref_kind(route,r,units)
                        if k in (None,'enums'): continue
                        ext[mm][lib].add((k,r['name']))
    return s,units,total,cons,ext
KINDS=['structs','functions','global_variables','unions','callbacks']
def records(modname, s, total, cons, ext, split):
    recs=[]
    others=set(o for (a,o) in total if a==modname)|set(cons[modname])
    for o in sorted(others):
        if o==modname: continue
        lu,name,fnd=split(o)
        c={k:len({x for x in cons[modname][o] if x[0]==k}) for k in KINDS}
        out=total.get((modname,o),0); inc=total.get((o,modname),0)
        if out==0: continue
        recs.append({'link_unit':lu,'module':name,'foundational':fnd,'outgoing_edges':out,'incoming_edges':inc,'counters':c})
    for lib in sorted(ext[modname]):
        c={k:len({x for x in ext[modname][lib] if x[0]==k}) for k in KINDS}
        recs.append({'library':lib,'counters':c})
    return recs
def split(o):
    fnd=o.endswith(' (foundational)')
    o2=o[:-len(' (foundational)')] if fnd else o
    lu,name=o2.split('/',1)
    return lu,name,fnd
# T1 stamps glibc's anonymous pthread unions struct_anonymous; the header declares them `typedef union`
import re as _re
for _f in ('/usr/include/x86_64-linux-gnu/bits/pthreadtypes.h',):
    for _n in _re.findall(r'typedef union\s*\{.*?\}\s*(\w+)\s*;',open(_f).read(),_re.S): TK[_n]='unions'
