#!/usr/bin/env bash
# Usage: rust/bench/run.sh <builddir> [filter]
# Builds the benchmark programs against <builddir>/libgit2.so, runs them
# pinned to $BENCH_CPU, and writes results.tsv and results.json to
# $BENCH_OUT (default rust/bench/results/run-<timestamp>).
# filter: comma-separated; a bare word matches "component/workload" as a
# substring, a word with glob characters is an fnmatch pattern.
set -euo pipefail
. "$(dirname "$0")/lib.sh"
[ $# -ge 1 ] || die "usage: $0 <builddir> [filter]"
BUILD=$(cd "$1" && pwd)
FILTER=$(bench_filter "${2:-}")
OUT=${BENCH_OUT:-$BENCH_DIR/results/run-$(date +%Y%m%d-%H%M%S)}
mkdir -p "$OUT"
bench_build "$BUILD"
TSV="$OUT/results.tsv"
printf 'component\tworkload\treps\tmedian_ns\tmin_ns\tops\tcheck\tsamples_ns\n' > "$TSV"
status=0
for p in "${BENCH_PROGRAMS[@]}"; do
	[ -n "$(BENCH_LIST=1 BENCH_FILTER="$FILTER" "$BIN_DIR/bench_$p")" ] || continue
	echo "== $p" >&2
	if ! bench_exec "$BIN_DIR" "$p" "$FILTER" > "$OUT/$p.out"; then
		echo "  bench_$p FAILED (exit status $?)" >&2
		status=1
	fi
	grep -v '^#' "$OUT/$p.out" | tee -a "$TSV" | awk -F'\t' '{
		printf "  %-10s %-30s median %11.3f ms  min %11.3f ms  check %s\n", $1, $2, $4/1e6, $5/1e6, $7 }' >&2
done
python3 -I "$BENCH_DIR/report.py" json "$BUILD" "$TSV" > "$OUT/results.json"
echo "results: $TSV $OUT/results.json" >&2
exit $status
