#!/usr/bin/env bash
# Build odb_bench.c against the C reference and a Rust build (through
# rust/difftest/link.sh), then run both alternately and print the medians.
#   usage: odb_run.sh [ITERS=7] [ROUNDS=5] [FILTER]
#   C_BUILD (default /work/build-c), RUST_BUILD (default /work/builds/l-odb)
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export C_BUILD="${C_BUILD:-/work/build-c}" RUST_BUILD="${RUST_BUILD:-/work/builds/l-odb}"
ITERS=${1:-7}; ROUNDS=${2:-5}; FILTER=${3:-}
OUT="$HERE/out"
CC="${CC:-clang}" "$HERE/../../difftest/link.sh" "$HERE/odb_bench.c" "$OUT" >/dev/null
for r in $(seq "$ROUNDS"); do
	"$OUT/odb_bench_c" "$ITERS" $FILTER | sed 's/^/C /'
	"$OUT/odb_bench_rust" "$ITERS" $FILTER | sed 's/^/R /'
done | awk '
	{ who=$1; name=$3; med=$5; chk=$11; v[name, who] = v[name, who] " " med; c[name, who] = chk; names[name] = 1 }
	function median(s,   a, n, i, j, t) { n = split(s, a, " "); for (i = 1; i <= n; i++) for (j = i + 1; j <= n; j++) if (a[j] < a[i]) { t = a[i]; a[i] = a[j]; a[j] = t } return a[int((n + 1) / 2)] }
	END { printf "%-26s %10s %10s %7s %s\n", "workload", "C ms", "Rust ms", "R/C", "check"
	      for (n in names) { mc = median(v[n, "C"]); mr = median(v[n, "R"]);
	        printf "%-26s %10.3f %10.3f %7.3f %s\n", n, mc, mr, mr / mc, (c[n, "C"] == c[n, "R"] ? "same" : "MISMATCH") } }'
