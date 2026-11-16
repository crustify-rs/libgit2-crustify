#!/usr/bin/env bash
# Run pack_diff.c against the C reference and the Rust build on the corpus of
# pack_corpus.sh and compare, case by case. A case the C did not survive
# (a crash on a corrupted pack: SIGSEGV/SIGBUS; a cyclic delta chain:
# SIGALRM after 20 s) is skipped, but the Rust must survive every case.
#   usage: pack_diff.sh [cases per set]     (RUST_BUILD as for link.sh)
#
# Expected differences (rust/DEFECTS.md, "Pack"): with 150 cases per set
# (906 cases), 20 are skipped (the C crashed, SIGSEGV, or looped on a
# cyclic delta chain, SIGALRM) and about 43 differ, all where the C reads
# memory it should not: a corrupted delta's header, which the C parses from
# an uninitialized buffer after a zlib error (git_odb_read_header /
# git_packfile_resolve_header return 0 with a garbage size where the Rust
# fails with "error reading from the zlib stream"), and offsets past the
# end of the pack within its last window (the C reads past the mapping;
# the Rust: "invalid pack window offset"). Run one case with
# PACKDIFF_ONLY=<case> PACKDIFF_VERBOSE=1 out/pack_diff_{c,rust} out/pack-corpus 150.
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
