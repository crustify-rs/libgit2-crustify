#!/usr/bin/env bash
# Usage: rust/bench/compare.sh <builddir-C> <builddir-Rust> [filter]
# Builds the programs against both builds, then runs each program
# $BENCH_ROUNDS times per build, interleaved (C,Rust,C,... alternating which
# goes first), pinned to $BENCH_CPU, with $BENCH_REPS timed repetitions per
# round.  Prints component, workload, C median, Rust median, ratio
# (Rust/C; < 1 means Rust is faster), and flags check-value mismatches.
# Results: $BENCH_OUT (default rust/bench/results/compare-<timestamp>)/
#   compare.tsv, compare.json, and the raw per-round outputs.
# Labels: $LABEL_A (default C), $LABEL_B (default Rust).
set -euo pipefail
. "$(dirname "$0")/lib.sh"
[ $# -ge 2 ] || die "usage: $0 <builddir-C> <builddir-Rust> [filter]"
A=$(cd "$1" && pwd); B=$(cd "$2" && pwd)
FILTER=$(bench_filter "${3:-}")
LA=${LABEL_A:-C}; LB=${LABEL_B:-Rust}
[ "$LA" != "$LB" ] || die "LABEL_A and LABEL_B must differ"
ROUNDS=${BENCH_ROUNDS:-3}
export BENCH_REPS=${BENCH_REPS:-3} BENCH_WARMUP=${BENCH_WARMUP:-1}
OUT=${BENCH_OUT:-$BENCH_DIR/results/compare-$(date +%Y%m%d-%H%M%S)}
mkdir -p "$OUT/$LA" "$OUT/$LB"

bench_build "$A"; BIN_A=$BIN_DIR
bench_build "$B"; BIN_B=$BIN_DIR
{
	echo "A=$LA $A"; echo "B=$LB $B"; echo "rounds=$ROUNDS reps=$BENCH_REPS warmup=$BENCH_WARMUP cpu=$BENCH_CPU"
	echo "date=$(date -Is) host=$(hostname) load=$(cut -d' ' -f1-3 /proc/loadavg)"
} > "$OUT/info.txt"
cat "$OUT/info.txt" >&2

status=0
run_one() { # label bindir program round
	local f="$OUT/$1/round$4-$3.out"
	if ! bench_exec "$2" "$3" "$FILTER" > "$f"; then
		echo "  bench_$3 FAILED for $1 (round $4)" >&2
		status=1
	fi
}
for p in "${BENCH_PROGRAMS[@]}"; do
	[ -n "$(BENCH_LIST=1 BENCH_FILTER="$FILTER" "$BIN_A/bench_$p")" ] || continue
	for ((r = 1; r <= ROUNDS; r++)); do
		echo "== $p round $r/$ROUNDS" >&2
		if (( r % 2 )); then
			run_one "$LA" "$BIN_A" "$p" "$r"; run_one "$LB" "$BIN_B" "$p" "$r"
		else
			run_one "$LB" "$BIN_B" "$p" "$r"; run_one "$LA" "$BIN_A" "$p" "$r"
		fi
	done
done
python3 -I "$BENCH_DIR/report.py" compare "$OUT" "$LA" "$LB" | tee "$OUT/table.txt" || status=1
echo "results: $OUT" >&2
exit $status
