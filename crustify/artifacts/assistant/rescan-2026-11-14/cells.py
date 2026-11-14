#!/usr/bin/env python3
"""Recompute the pilot's safety-audit cells from the rescans and compare with the dashboard."""
import json, re
from pathlib import Path
R = Path('/target/libgit2-crustify/crustify'); O = R / 'tmp/assistant/rescan'
COLS = ['pub raw ptr', 'pub void ptr', 'pub unsafe fn', '&cell', 'C by value', 'celled %', 'raw→std',
        'unmanaged ptr types', 'ptr Drop', 'Send/Sync', 'raw deref', 'FFI calls', 'unsafe loc %',
        'unsafe blocks']
def flat(d, p=''):
    o = {}
    for k, v in d.items():
        if isinstance(v, dict): o.update(flat(v, p + k + '.'))
        else: o[p + k] = v
    return o
def load(c, feature=''):
    with_f = O / f'{c[:12]}+{feature}.json'
    src = with_f if feature and with_f.exists() else O / f'{c[:12]}.json'
    f = flat(json.loads(src.read_text())['counts'])
    g = lambda k: f.get(k, f.get(k + '.total', 0)) or 0
    w = 'ty.ffibox.wrapper.as.borrowed.std'
    used = g('ty.ffibox.sys.of_which.raw_ptr_in_sys.of_which.used')
    loc = g('code.loc.total')
    return {
        'pub raw ptr': g('raw_ptr.total.safe.pub'),
        'pub void ptr': g('raw_ptr.field.of_which.void.pub') + sum(
            g(f'raw_ptr.fn_sig_{a}.of_which.void.{s}.pub') for a in ('arg', 'ret') for s in ('safe', 'unsafe')),
        'pub unsafe fn': g('fn.unsafe.pub'),
        '&cell': g(f'{w}.pub') - g(f'{w}.of_which.self_in_ref_accessor.pub') - g(f'{w}.of_which.self_in_trait_impl.pub'),
        'C by value': g('ty.ffibox.sys.as.by_val.total') + g('ty.ffibox.sys.as.owned.std.total'),
        'celled %': 100 * g('ty.ffibox.sys.of_which.raw_ptr_in_sys.of_which.wrapped') / used if used else 0.0,
        'raw→std': g('raw_ptr.casted_to.std.total'),
        'unmanaged ptr types': g('ty.storing_raw_ptr.of_which.none.total'),
        'ptr Drop': g('ty.storing_raw_ptr.of_which.impl_drop.total'),
        'Send/Sync': g('ty.impl_trait.of_which.std.send.total') + g('ty.impl_trait.of_which.std.sync.total'),
        'raw deref': g('raw_ptr.deref.total'),
        'FFI calls': g('code.unsafe_call.ffi'),
        'unsafe loc %': 100 * g('code.loc.unsafe') / loc if loc else 0.0,
        'unsafe blocks': g('code.unsafe_block.total') - g('code.unsafe_block.of_which.ffi_export'),
    }
def fmt(col, v):
    if col.endswith('%'):
        s = f'{v:+.2f}'
        if s in ('+0.00', '-0.00'): s = '+0.00' if v >= 0 else '-0.00'
        return f'`{s}` pp'
    return f'`{int(round(v)):+d}`'
plan = json.loads((O / 'plan.json').read_text())
dash = (R / 'artifacts/dashboard.md').read_text().split('\n')
out = []; changed = 0
for p in plan:
    b, t = load(p['base'], p.get('feature', '')), load(p['tip'], p.get('feature', ''))
    new = [fmt(c, t[c] - b[c]) for c in COLS]
    key = f"| `{p['link_unit']}/{p['module']}` | `{p['wave']}` | `{p['batch']}` |"
    rows = [(i, l) for i, l in enumerate(dash) if l.startswith(key) and l.count('|') == 20]
    if len(rows) != 1:
        out.append({'wave': p['wave'], 'batch': p['batch'], 'error': f'{len(rows)} dashboard rows'}); continue
    i, l = rows[0]; cells = [c.strip() for c in l.split('|')[1:-1]]
    old = cells[5:]
    diff = {c: (o, n) for c, o, n in zip(COLS, old, new) if o != n}
    out.append({'wave': p['wave'], 'batch': p['batch'], 'module': p['module'], 'line': i + 1, 'diff': diff,
                'new': new, 'abs': {c: (b[c], t[c]) for c in ('celled %', 'C by value')}})
    changed += bool(diff)
(O / 'cells.json').write_text(json.dumps(out, indent=1, ensure_ascii=False))
print(len(out), 'rows,', changed, 'with a cell that differs')
import collections
cc = collections.Counter(c for r in out for c in r.get('diff', {}))
print(dict(cc))
for r in out:
    if r.get('error'): print(r); continue
    if r['diff']:
        print(r['wave'], r['batch'], r['module'], '; '.join(f"{c}: {o} -> {n}" for c, (o, n) in r['diff'].items()))
