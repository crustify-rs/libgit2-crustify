#!/bin/bash
# Counts the atomic read-modify-write and exchange instructions inlined into the C objects of a
# variant's libgit2 build, thread.c's own object left out: each one an expansion of thread.h's
# atomics that an export would turn into a call. Run from the repository root:
#   crustify/reviews/.../header-functions-without-exports/measure/count-sites.sh [variant]
set -euo pipefail
dir=crustify/builds/${1:-plain}/src
for o in $(find "$dir" -name '*.o' ! -name thread.c.o); do
  objdump -d --no-show-raw-insn "$o" |
    grep -E 'lock (xadd|add|sub|inc|dec|cmpxchg)|[[:space:]]xchg ' | grep -v 'xchg   %ax,%ax' |
    sed "s|^|$(basename "$o") |" || true
done | awk '{print $1}' | sort | uniq -c | sort -rn |
  awk '{n += $1; f++} END {print n " instructions in " f " objects"} {print}'
