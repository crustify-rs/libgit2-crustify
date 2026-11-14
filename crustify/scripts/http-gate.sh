#!/bin/bash
# Drive libgit2's HTTP client through a variant's own C, over a smart-HTTP remote on localhost
# (crustify's testing-gates skill, Regressions: a module's switch, for a module whose exports
# the HTTP client reaches, which no offline C test does; crustify/evolution/module-switch-procedure).
#
#   http-gate.sh <variant dir> <out dir>
#
# <variant dir> is a built variant, crustify/builds.gen<n>/<variant>, with its `git2` and
# `libgit2_tests`; nothing is written into it. Two remotes are started from
# crustify/scripts/http-remote.py over scratch repositories under <out dir>/remotes, one open
# and one asking for Basic credentials (foo:bar), each serving every framing it offers
# (Content-Length, chunked, chunked with extensions, close-delimited, malformed):
#
# 1. the variant's `git2 clone` of testrepo.git (c/tests/resources) and of a three-commit
#    repository of the generation's c/ tree at a fixed commit, through the Content-Length and
#    chunked framings, and clones that fail: through the close-delimited framing, which
#    libgit2 does not read to its end, through a response llhttp refuses, of a missing
#    repository (404), and of the guarded remote without credentials (401); each clone's exit
#    status, refs, HEAD, checked-out tree and `git fsck --connectivity-only` outcome, or its
#    error;
# 2. the variant's `libgit2_tests -sonline::push -sonline::clone::credentials`, libgit2's own
#    C tests of a push and a clone over GITTEST_REMOTE_URL, against the guarded remote with
#    foo:bar, through the Content-Length and chunked framings: its TAP tally.
#
# 3. the requests each remote answered, by method, framing and status, and the connections the
#    client opened to it, by framing.
#
# <out dir>/results.txt is one line per case, free of times, ports and paths, so that two
# generations' results compare with `diff`; every process's output is kept beside it, and each
# line counts the sanitizer reports that process printed (`reports=`). The exit status is 0
# when every clone that should succeed did and every suite passed, 1 otherwise.
set -uo pipefail
usage='usage: http-gate.sh <variant dir> <out dir>'
d=$(cd -P "${1:?$usage}" && pwd) || exit 2
out=${2:?$usage}
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
c=$(cd -P "$d/../c" && pwd) || { echo "http-gate.sh: no generation c/ beside $d" >&2; exit 2; }
[ -x "$d/git2" ] && [ -x "$d/libgit2_tests" ] || { echo "http-gate.sh: $d has no git2 or libgit2_tests" >&2; exit 2; }
rm -rf "$out"
mkdir -p "$out"
out=$(cd -P "$out" && pwd)
export LLVM_PROFILE_FILE=${LLVM_PROFILE_FILE:-/dev/null}
export GIT_AUTHOR_DATE='2026-01-01T00:00:00Z' GIT_COMMITTER_DATE='2026-01-01T00:00:00Z'
# big.git's tree: the campaign's source revision, which every generation's c/ holds.
big_tip=0551dfd4a
framings=(length chunked chunked-ext)
status=0
results=$out/results.txt
: >"$results"
reports() { grep -cE 'WARNING: ThreadSanitizer|ERROR: AddressSanitizer|ERROR: LeakSanitizer|runtime error:' "$@" 2>/dev/null | awk -F: '{s+=$NF} END {print s+0}'; }

# The remotes' repositories.
r=$out/remotes
mkdir -p "$r/open" "$r/guarded"
git clone -q --bare "$c/tests/resources/testrepo.git" "$r/open/testrepo.git"
# big.git: the source revision's tree as three commits, the second removing src/ and the
# third restoring it, each with a fixed author and date, so that its ids are every run's.
mkdir -p "$r/src"
git -C "$c" archive "$big_tip" | tar -x -C "$r/src"
(
  cd "$r/src" && git init -q -b main . &&
    export GIT_AUTHOR_NAME=crustify GIT_AUTHOR_EMAIL=crustify@berkeley.edu \
      GIT_COMMITTER_NAME=crustify GIT_COMMITTER_EMAIL=crustify@berkeley.edu &&
    git add -A && git commit -q -m tree &&
    git rm -q -r src && git commit -q -m 'without src' &&
    git revert --no-edit HEAD >/dev/null
)
git clone -q --bare "$r/src" "$r/open/big.git"
rm -rf "$r/src"
for f in "${framings[@]}"; do git init -q --bare "$r/guarded/push-$f.git"; done
git clone -q --bare "$c/tests/resources/testrepo.git" "$r/guarded/testrepo.git"

