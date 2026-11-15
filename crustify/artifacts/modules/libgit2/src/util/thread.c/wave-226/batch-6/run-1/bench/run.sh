#!/bin/bash
# Builds pair.c twice with the plain variant's C flags: against gen5's c/, where thread.h
# inlines the builtins, and against a switched c/, where it declares the exports, linked with
# that build's C archive and a Rust static library. Then runs the two alternately, <rounds>
# times each, and prints each run's five figures, in ns per pair (switch-rehearsal.md, step 6).
#
#   run.sh <switched c/> <switched plain build> <Rust static library> <out dir> [rounds] [pairs]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
c=${1:?}; build=${2:?}; rust=${3:?}; out=${4:?}; rounds=${5:-4}; n=${6:-50000000}
mkdir -p "$out"
FLAGS="-D_GNU_SOURCE -DGIT_DEPRECATE_HARD -D_FILE_OFFSET_BITS=64 -fvisibility=hidden -fPIC -O2 -g -DNDEBUG -std=c90"
L=/target/openssl-bindings/crustify/builds/plain/install
P=/target/pcre2-bindings/crustify/builds/plain/install
Z=/target/zlib-bindings/crustify/builds/plain/install
inc() { echo "-I$1/src/util -I$1/include -I$2/gen_headers -isystem $L/include -isystem $P/include -isystem $Z/include"; }
G5=/target/libgit2-crustify/crustify/builds.gen5
clang $FLAGS $(inc $G5/c $G5/plain) -c "$here/pair.c" -o "$out/before.o"
clang "$out/before.o" -o "$out/before" -lpthread
clang $FLAGS $(inc "$c" "$build") -c "$here/pair.c" -o "$out/after.o"
clang "$out/after.o" -o "$out/after" -Wl,--start-group "$build/crustify/libcrustify.a" "$rust" -Wl,--end-group \
  -Wl,-rpath,$L/lib:$P/lib:$Z/lib $L/lib/libssl.so $L/lib/libcrypto.so $P/lib/libpcre2-8.so $Z/lib/libz.so \
  -lrt -lm -ldl -lpthread
for r in $(seq "$rounds"); do
  echo "before $r: $("$out/before" "$n" | tr '\n' ' ')"
  echo "after  $r: $("$out/after" "$n" | tr '\n' ' ')"
done
