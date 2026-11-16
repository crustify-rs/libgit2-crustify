#!/usr/bin/env bash
# Build pack_bench.c against the C reference and a Rust build (with
# difftest/link.sh, which links libgit2_tests' objects for the C and
# recompiles against the Rust build's headers), then run both alternately.
#   usage: pack_bench.sh [ROUNDS=5] [ITERS=7] [FILTER]
#   RUST_BUILD (default /work/builds/l-pack), C_BUILD, BENCH_CPU (default 2)
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$HERE/out"
export RUST_BUILD="${RUST_BUILD:-/work/builds/l-pack}"
rounds=${1:-5}; iters=${2:-7}; filter=${3:-}
"$HERE/../../difftest/link.sh" "$HERE/pack_bench.c" "$OUT" >/dev/null
pin=()
if [ -n "${BENCH_CPU-2}" ] && command -v taskset >/dev/null; then pin=(taskset -c "${BENCH_CPU-2}"); fi
: > "$OUT/pack_bench.tsv"
for r in $(seq 1 "$rounds"); do
	for b in c rust; do
		"${pin[@]}" "$OUT/pack_bench_$b" "$iters" "$filter" | sed "s/^/$b /" >> "$OUT/pack_bench.tsv"
	done
done
python3 -I - "$OUT/pack_bench.tsv" <<'PY'
import sys, statistics as st
rows = {}
for line in open(sys.argv[1]):
    b, _, name, _, med, _, mn = line.split()
    rows.setdefault(name, {}).setdefault(b, []).append((float(med), float(mn)))
print(f"{'workload':16} {'C ns/op':>10} {'Rust ns/op':>11} {'ratio':>6} {'minratio':>8}")
for name, d in rows.items():
    c = st.median(x[0] for x in d['c']); r = st.median(x[0] for x in d['rust'])
    cm = min(x[1] for x in d['c']); rm = min(x[1] for x in d['rust'])
    print(f"{name:16} {c:10.1f} {r:11.1f} {r/c:6.2f} {rm/cm:8.2f}")
PY
