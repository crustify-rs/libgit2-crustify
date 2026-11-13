#!/bin/bash
# Run the target's remaining C tests over a variant's build and compare them with the C test
# baseline (crustify's testing-gates skill, Regressions: a module's switch; setup, Baseline).
#
#   c-tests.sh <variant dir> <out dir> [jobs]
#
# <variant dir> is a built variant, crustify/builds/<variant>, whose C tests link the safe
# crate's Rust static library; its name is the variant whose record of
# crustify/artifacts/c-baseline.json the run is compared with. Nothing is written into it:
# every output goes under <out dir>.
#
# 1. build.json's `test` command, `ctest --output-on-failure -R '^(offline|util)$'`, over a
#    copy of the variant's CTestTestfile.cmake tree, so that ctest's own records stay out of
#    the variant, beside the suites of step 2: <out dir>/ctest.log, its exit status in
#    <out dir>/ctest.rc.
# 2. Every offline libgit2_tests suite and every util_tests suite alone, <jobs> side by side
#    (default 8), each under a 30-minute timeout, as the baseline ran them: TAP output in
#    <out dir>/suites/<binary>/<suite>.{tap,err,rc}. `online::*` suites need the network and
#    are left out, as the `offline` ctest target leaves them.
# 3. The tally, <out dir>/tally.json (cases passed, failed and skipped, and every suite that
#    failed a case or exited non-zero, with the sanitizer reports it printed), compared with
#    the baseline's record: the exit status is 0 when the case counts and the non-zero suites
#    (binary, suite, exit status) match it, and 1 otherwise, naming each difference. ctest's
#    status is reported beside it; under TSan the baseline's own races fail `offline`.
#
# The coverage variant's profiles go to /dev/null unless LLVM_PROFILE_FILE says otherwise.
set -uo pipefail
usage='usage: c-tests.sh <variant dir> <out dir> [jobs]'
d=$(cd -P "${1:?$usage}" && pwd) || exit 2
out=${2:?$usage}
jobs=${3:-8}
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
variant=$(basename "$d")
[ -x "$d/libgit2_tests" ] && [ -x "$d/util_tests" ] || { echo "c-tests.sh: $d has no C tests built" >&2; exit 2; }
mkdir -p "$out"
out=$(cd -P "$out" && pwd)
export LLVM_PROFILE_FILE=${LLVM_PROFILE_FILE:-/dev/null}

# 1. ctest, over a copy of the test files: they name the executables by absolute path.
rm -rf "$out/ctest"
mkdir -p "$out/ctest"
(cd "$d" && find . -name CTestTestfile.cmake -not -path './crustify/*' -exec cp --parents {} "$out/ctest" \;)
# It runs beside the suites below, as one more job.
(
  ctest --test-dir "$out/ctest" --output-on-failure -R '^(offline|util)$' >"$out/ctest.log" 2>&1
  echo $? >"$out/ctest.rc"
) &
ctest_pid=$!

# 2. Every suite alone.
rm -rf "$out/suites"
for b in libgit2_tests util_tests; do
  mkdir -p "$out/suites/$b"
  "$d/$b" -l 2>/dev/null | sed -nE 's/^ *[0-9]+: (.*)$/\1/p' | grep -v '^online::' |
    D="$d" O="$out" B="$b" xargs -P "$jobs" -I{} bash -c \
      'cd "$O" && timeout 1800 "$D/$B" -t "-s$1" > "suites/$B/$1.tap" 2> "suites/$B/$1.err"; echo $? > "suites/$B/$1.rc"' _ {}
done

wait "$ctest_pid"

# 3. The tally, against the baseline.
python3 -I "$here/c-tests-tally.py" "$out" "$here/../artifacts/c-baseline.json" "$variant"
status=$?
echo "c-tests.sh: ctest exited $(cat "$out/ctest.rc") ($out/ctest.log)"
exit $status
