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
	# Drivers run from the top of the source tree (for tests/resources).
	(cd "$HERE/../.." && "$OUT/${name}_c") > "$OUT/$name.c.txt" 2>&1 || echo "$name: C exited $?"
	(cd "$HERE/../.." && "$OUT/${name}_rust") > "$OUT/$name.rust.txt" 2>&1 || echo "$name: Rust exited $?"
	if cmp -s "$OUT/$name.c.txt" "$OUT/$name.rust.txt"; then
		echo "$name: same output ($(wc -l < "$OUT/$name.c.txt") lines)"
	else
		echo "$name: DIFFERENT"; diff "$OUT/$name.c.txt" "$OUT/$name.rust.txt" | head -40; fail=1
	fi
done
# The drivers write only to scratch copies of the fixtures.
"$HERE/../check_fixtures.sh" || fail=1
exit $fail
