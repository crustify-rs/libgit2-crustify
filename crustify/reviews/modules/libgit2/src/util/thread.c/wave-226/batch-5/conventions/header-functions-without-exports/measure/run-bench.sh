#!/bin/bash
# An increment and a decrement of one git_atomic32, as thread.h inlines them and as a call to a
# function of another translation unit, with no LTO, as an export of Rust would be called.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); out=$(mktemp -d)
clang -O2 -c "$here/ext.c" -o "$out/ext.o"
clang -O2 "$here/main.c" "$out/ext.o" -o "$out/bench"
"$out/bench" "${1:-100000000}"
rm -r "$out"
