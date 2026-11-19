#!/usr/bin/env bash
# The port's gates on one source tree (PORTING.md §6, INTEGRATION.md):
#   rust/gates.sh <rust-build-dir> <c-fallback-build-dir> [--no-miri] [--no-difftest]
# Runs, from the tree this script is in: include/ unchanged since the base
# commit (GATES_BASE, default 186c04b), both builds (ninja), clar on both,
# cargo fmt/clippy (with and without link-c)/test (link-c), Miri (in a
# scratch clone, -Zmiri-strict-provenance), every difftest driver, and
# last the fixture check (tests/resources unchanged by any of these).
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
top=$(git -C "$here" rev-parse --show-toplevel)
rb=${1:?rust build dir}; cb=${2:?C fallback build dir}; shift 2
miri=1; difft=1
for a in "$@"; do
	case $a in --no-miri) miri=0;; --no-difftest) difft=0;; *) echo "unknown $a"; exit 2;; esac
done
fail=0
step() { echo "== $*"; }
step include
[ -z "$(git -C "$top" diff "${GATES_BASE:-186c04b}" -- include/)" ] && echo "include/ unchanged" || { echo "FAIL: include/ changed"; fail=1; }
step builds
ninja -C "$rb" >/dev/null 2>&1 || { echo "FAIL: rust build"; fail=1; }
ninja -C "$cb" >/dev/null 2>&1 || { echo "FAIL: C fallback build"; fail=1; }
step clar
for b in "$rb" "$cb"; do
	r=$(/work/tools/runtests.sh "$b" -j 48 | tail -n 1); echo "$b: $r"
	case $r in *"failing: 0 "*) ;; *) fail=1;; esac
done
cd "$here" || exit 1
step fmt
cargo fmt --check >/dev/null || { echo "FAIL: fmt"; fail=1; }
step clippy
n=$(cargo clippy --all-targets 2>&1 | grep -cE "^(warning|error)"); echo "clippy: $n"; [ "$n" = 0 ] || fail=1
n=$(LIBGIT2_BUILD_DIR=$rb cargo clippy --all-targets --features link-c 2>&1 | grep -cE "^(warning|error)"); echo "clippy link-c: $n"; [ "$n" = 0 ] || fail=1
step "cargo test link-c"
log=$(mktemp); LIBGIT2_BUILD_DIR=$rb cargo test --features link-c >"$log" 2>&1 && echo "ok" || { echo "FAIL (log $log)"; fail=1; }
grep -E '^test result' "$log" | awk '{p+=$4; f+=$6} END{print "passed",p,"failed",f}'
if [ $miri = 1 ]; then
	step miri
	d=$(mktemp -d); git clone -q --shared "$top" "$d/src" && git -C "$d/src" checkout -q "$(git -C "$top" rev-parse HEAD)"
	if [ -n "$(git -C "$top" status --porcelain -- rust)" ]; then echo "(note: uncommitted changes are not in the Miri run)"; fi
	(cd "$d/src/rust" && CARGO_TARGET_DIR=${MIRI_TARGET_DIR:-$d/target} MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly miri test >"$d/miri.log" 2>&1) && echo "ok" || { echo "FAIL (log $d/miri.log)"; fail=1; }
	grep -E '^test result' "$d/miri.log" | awk '{p+=$4; f+=$6; i+=$8} END{print "passed",p,"failed",f,"ignored",i}'
fi
if [ $difft = 1 ]; then
	step difftests
	dl=$(mktemp)
	for d in "$here"/difftest/*_diff.c; do
		RUST_BUILD=$rb timeout 1800 "$here/difftest/run.sh" "$d" 2>&1 | grep -E 'same output|DIFFERENT|FAIL|exited' >>"$dl"
	done
	cat "$dl"
	grep -qE 'DIFFERENT|FAIL' "$dl" && fail=1
fi
step fixtures
"$here/check_fixtures.sh" || fail=1
exit $fail
