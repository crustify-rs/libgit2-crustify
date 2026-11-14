"""Run two `git2` executables over the same command lines and compare them.

    python3 -I cli-smoke.py <worktree> <stock git2> <relinked git2>

Each command line runs in a fresh scratch repository under <worktree>/crustify/tmp/cli/work,
made with git and holding two committed files and a `user.name`, once per executable, with
`ENVV` set for `--config-env` and stdin given. Each run's exit status, standard output and
error (the scratch path masked) and the repository's top-level entries are compared; the
results go to <worktree>/crustify/tmp/cli/smoke.json. `wave-186/batch-7/run-1` ran it with the
plain variant's `git2` and the same link without `opt.c.o`, over the crate's exports.
"""
import os, shutil, subprocess, sys, json
W = sys.argv[1]; stock = sys.argv[2]; rust = sys.argv[3]
base = W + '/crustify/tmp/cli/work'
cases = [
 [], ['--help'], ['--version'], ['version'], ['-'], ['--nope'], ['-x'], ['nosuch'], ['help'], ['help', 'init'],
 ['init', '--help'], ['init', '-q', 'd1'], ['init', 'd2', '-q'], ['init', '--', 'd3'], ['init', '--', 'd4', '-q'],
 ['init', '-'], ['init', '--bare', 'd5'], ['init', 'd6', 'extra'], ['--no-pager', 'init', 'd7'],
 ['cat-file'], ['cat-file', '-t', 'HEAD'], ['cat-file', '-p', 'HEAD'], ['cat-file', '-s', 'HEAD'], ['cat-file', 'commit', 'HEAD'],
 ['cat-file', '-t'], ['cat-file', '--nope', 'HEAD'], ['cat-file', 'HEAD', '-t'], ['cat-file', '-tp', 'HEAD'],
 ['hash-object'], ['hash-object', 'f1'], ['hash-object', 'f1', 'f2'], ['hash-object', '-w', 'f1'], ['hash-object', 'f1', '-w', 'f2'],
 ['hash-object', '-t', 'blob', 'f1'], ['hash-object', '-tblob', 'f1'], ['hash-object', '-t'], ['hash-object', '--stdin'],
 ['hash-object', '-w', '-c', 'core.x=1', 'f1', 'f2'], ['hash-object', '-'], ['hash-object', '--', '-w'], ['hash-object', '--literally', 'f1'],
 ['config', 'user.name'], ['config', '--list'], ['-c', 'user.name=X', 'config', 'user.name'], ['config', '-c', 'user.name=Y', 'user.name'],
 ['-cuser.name=Z', 'config', 'user.name'], ['--config-env=user.name=ENVV', 'config', 'user.name'], ['config', '--config-env', 'user.name=ENVV', 'user.name'],
 ['--no-pager', 'config', 'user.name'], ['config'], ['config', '--get'], ['config', '--nope'],
 ['blame'], ['blame', 'f1'], ['blame', '--nope', 'f1'], ['commit', '--help'], ['commit', '-m'], ['clone'], ['clone', '--help'], ['index-pack'], ['index-pack', '--help'],
 ['cat-file', '-h'], ['hash-object', '--help', '--nope'], ['-c'], ['--config-env'], ['help', '--nope'],
]
def setup(d):
    shutil.rmtree(d, ignore_errors=True); os.makedirs(d)
    env = dict(os.environ, HOME=d, GIT_CONFIG_NOSYSTEM='1', GIT_AUTHOR_NAME='a', GIT_AUTHOR_EMAIL='a@b', GIT_COMMITTER_NAME='a', GIT_COMMITTER_EMAIL='a@b')
    subprocess.run(['git', 'init', '-q', d], check=True, env=env)
    for n, t in (('f1', 'one\n'), ('f2', 'two\n')):
        open(d + '/' + n, 'w').write(t)
    subprocess.run(['git', '-C', d, 'add', 'f1', 'f2'], check=True, env=env)
    subprocess.run(['git', '-C', d, 'commit', '-qm', 'c'], check=True, env=env)
    subprocess.run(['git', '-C', d, 'config', 'user.name', 'Cfg'], check=True, env=env)
    return env
out = []
for i, args in enumerate(cases):
    res = []
    for name, exe in (('C', stock), ('Rust', rust)):
        d = f'{base}/{name}/{i}'
        env = setup(d); env['ENVV'] = 'FromEnv'; env['PAGER'] = 'cat'; env['GIT_PAGER'] = 'cat'
        r = subprocess.run([exe] + args, cwd=d, env=env, input=b'stdin data\n', capture_output=True, timeout=60)
        listing = sorted(os.listdir(d))
        res.append((r.returncode, r.stdout.decode('latin1').replace(d, '<d>'), r.stderr.decode('latin1').replace(d, '<d>'), listing))
    same = res[0] == res[1]
    out.append({'args': args, 'same': same, 'C': res[0], 'Rust': res[1]})
    print(('same   ' if same else 'DIFFER ') + ' '.join(['git2'] + args) + ('' if same else f'\n   C:    {res[0][0]} {res[0][2][:160]!r} {res[0][1][:80]!r} {res[0][3]}\n   Rust: {res[1][0]} {res[1][2][:160]!r} {res[1][1][:80]!r} {res[1][3]}'))
json.dump(out, open(W + '/crustify/tmp/cli/smoke.json', 'w'), indent=1)
print(sum(o['same'] for o in out), 'of', len(out), 'the same')
