#!/usr/bin/env bash
# Prepare the benchmark data under $BENCH_DATA (default /work/bench-data).
# Idempotent: each dataset is skipped when it already exists (delete it to
# regenerate).  Requires git.git (a bare clone of git/git) to be present.
#
# Datasets (all derived from git.git, deterministic):
#   git.git       bare clone of git/git: one large pack (~425k objects)
#   git-wd        non-bare checkout of git.git (shared objects): real index
#   bigindex      synthetic index of 100,000 entries (+ TREE extension)
#   loose.git     ~30k loose objects (objects of v2.40.0..v2.47.0, unpacked)
#   refs.git      50k packed refs + 5,000 loose refs, a 10,000-entry reflog
#   config/       large config files (20k+ entries, includes, multivars)
#   packs/        a self-contained pack of v2.46.0..v2.47.0 (indexer input)
set -euo pipefail
D=${BENCH_DATA:-/work/bench-data}
SRC=$D/git.git
[ -f "$SRC/packed-refs" ] || { echo "missing $SRC (bare clone of git/git)" >&2; exit 1; }
export GIT_CONFIG_NOSYSTEM=1 GIT_AUTHOR_NAME=bench GIT_AUTHOR_EMAIL=bench@example.com \
       GIT_COMMITTER_NAME=bench GIT_COMMITTER_EMAIL=bench@example.com \
       GIT_AUTHOR_DATE='1700000000 +0000' GIT_COMMITTER_DATE='1700000000 +0000'
HEADREV=v2.47.0

if [ ! -d "$D/git-wd/.git" ]; then
	echo "== git-wd"
	git clone -q --shared --no-checkout "$SRC" "$D/git-wd.tmp"
	git -C "$D/git-wd.tmp" checkout -q -B main "$HEADREV"
	# a deep directory used by the discover benchmark
	mkdir -p "$D/git-wd.tmp/t/a/b/c/d/e/f/g/h"
	mv "$D/git-wd.tmp" "$D/git-wd"
fi

if [ ! -f "$D/bigindex/.git/index" ]; then
	echo "== bigindex"
	rm -rf "$D/bigindex.tmp"
	git init -q "$D/bigindex.tmp"
	echo "$SRC/objects" > "$D/bigindex.tmp/.git/objects/info/alternates"
	git -C "$SRC" ls-tree -r "$HEADREV" | awk '$2=="blob"{print $1, $3}' > "$D/bigindex.blobs"
	python3 -I - "$D/bigindex.blobs" > "$D/bigindex.info" <<'PY'
import sys
blobs = [l.split() for l in open(sys.argv[1])]
n = 0
out = []
# 100 top dirs x 10 subdirs x 100 files = 100,000 entries
for a in range(100):
    for b in range(10):
        for c in range(100):
            mode, oid = blobs[n % len(blobs)]
            out.append("%s %s\tsrc%03d/mod%02d/file%03d_%s.c" % ("100644" if mode != "100755" else mode, oid, a, b, c, oid[:6]))
            n += 1
sys.stdout.write("\n".join(out) + "\n")
PY
	git -C "$D/bigindex.tmp" update-index --index-info < "$D/bigindex.info"
	git -C "$D/bigindex.tmp" write-tree > /dev/null   # adds the TREE extension
	rm -f "$D/bigindex.info" "$D/bigindex.blobs"
	mv "$D/bigindex.tmp" "$D/bigindex"
fi

if [ ! -f "$D/loose.git/HEAD" ]; then
	echo "== loose.git"
	rm -rf "$D/loose.git.tmp"
	git init -q --bare "$D/loose.git.tmp"
	git -C "$SRC" pack-objects -q --revs --stdout <<<"v2.40.0..$HEADREV" \
		| git -C "$D/loose.git.tmp" unpack-objects -q
	git -C "$D/loose.git.tmp" update-ref refs/heads/main "$(git -C "$SRC" rev-parse "$HEADREV^{commit}")" 2>/dev/null || \
		echo "$(git -C "$SRC" rev-parse "$HEADREV^{commit}")" > "$D/loose.git.tmp/refs/heads/main"
	git -C "$D/loose.git.tmp" symbolic-ref HEAD refs/heads/main
	mv "$D/loose.git.tmp" "$D/loose.git"
fi

if [ ! -f "$D/refs.git/packed-refs" ]; then
	echo "== refs.git"
	rm -rf "$D/refs.git.tmp"
	git init -q --bare "$D/refs.git.tmp"
	echo "$SRC/objects" > "$D/refs.git.tmp/objects/info/alternates"
	git -C "$SRC" rev-list --first-parent -n 10000 "$HEADREV" > "$D/refs.commits"
	cp "$SRC/packed-refs" "$D/refs.packed.orig"
	python3 -I - "$D/refs.git.tmp" "$D/refs.commits" "$D/refs.packed.orig" <<'PY'
import os, sys
repo, commits, orig = sys.argv[1], [l.strip() for l in open(sys.argv[2])], sys.argv[3]
lines = {}
cur = None
for l in open(orig):
    if l.startswith('#'):
        continue
    l = l.rstrip('\n')
    if l.startswith('^'):
        lines[cur].append(l)
    else:
        oid, name = l.split(' ', 1)
        cur = name
        lines[name] = [l]
