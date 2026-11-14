#!/usr/bin/env python3
"""Rescan the pilot's batches with the fixed ffimap: for each batch of the waves named, scan
its base (the parent of its first commit) and its tip (its last commit), each in a detached
worktree linked to the builds generation its C belongs to. Scans are cached by commit in
this directory; `plan.json` maps each batch to its two commits."""
import json, os, subprocess, sys
from pathlib import Path

REPO = Path('/target/libgit2-crustify')
OUT = REPO / 'crustify/tmp/assistant/rescan'
WAVES = [186, 194, 197, 200, 225, 229, 232]
SEAMS = ('as_ptr,as_mut_ptr,as_void_ptr,from_ptr,from_void_ptr,from_raw_parts,into_raw_parts,'
         'into_raw,from_c,into_c,as_c_ptr,from_raw,as_object')


def sh(*a, cwd=REPO, check=True):
    return subprocess.run(a, cwd=cwd, check=check, text=True, capture_output=True).stdout.strip()


def generation(csha: str) -> Path:
    if csha.startswith('0551dfd4a') and (REPO / 'crustify/builds.gen0').exists():
        return REPO / 'crustify/builds.gen0'
    for g in sorted((REPO / 'crustify').glob('builds.gen*')):
        c = g / 'c'
        if c.exists() and sh('git', 'rev-parse', 'HEAD', cwd=c) == csha:
            return g
    raise SystemExit(f'no builds generation for C {csha}')


def scan(commit: str, feature: str = '') -> str:
    if feature and feature not in sh('git', 'show', f'{commit}:Cargo.toml'):
        return 'feature not in this manifest'
    out = OUT / (f'{commit[:12]}+{feature}.json' if feature else f'{commit[:12]}.json')
    if out.exists() and out.stat().st_size > 1000:
        return 'cached'
    wt = OUT / f'wt-{commit[:12]}'
    extra = ['--features', feature] if feature else []
    csha = sh('git', 'ls-tree', commit, 'c').split()[2]
    gen = generation(csha)
    try:
        sh('git', 'worktree', 'add', '--detach', str(wt), commit)
        sh('git', 'worktree', 'add', '--detach', str(wt / 'c'), csha, cwd=REPO / 'c')
        os.symlink(gen, wt / 'crustify/builds')
        for g in (REPO / 'crustify').glob('builds.gen*'):
            if not (wt / 'crustify' / g.name).exists():
                os.symlink(g, wt / 'crustify' / g.name)
        side = os.path.realpath(wt / 'crustify/builds/plain/../../../..')
        env = dict(os.environ, CRUSTIFY_BUILD_VARIANT='plain',
                   CARGO_TARGET_DIR=str(OUT / 'target'),
                   PKG_CONFIG_PATH=f'{side}/pcre2-bindings/crustify/builds/plain/install/lib/pkgconfig')
        r = subprocess.run(['/opt/venv/bin/ffimap', str(wt), '--ffibox', '--sys-crates',
                            'crate::sys,pcre2_sys', '--regions', 'ffi_import,ffi_export',
                            '--seams', SEAMS, *extra], env=env, text=True, capture_output=True)
        if r.returncode != 0:
            (OUT / f'{commit[:12]}{feature}.err').write_text(r.stderr[-6000:])
            return f'failed rc={r.returncode}'
        out.write_text(r.stdout)
        return f'ok gen={gen.name}'
    finally:
        subprocess.run(['git', 'worktree', 'remove', '--force', str(wt / 'c')], cwd=REPO / 'c',
                       capture_output=True)
        subprocess.run(['git', 'worktree', 'remove', '--force', str(wt)], cwd=REPO,
                       capture_output=True)


def main() -> None:
    import glob
    plan = json.loads((OUT / 'plan.json').read_text())
    todo = []
    for p in plan:
        fs = sorted(glob.glob(str(REPO / f"crustify/artifacts/modules/{p['link_unit']}/{p['module']}/wave-{p['wave']}/batch-{p['batch']}/run-*/ffimap.json")))
        feats = json.loads(Path(fs[-1]).read_text())['scanner'].get('features') or [] if fs else []
        p['feature'] = feats[0] if feats else ''
        if p['feature']:
            for c in (p['base'], p['tip']):
                if (c, p['feature']) not in todo:
                    todo.append((c, p['feature']))
    (OUT / 'plan.json').write_text(json.dumps(plan, indent=1))
    print(len(todo), 'scans with a feature', flush=True)
    for i, (c, f) in enumerate(todo, 1):
        print(i, c[:12], f, scan(c, f), flush=True)


main()
