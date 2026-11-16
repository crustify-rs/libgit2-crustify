#!/usr/bin/env bash
# Build util_bench_c (C reference) and util_bench_rust (Rust port) into out/.
#   C_BUILD    C reference build dir   (default /work/build-c)
#   RUST_BUILD USE_RUST=ON build dir   (default /work/builds/foundation)
#   CC         compiler                (default clang)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_ROOT="$(cd "$HERE/../.." && pwd)"
C_BUILD="${C_BUILD:-/work/build-c}"
RUST_BUILD="${RUST_BUILD:-/work/builds/foundation}"
CC="${CC:-clang}"
OUT="$HERE/out"

die() { echo "build.sh: error: $*" >&2; exit 1; }

[ -d "$C_BUILD" ]               || die "C build dir '$C_BUILD' not found"
[ -f "$C_BUILD/build.ninja" ]   || die "'$C_BUILD/build.ninja' not found (not a ninja build?)"
[ -d "$C_BUILD/gen_headers" ]   || die "'$C_BUILD/gen_headers' not found"
[ -d "$RUST_BUILD" ]            || die "Rust build dir '$RUST_BUILD' not found"
RUST_C_REST="$RUST_BUILD/libgit2_c_rest.a"
RUST_LIB="$RUST_BUILD/rust-target/release/liblibgit2_rs.a"
RUST_LIBS_TXT="$RUST_BUILD/rust-link-libs.txt"
for f in "$RUST_C_REST" "$RUST_LIB" "$RUST_LIBS_TXT"; do
	[ -f "$f" ] || die "'$f' not found (is '$RUST_BUILD' a built USE_RUST=ON tree?)"
done

mkdir -p "$OUT"

CFLAGS=(-O2 -g -D_GNU_SOURCE -DNDEBUG
	-I "$SRC_ROOT/src/util" -I "$SRC_ROOT/include" -I "$C_BUILD/gen_headers")

echo "[bench] compiling util_bench.o"
"$CC" "${CFLAGS[@]}" -c "$HERE/util_bench.c" -o "$OUT/util_bench.o"

# ---- C reference: objects + libs from the util_tests link edge, minus tests/ ----
link_line="$(awk '/^build util_tests: /{f=1} f{print} f&&/^$/{exit}' "$C_BUILD/build.ninja")"
[ -n "$link_line" ] || die "could not find 'build util_tests' in $C_BUILD/build.ninja"

mapfile -t C_OBJS < <(echo "$link_line" | head -n1 | sed 's/ | .*//' | tr ' ' '\n' \
	| grep '\.o$' | grep -v '^tests/' | sed "s|^|$C_BUILD/|")
[ "${#C_OBJS[@]}" -gt 0 ] || die "no objects parsed from util_tests link line"
for o in "${C_OBJS[@]}"; do [ -f "$o" ] || die "missing object $o (is $C_BUILD built?)"; done

read -r -a C_LIBS <<<"$(echo "$link_line" | sed -n 's/^ *LINK_LIBRARIES = //p')"

echo "[bench] linking util_bench_c (${#C_OBJS[@]} objects from $C_BUILD)"
"$CC" -O2 -o "$OUT/util_bench_c" "$OUT/util_bench.o" "${C_OBJS[@]}" \
	"${C_LIBS[@]}" -ldl -lm -lpthread

# ---- Rust variant ----
RUST_LIBS=()
while IFS= read -r l; do
	l="${l%%#*}"; l="$(echo "$l" | xargs)"
	[ -z "$l" ] && continue
	case "$l" in
		/*|*.so|*.a) RUST_LIBS+=("$l") ;;
		-*)          RUST_LIBS+=("$l") ;;
		*)           RUST_LIBS+=("-l$l") ;;
	esac
done < "$RUST_LIBS_TXT"

echo "[bench] linking util_bench_rust (against $RUST_BUILD)"
"$CC" -O2 -o "$OUT/util_bench_rust" "$OUT/util_bench.o" \
	-Wl,--whole-archive "$RUST_C_REST" -Wl,--no-whole-archive \
	"$RUST_LIB" "${RUST_LIBS[@]}" -ldl -lm -lpthread

echo "[bench] built: $OUT/util_bench_c $OUT/util_bench_rust"
