#!/usr/bin/env bash
# Build each difftest driver against the C reference and the Rust build, run
# both, and compare their output.  usage: run.sh [driver.c ...]
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$HERE/out"
drivers=("$@")
[ ${#drivers[@]} -gt 0 ] || drivers=("$HERE"/*_diff.c)
fail=0
for d in "${drivers[@]}"; do
	name=$(basename "$d" .c)
	"$HERE/link.sh" "$d" "$OUT"
	"$OUT/${name}_c" > "$OUT/$name.c.txt" 2>&1 || echo "$name: C exited $?"
	"$OUT/${name}_rust" > "$OUT/$name.rust.txt" 2>&1 || echo "$name: Rust exited $?"
	if cmp -s "$OUT/$name.c.txt" "$OUT/$name.rust.txt"; then
		echo "$name: same output ($(wc -l < "$OUT/$name.c.txt") lines)"
	else
		echo "$name: DIFFERENT"; diff "$OUT/$name.c.txt" "$OUT/$name.rust.txt" | head -40; fail=1
	fi
done
exit $fail
