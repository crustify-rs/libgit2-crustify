#!/usr/bin/env bash
# Fail if anything under tests/resources differs from the commit or is
# untracked or ignored: the shared clar fixtures must never be written by a
# Rust test, difftest or consumer test (they use scratch copies).
# usage: rust/check_fixtures.sh   (from anywhere in the source tree)
set -euo pipefail
top=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
s=$(git -C "$top" status --porcelain --ignored -- tests/resources)
if [ -n "$s" ]; then
	echo "FAIL: tests/resources was modified:"
	echo "$s"
	exit 1
fi
echo "tests/resources unchanged"
