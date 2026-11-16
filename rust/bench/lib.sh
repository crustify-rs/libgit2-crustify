# Shared by run.sh and compare.sh (sourced).
BENCH_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SRC_DIR=$(cd "$BENCH_DIR/../.." && pwd)
CC=${CC:-clang}
BENCH_CFLAGS=${BENCH_CFLAGS:--O2 -g -std=c11 -Wall -Wextra -Wno-missing-field-initializers}
BENCH_CPU=${BENCH_CPU:-2}
export BENCH_DATA=${BENCH_DATA:-/work/bench-data}

# The programs, in run order.
BENCH_PROGRAMS=(oid odb delta objects revwalk refs config index repository hashsig util)

die() { echo "error: $*" >&2; exit 1; }

# bench_build <builddir> -> sets BIN_DIR to the directory with the programs
bench_build() {
	local bd
	bd=$(cd "$1" 2>/dev/null && pwd) || die "no build directory: $1"
	[ -e "$bd/libgit2.so" ] || die "$bd/libgit2.so not found"
	BIN_DIR="$BENCH_DIR/build/$(echo "$bd" | tr '/' '_' | sed 's/^_//')"
	mkdir -p "$BIN_DIR"
	local p inc=()
	[ -d "$bd/include" ] && inc+=("-I$bd/include")
	inc+=("-I$SRC_DIR/include")
	for p in "${BENCH_PROGRAMS[@]}"; do
		local src="$BENCH_DIR/src/bench_$p.c" out="$BIN_DIR/bench_$p"
		if [ ! -x "$out" ] || [ "$src" -nt "$out" ] || [ "$BENCH_DIR/src/bench.h" -nt "$out" ] \
		   || [ "$bd/libgit2.so" -nt "$out" ]; then
			$CC $BENCH_CFLAGS "${inc[@]}" -o "$out" "$src" \
				-L"$bd" -lgit2 -Wl,-rpath,"$bd" || die "build of $src failed"
		fi
	done
}

# bench_filter <filter> -> BENCH_FILTER pattern (a bare word matches as a substring)
bench_filter() {
	local f=$1 out="" t
	[ -z "$f" ] && { echo ""; return; }
	IFS=',' read -ra parts <<< "$f"
	for t in "${parts[@]}"; do
		case "$t" in *[\*\?\[]*) ;; *) t="*$t*" ;; esac
		out="${out:+$out,}$t"
	done
	echo "$out"
}

# bench_exec <bindir> <program> <filter-pattern>: run one program, TSV on stdout
bench_exec() {
	local pin=()
	if command -v taskset >/dev/null && [ "$BENCH_CPU" != "none" ]; then
		pin=(taskset -c "$BENCH_CPU")
	fi
	BENCH_FILTER="$3" "${pin[@]}" "$1/bench_$2"
}