pids=()
trap 'kill "${pids[@]}" 2>/dev/null' EXIT
python3 -I "$here/http-remote.py" "$r/open" "$out/open.port" --log "$out/open.log" 2>"$out/open.err" &
pids+=($!)
python3 -I "$here/http-remote.py" "$r/guarded" "$out/guarded.port" --auth foo:bar --log "$out/guarded.log" 2>"$out/guarded.err" &
pids+=($!)
for _ in $(seq 100); do [ -s "$out/open.port" ] && [ -s "$out/guarded.port" ] && break; sleep 0.1; done
open=http://127.0.0.1:$(cat "$out/open.port")
guarded=http://127.0.0.1:$(cat "$out/guarded.port")

# 1. The CLI's clones. <expect> is ok or fail.
clone() {
  local name=$1 url=$2 expect=$3 dir=$out/clones/$1
  mkdir -p "$out/clones"
  (cd "$out/clones" && timeout 600 "$d/git2" clone "$url" "$dir" >"$dir.out" 2>"$dir.err" </dev/null)
  local rc=$?
  local line="clone $name rc=$rc"
  if [ $rc -eq 0 ] && [ -d "$dir/.git" ]; then
    local refs head tree fsck
    refs=$(git -C "$dir" for-each-ref --format='%(objectname) %(refname)' | sha256sum | cut -c1-16)
    head=$(git -C "$dir" rev-parse HEAD 2>/dev/null)
    tree=$(git -C "$dir" status --porcelain --untracked-files=all | sha256sum | cut -c1-16)
    git -C "$dir" fsck --connectivity-only --no-progress >"$dir.fsck" 2>&1 && fsck=ok || fsck=failed
    line+=" refs=$refs head=$head status=$tree fsck=$fsck"
  else
    line+=" error=\"$(grep -v '^\s*$' "$dir.err" | grep -vE 'Sanitizer|^ +#|^SUMMARY|^==' | tail -1)\""
  fi
  line+=" reports=$(reports "$dir.err")"
  echo "$line" >>"$results"
  case $expect in
    ok) [ $rc -eq 0 ] && [[ $line == *fsck=ok* ]] || status=1 ;;
    fail) [ $rc -ne 0 ] || status=1 ;;
  esac
}
for f in "${framings[@]}"; do
  clone "testrepo-$f" "$open/$f/testrepo.git" ok
  clone "big-$f" "$open/$f/big.git" ok
done
clone testrepo-close "$open/close/testrepo.git" fail
clone testrepo-malformed "$open/malformed/testrepo.git" fail
clone missing "$open/length/missing.git" fail
clone unauthorised "$guarded/length/testrepo.git" fail

# 2. libgit2's own online tests of a push and a clone.
for f in "${framings[@]}"; do
  s=$out/suites/$f
  mkdir -p "$s/tmp"
  (cd "$s" && TMPDIR=$s/tmp GITTEST_REMOTE_URL="$guarded/$f/push-$f.git" GITTEST_REMOTE_USER=foo \
    GITTEST_REMOTE_PASS=bar timeout 1800 "$d/libgit2_tests" -t -sonline::push \
    -sonline::clone::credentials >"$s/tap" 2>"$s/err" </dev/null)
  rc=$?
  ok=$(grep -c '^ok ' "$s/tap") notok=$(grep -c '^not ok ' "$s/tap") skip=$(grep -c '# SKIP' "$s/tap")
  echo "suites $f rc=$rc ok=$ok not_ok=$notok skip=$skip reports=$(reports "$s/err" "$s/tap")" >>"$results"
  [ $rc -eq 0 ] && [ "$notok" -eq 0 ] && [ "$ok" -gt 0 ] || status=1
done

# The requests the remotes answered, by method, framing and status; the connections a client
# reset or closed mid-request (`error` lines) depend on timing, and stay in the logs alone.
# Then the connections the client opened, by framing: a request that is the first on its
# connection opened it, so that a client that stops keeping connections alive, as
# `llhttp_should_keep_alive` tells libgit2's to, opens more.
for side in open guarded; do
  grep -v '^error ' "$out/$side.log" | awk -v s="$side" '{split($2, p, "/"); print "requests " s " " $1 " " p[2] " " $3}' |
    sort | uniq -c | awk '{print $2, $3, $4, $5, $6, "count=" $1}' >>"$results"
done
for side in open guarded; do
  grep -v '^error ' "$out/$side.log" | awk -v s="$side" '$4 == "conn=1" {split($2, p, "/"); print "connections " s " " p[2]}' |
    sort | uniq -c | awk '{print $2, $3, $4, "count=" $1}' >>"$results"
done
cat "$results"
exit $status
