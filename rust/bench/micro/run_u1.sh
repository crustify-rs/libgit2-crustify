#!/usr/bin/env bash
# Build u1_bench.c (internal API, per call) against the C reference and the Rust build, run both
# alternately ROUNDS times (pinned to one CPU), and print the median of
# each call's ns over the rounds.  usage: run_u1.sh [ROUNDS=5] [FILTER]
#   RUST_BUILD (default /work/builds/u1), C_BUILD, BENCH_CPU (default 2)
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$HERE/out"; ROUNDS="${1:-5}"; FILTER="${2:-}"; CPU="${BENCH_CPU-2}"
RUST_BUILD="${RUST_BUILD:-/work/builds/u1}" "$HERE/../../difftest/link.sh" "$HERE/u1_bench.c" "$OUT"
PIN=(); if [ -n "$CPU" ] && command -v taskset >/dev/null; then PIN=(taskset -c "$CPU"); fi
: > "$OUT/u1.c.txt"; : > "$OUT/u1.r.txt"
for ((i = 0; i < ROUNDS; i++)); do
	"${PIN[@]}" "$OUT/u1_bench_c" $FILTER 2>/dev/null >> "$OUT/u1.c.txt"
	"${PIN[@]}" "$OUT/u1_bench_rust" $FILTER 2>/dev/null >> "$OUT/u1.r.txt"
done
echo "# rounds=$ROUNDS load=$(cut -d' ' -f1 /proc/loadavg)"
printf "%-22s %9s %9s %7s\n" call "C ns" "Rust ns" "R/C"
for n in $(awk '{print $2}' "$OUT/u1.c.txt" | awk '!s[$0]++'); do
	c=$(awk -v n="$n" '$2==n{print $3}' "$OUT/u1.c.txt" | sort -g | awk '{a[NR]=$1} END{print a[int((NR+1)/2)]}')
	r=$(awk -v n="$n" '$2==n{print $3}' "$OUT/u1.r.txt" | sort -g | awk '{a[NR]=$1} END{print a[int((NR+1)/2)]}')
	awk -v n="$n" -v c="$c" -v r="$r" 'BEGIN{printf "%-22s %9.2f %9.2f %7.2f\n", n, c, r, r/c}'
done
