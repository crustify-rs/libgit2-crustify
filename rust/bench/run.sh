#!/usr/bin/env bash
# Run util_bench_c and util_bench_rust and print a comparison table.
# Usage: run.sh [ITERS] [ROUNDS] [FILTER]
#   ITERS   timed iterations per workload per process (default 7)
#   ROUNDS  alternating C/Rust process runs per workload (default 5): the
#           machine may be loaded, so C and Rust are interleaved and the
#           table shows the median and the minimum over the rounds' medians.
#   FILTER  substring of workload names
#   BENCH_CPU=N  pin to CPU N with taskset (default 2; set BENCH_CPU= to disable)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$HERE/out"
ITERS="${1:-7}"
ROUNDS="${2:-5}"
FILTER="${3:-}"
CPU="${BENCH_CPU-2}"

for b in util_bench_c util_bench_rust; do
	[ -x "$OUT/$b" ] || { echo "run.sh: $OUT/$b missing; run build.sh first" >&2; exit 1; }
done

PIN=()
if [ -n "$CPU" ] && command -v taskset >/dev/null 2>&1; then
	PIN=(taskset -c "$CPU")
fi

names=$("$OUT/util_bench_c" 1 $FILTER 2>/dev/null | awk '$1=="bench"{print $2}')
echo "# iters=$ITERS rounds=$ROUNDS pin=${PIN[*]:-none} load=$(cut -d' ' -f1 /proc/loadavg)" >&2
printf "%-22s | %9s %9s | %9s %9s | %6s %6s\n" name "C med" "C min" "Rust med" "Rust min" "r med" "r min"
printf -- "-----------------------+---------------------+---------------------+--------------\n"
for n in $names; do
	cs=(); rs=()
	for ((i = 0; i < ROUNDS; i++)); do
		cs+=("$("${PIN[@]}" "$OUT/util_bench_c" "$ITERS" "$n" 2>/dev/null | awk -v n="$n" '$1=="bench" && $2==n{print $4}')")
		rs+=("$("${PIN[@]}" "$OUT/util_bench_rust" "$ITERS" "$n" 2>/dev/null | awk -v n="$n" '$1=="bench" && $2==n{print $4}')")
	done
	printf "%s\n" "${cs[@]}" | sort -g > "$OUT/.c"; printf "%s\n" "${rs[@]}" | sort -g > "$OUT/.r"
	cmed=$(awk '{a[NR]=$1} END{print a[int((NR+1)/2)]}' "$OUT/.c"); cmin=$(head -1 "$OUT/.c")
	rmed=$(awk '{a[NR]=$1} END{print a[int((NR+1)/2)]}' "$OUT/.r"); rmin=$(head -1 "$OUT/.r")
	awk -v n="$n" -v cm="$cmed" -v cn="$cmin" -v rm="$rmed" -v rn="$rmin" \
		'BEGIN{printf "%-22s | %9.2f %9.2f | %9.2f %9.2f | %6.3f %6.3f\n", n, cm, cn, rm, rn, rm/cm, rn/cn}'
done
echo "(times in ms; r = Rust / C, < 1 means the Rust is faster)"