for i in range(50000):
    name = "refs/tags/synth/t%05d" % i
    lines[name] = ["%s %s" % (commits[i % len(commits)], name)]
for i in range(2000):  # packed refs shadowed by loose ones
    name = "refs/heads/loose/b%04d" % i
    lines[name] = ["%s %s" % (commits[(i + 7) % len(commits)], name)]
with open(os.path.join(repo, "packed-refs"), "w") as f:
    f.write("# pack-refs with: peeled fully-peeled sorted \n")
    for name in sorted(lines):
        f.write("\n".join(lines[name]) + "\n")
os.makedirs(os.path.join(repo, "refs/heads/loose"), exist_ok=True)
os.makedirs(os.path.join(repo, "logs/refs/heads/loose"), exist_ok=True)
sig = "bench <bench@example.com>"
for i in range(5000):
    name = "refs/heads/loose/b%04d" % i
    with open(os.path.join(repo, name), "w") as f:
        f.write(commits[i % len(commits)] + "\n")
    with open(os.path.join(repo, "logs", name), "w") as f:
        prev = "0" * 40
        for j in range(10):
            new = commits[(i + j) % len(commits)]
            f.write("%s %s %s %d +0000\tupdate: %d\n" % (prev, new, sig, 1700000000 + j, j))
            prev = new
with open(os.path.join(repo, "refs/heads/main"), "w") as f:
    f.write(commits[0] + "\n")
with open(os.path.join(repo, "logs/refs/heads/main"), "w") as f:
    prev = "0" * 40
    for j in range(10000):
        new = commits[len(commits) - 1 - j]
        f.write("%s %s %s %d +0000\tcommit: synthetic entry %d\n" % (prev, new, sig, 1600000000 + 60 * j, j))
        prev = new
with open(os.path.join(repo, "HEAD"), "w") as f:
    f.write("ref: refs/heads/main\n")
PY
	rm -f "$D/refs.commits" "$D/refs.packed.orig"
	mv "$D/refs.git.tmp" "$D/refs.git"
fi

if [ ! -f "$D/config/big.config" ]; then
	echo "== config"
	rm -rf "$D/config.tmp"; mkdir -p "$D/config.tmp"
	python3 -I - "$D/config.tmp" <<'PY'
import os, sys
d = sys.argv[1]
with open(os.path.join(d, "big.config"), "w") as f:
    f.write("[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = false\n"
            "\tlogallrefupdates = true\n\tbigFileThreshold = 512m\n\tcompression = 9\n")
    for i in range(8):
        f.write("[include]\n\tpath = inc%d.config\n" % i)
    for i in range(2000):
        f.write('[remote "r%04d"]\n\turl = https://example.com/org%d/repo%04d.git\n'
                '\tfetch = +refs/heads/*:refs/remotes/r%04d/*\n'
                '\tfetch = +refs/tags/*:refs/tags/r%04d/*\n'
                '\tpushurl = ssh://git@example.com/repo%04d.git ; trailing comment\n' % (i, i % 17, i, i, i, i))
    for i in range(2000):
        f.write('[branch "feature/b%04d"]\n\tremote = r%04d\n\tmerge = refs/heads/feature/b%04d\n'
                '\trebase = %s\n' % (i, i, i, "true" if i % 2 else "false"))
    f.write("[multi]\n")
    for i in range(2000):
        f.write("\tvalue = \"quoted value %d with \\\"escapes\\\" and \\t tab\"\n" % i)
    f.write("[user]\n\tname = Bench Mark\n\temail = bench@example.com\n")
for k in range(8):
    with open(os.path.join(d, "inc%d.config" % k), "w") as f:
        for s in range(50):
            f.write("[sect%d \"sub%d\"]\n" % (k, s))
            for e in range(10):
                f.write("\tkey%d = value-%d-%d-%d\n" % (e, k, s, e))
# a typical small repository config
with open(os.path.join(d, "small.config"), "w") as f:
    f.write("[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = false\n"
            "\tlogallrefupdates = true\n"
            '[remote "origin"]\n\turl = https://github.com/git/git.git\n'
            "\tfetch = +refs/heads/*:refs/remotes/origin/*\n"
            '[branch "master"]\n\tremote = origin\n\tmerge = refs/heads/master\n')
PY
	mv "$D/config.tmp" "$D/config"
fi
if [ ! -f "$D/packs/v2.46.0-v2.47.0.pack" ]; then
	echo "== packs"
	mkdir -p "$D/packs"
	# single-threaded delta search: deterministic output
	echo v2.46.0..v2.47.0 | git -C "$SRC" -c pack.threads=1 pack-objects -q --revs --stdout \
		> "$D/packs/v2.46.0-v2.47.0.pack.tmp"
	mv "$D/packs/v2.46.0-v2.47.0.pack.tmp" "$D/packs/v2.46.0-v2.47.0.pack"
fi

echo "bench data ready in $D"
