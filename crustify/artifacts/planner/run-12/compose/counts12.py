"""The counts run 12's commit reports: how many todo waves and items wavefront
e529c26's order moved, added or removed, HEAD's schedule against
schedule.v12.json (run 11's counts11.py, with its change 3 dropped).

A wave is identified by its module and its content: a translation wave by its
items, a later-stage wave by its module and the modules whose switch it waits
for. An item is identified by (name, defined_in, kind, objective); it moved
when its wave's identity changed. A wave is renumbered when its index changed.
Also: the waves holding the last item of each module the settings' first ship
point names, and of reftable_error and reftable_error_str."""
import json, subprocess, collections, sys
R = '/target/libgit2-crustify/crustify/.worktrees/planner/run-12/'
T = R + 'crustify/tmp/planner-run-12/'
HEAD = json.loads(subprocess.run(['git', '-C', R, 'show', 'HEAD:crustify/artifacts/schedule.json'],
                                 capture_output=True, check=True).stdout)
NEW = json.load(open(T + 'schedule.v12.json'))
STAGE = ('port/layout', 'port/memory', 'port/unexport')
mk = lambda w: (w['link_unit'], w['module'], w['foundational'])
def ident(D):
    W = {w['index']: w for w in D['waves']}
    out = {}
    for w in D['waves']:
        if w['status'] != 'todo': continue
        if all(i['objective'] in STAGE for i in w['items']):
            frees = frozenset(mk(W[d]) for d in w['depends_on']['scheduled'] if mk(W[d]) != mk(w))
            out[w['index']] = ('stage', mk(w), frees)
        else:
            out[w['index']] = ('translation', mk(w), frozenset((i['name'], i['defined_in'], i['kind'], i['objective']) for i in w['items']))
    return W, out
def compare(A, B, label):
    WA, IA = ident(A); WB, IB = ident(B)
    ia = {v: k for k, v in IA.items()}; ib = {v: k for k, v in IB.items()}
    assert len(ia) == len(IA) and len(ib) == len(IB)
    items = lambda W, I: {(i['name'], i['defined_in'], i['kind'], i['objective']): I[w] for w in I for i in W[w]['items']}
    xa, xb = items(WA, IA), items(WB, IB)
    r = collections.OrderedDict()
    r['waves removed'] = len(set(ia) - set(ib)); r['waves added'] = len(set(ib) - set(ia))
    r['waves kept'] = len(set(ia) & set(ib))
    r['waves renumbered'] = sum(1 for v in set(ia) & set(ib) if ia[v] != ib[v])
    r['items removed'] = len(set(xa) - set(xb)); r['items added'] = len(set(xb) - set(xa))
    r['items moved'] = sum(1 for k in set(xa) & set(xb) if xa[k] != xb[k])
    by = collections.Counter()
    for k in set(xa) - set(xb): by[('removed', k[3], k[2])] += 1
    for k in set(xb) - set(xa): by[('added', k[3], k[2])] += 1
    for k in set(xa) & set(xb):
        if xa[k] != xb[k]: by[('moved', k[3], k[2])] += 1
    r['by objective and kind'] = sorted(by.items())
    print(label, json.dumps(r, default=str))
    return r
compare(HEAD, NEW, 'wavefront e529c26 (macros after their expansions):')
SHIP = ['git2/src/cli/opt.c', 'libgit2/deps/llhttp/api.c', 'libgit2/deps/llhttp/http.c', 'libgit2/deps/llhttp/llhttp.c',
        'libgit2/deps/ntlmclient/compat.h', 'libgit2/deps/ntlmclient/crypt_openssl.c', 'libgit2/deps/ntlmclient/unicode_builtin.c',
        'libgit2/deps/ntlmclient/utf8.h', 'libgit2/deps/ntlmclient/util.c', 'libgit2/deps/ntlmclient/ntlm.c',
        'libgit2/deps/reftable/error.c', 'libgit2/deps/reftable/record.c', 'libgit2/deps/xdiff/xmacros.h',
        'libgit2/src/libgit2/common.h', 'libgit2/src/libgit2/streams/mbedtls.c', 'libgit2/src/util/cc-compat.h',
        'libgit2/src/util/ctype_compat.h', 'libgit2/src/util/hash/sha1dc/ubc_check.c', 'libgit2/src/util/hashmap.h',
        'libgit2/src/util/integer.h', 'libgit2/src/util/staticstr.h', 'libgit2/src/util/strnlen.h',
        'libgit2/src/util/thread.c', 'libgit2/src/util/utf8.c', 'libgit2/src/util/wildmatch.c']
def lastwave(D, m):
    ws = [w['index'] for w in D['waves'] if f"{w['link_unit']}/{w['module']}" == m and not w['foundational']]
    return max(ws) if ws else None
def lasttrans(D, m):
    ws = [w['index'] for w in D['waves'] if f"{w['link_unit']}/{w['module']}" == m and not w['foundational']
          and any(i['objective'] in ('wrap', 'port') for i in w['items'])]
    return max(ws) if ws else None
print('first ship point: module, last wave (any item) HEAD -> now, last translation wave HEAD -> now')
for m in SHIP:
    print(f'  {m}: {lastwave(HEAD, m)} -> {lastwave(NEW, m)}; {lasttrans(HEAD, m)} -> {lasttrans(NEW, m)}')
print('  max:', max(lastwave(HEAD, m) for m in SHIP), '->', max(lastwave(NEW, m) for m in SHIP),
      '; translation', max(lasttrans(HEAD, m) for m in SHIP), '->', max(lasttrans(NEW, m) for m in SHIP))
for D, lab in ((HEAD, 'HEAD'), (NEW, 'now')):
    print(lab, [(w['index'], i['name'], i['kind'], i['objective']) for w in D['waves'] if w['status'] == 'todo'
                for i in w['items'] if i['name'] in ('reftable_error', 'reftable_error_str')])
