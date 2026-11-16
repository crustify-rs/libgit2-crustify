#!/usr/bin/env bash
# Usage: rust/bench/compare.sh <builddir-C> <builddir-Rust> [filter]
# Builds the programs against both builds, then runs every workload
# $BENCH_ROUNDS times per build, interleaved (C,Rust / Rust,C alternating),
# pinned to $BENCH_CPU, with $BENCH_REPS timed repetitions per round.
# Prints component, workload, C and Rust median wall time, their ratio
# (Rust/C; < 1 means Rust is faster), the ratio of the minima, the median
# CPU times and their ratio, and flags check-value mismatches.
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
	echo "A=$LA $A"; echo "B=$LB $B"; echo "rounds=$ROUNDS reps=$BENCH_REPS warmup=$BENCH_WARMUP cpu=$BENCH_CPU interleave=${BENCH_INTERLEAVE:-workload}"
	echo "date=$(date -Is) host=$(hostname) load=$(cut -d' ' -f1-3 /proc/loadavg)"
} > "$OUT/info.txt"
cat "$OUT/info.txt" >&2

status=0
run_one() { # label bindir program round filter
	local f="$OUT/$1/round$4-$(printf %02d "$pi")-$3.out"
	if ! bench_exec "$2" "$3" "$5" >> "$f"; then
		echo "  bench_$3 FAILED for $1 (round $4)" >&2
		status=1
	fi
}
# Interleaving is per workload: each workload runs in its own process, A
# and B back to back (order alternating), so that bursts of machine load hit
# both builds alike.  BENCH_INTERLEAVE=program runs whole programs instead
# (less prep overhead, coarser interleaving).
INTERLEAVE=${BENCH_INTERLEAVE:-workload}
pi=0
for p in "${BENCH_PROGRAMS[@]}"; do
	pi=$((pi + 1))
	mapfile -t wls < <(BENCH_LIST=1 BENCH_FILTER="$FILTER" "$BIN_A/bench_$p")
	[ "${#wls[@]}" -gt 0 ] || continue
	[ "$INTERLEAVE" = workload ] || wls=("$FILTER")
	for ((r = 1; r <= ROUNDS; r++)); do
		echo "== $p round $r/$ROUNDS" >&2
		for ((j = 0; j < ${#wls[@]}; j++)); do
			if (( (r + j) % 2 )); then
				run_one "$LA" "$BIN_A" "$p" "$r" "${wls[j]}"; run_one "$LB" "$BIN_B" "$p" "$r" "${wls[j]}"
			else
				run_one "$LB" "$BIN_B" "$p" "$r" "${wls[j]}"; run_one "$LA" "$BIN_A" "$p" "$r" "${wls[j]}"
			fi
		done
	done
done
python3 -I "$BENCH_DIR/report.py" compare "$OUT" "$LA" "$LB" | tee "$OUT/table.txt" || status=1
echo "results: $OUT" >&2
exit $status
