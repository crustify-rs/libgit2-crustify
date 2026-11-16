#!/usr/bin/env bash
# Run pack_diff.c against the C reference and the Rust build on the corpus of
# pack_corpus.sh and compare, case by case. A case the C did not survive
# (a crash on a corrupted pack: SIGSEGV/SIGBUS; a cyclic delta chain:
# SIGALRM after 20 s) is skipped, but the Rust must survive every case.
#   usage: pack_diff.sh [cases per set]     (RUST_BUILD as for link.sh)
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$HERE/out"
mkdir -p "$OUT"
"$HERE/pack_corpus.sh" "$OUT/pack-corpus"
"$HERE/link.sh" "$HERE/pack_diff.c" "$OUT"
n=${1:-300}
"$OUT/pack_diff_c" "$OUT/pack-corpus" "$n" > "$OUT/pack_diff.c.txt" 2>&1 || echo "C exited $?"
"$OUT/pack_diff_rust" "$OUT/pack-corpus" "$n" > "$OUT/pack_diff.rust.txt" 2>&1 || echo "Rust exited $?"
python3 -I - "$OUT/pack_diff.c.txt" "$OUT/pack_diff.rust.txt" <<'PY'
import sys, re
def load(p):
    d = {}
    for line in open(p, errors="replace"):
        m = re.match(r"case (\S+): (.*)", line.rstrip("\n"))
        if m:
            d[m.group(1)] = m.group(2)
    return d
c, r = load(sys.argv[1]), load(sys.argv[2])
same = skipped = 0
bad = []
for k, v in c.items():
    rv = r.get(k)
    if rv is None or rv.startswith(("SIGNAL", "EXIT")):
        bad.append((k, v, rv))
    elif v.startswith(("SIGNAL", "EXIT")):
        skipped += 1
    elif v == rv:
        same += 1
    else:
        bad.append((k, v, rv))
print(f"pack_diff: {len(c)} cases, {same} same, {skipped} skipped (C crashed or hung), {len(bad)} different")
for k, v, rv in bad[:30]:
    print(f"  {k}\n    C:    {v}\n    Rust: {rv}")
sys.exit(1 if bad else 0)
PY
