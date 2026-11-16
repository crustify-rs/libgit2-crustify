#!/usr/bin/env bash
# Build a C program that uses libgit2's internal API against the C reference
# build and against a USE_RUST=ON build, for differential tests.
#   usage: link.sh <prog.c> <outdir>      -> <outdir>/<prog>_c, <outdir>/<prog>_rust
#   C_BUILD (default /work/build-c), RUST_BUILD (default /work/builds/foundation)
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC_ROOT="$(cd "$HERE/../.." && pwd)"
C_BUILD="${C_BUILD:-/work/build-c}"
RUST_BUILD="${RUST_BUILD:-/work/builds/foundation}"
CC="${CC:-clang}"
src=$1; out=$2; name=$(basename "$src" .c)
mkdir -p "$out"
CFLAGS=(-O1 -g -D_GNU_SOURCE -I "$SRC_ROOT/src/util" -I "$SRC_ROOT/src/libgit2" -I "$SRC_ROOT/include" -I "$C_BUILD/gen_headers")
"$CC" "${CFLAGS[@]}" -c "$src" -o "$out/$name.o"
<<<<<<< HEAD
# The Rust build's generated headers define GIT_RUST, which changes the
# layout of types the Rust defines (git_hash_ctx, and so git_filebuf): the
# driver is compiled again against them for the Rust link.
RUST_CFLAGS=("${CFLAGS[@]/#$C_BUILD\/gen_headers/$RUST_BUILD/gen_headers}")
link_line="$(awk '/^build util_tests: /{f=1} f{print} f&&/^$/{exit}' "$C_BUILD/build.ninja")"
=======
# The C reference's objects come from a test binary's link edge: util_tests,
# or the one a driver names in a "difftest-link: <target>" comment.
edge=$(sed -n 's/.*difftest-link: *\([a-z0-9_]*\).*/\1/p' "$src" | head -n1)
edge=${edge:-util_tests}
link_line="$(awk -v e="build $edge: " 'index($0, e) == 1 {f=1} f{print} f&&/^$/{exit}' "$C_BUILD/build.ninja")"
>>>>>>> wp/u1
mapfile -t C_OBJS < <(echo "$link_line" | head -n1 | sed 's/ | .*//' | tr ' ' '\n' | grep '\.o$' | grep -v '^tests/' | sed "s|^|$C_BUILD/|")
read -r -a C_LIBS <<<"$(echo "$link_line" | sed -n 's/^ *LINK_LIBRARIES = //p')"
"$CC" -o "$out/${name}_c" "$out/$name.o" "${C_OBJS[@]}" "${C_LIBS[@]}" -ldl -lm -lpthread
if [ -f "$RUST_BUILD/libgit2_c_rest.a" ]; then
	RUST_LIBS=()
	while IFS= read -r l; do
		[ -z "$l" ] && continue
		case "$l" in /*|-*) RUST_LIBS+=("$l");; *) RUST_LIBS+=("-l$l");; esac
	done < "$RUST_BUILD/rust-link-libs.txt"
	"$CC" "${RUST_CFLAGS[@]}" -c "$src" -o "$out/$name.rust.o"
	"$CC" -o "$out/${name}_rust" "$out/$name.rust.o" -Wl,--whole-archive "$RUST_BUILD/libgit2_c_rest.a" -Wl,--no-whole-archive \
		"$RUST_BUILD/rust-target/release/liblibgit2_rs.a" "${RUST_LIBS[@]}" -ldl -lm -lpthread
fi
