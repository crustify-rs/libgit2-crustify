#!/bin/bash
# Wave and module-end gates over one tree: gates.sh <tree> <logdir> <stream: san|miri-bsan>
#   san:       plain, asan-ubsan and tsan over the whole workspace, one after another
#   miri-bsan: Miri per test binary, then BSan per test binary, each under a timeout
set -u
tree=$1 logs=$2 stream=$3
mkdir -p "$logs"
cd "$tree"
export CARGO_BUILD_JOBS=8 RUST_TEST_THREADS=8 CARGO_HOME=/root/.cargo
nightly=nightly-2026-10-08
v=crustify/scripts/cargo-variant.sh
run() { # name, command...
  local name=$1; shift
  echo "== $(date -u +%T) start $name: $*" >>"$logs/summary.txt"
  "$@" >"$logs/$name.log" 2>&1
  local rc=$?
  echo "== $(date -u +%T) end $name rc=$rc" >>"$logs/summary.txt"
}
case $stream in
  san)
    run plain $v plain test --workspace --no-fail-fast
    run asan-ubsan $v asan-ubsan test --workspace --no-fail-fast
    run tsan $v tsan test --workspace --no-fail-fast
    ;;
  miri-bsan|miri)
    export CARGO_TARGET_DIR=$tree/target/miri
    run miri-build cargo +$nightly miri test -p libgit2 --no-run
    for t in "--lib" "--test ub_safe" "--test ub_unsafe" "--test equiv" "--test unit_pub"; do
      n=miri-${t##* }; n=${n#miri---}
      run "miri-${t##* }" cargo +$nightly miri test -p libgit2 $t -- --skip compile_fail &
    done
    wait
    unset CARGO_TARGET_DIR
    [ "$stream" = miri ] && { echo "== $(date -u +%T) stream miri done" >>"$logs/summary.txt"; exit 0; }
    ;&
  bsan)
    run bsan-build $v bsan test -p libgit2 --no-run
    export RUST_TEST_THREADS=4
    run bsan-lib timeout 3600 $v bsan test -p libgit2 --lib -- \
      --skip indexer::unit_tests::a_break_cancels_the_indexing_with_git_euser \
      --skip indexer::unit_tests::a_panic_cancels_then_resumes_after_the_walk \
      --skip indexer::unit_tests::libgit2_calls_the_closure_with_its_progress &
    run bsan-equiv timeout 3600 $v bsan test -p libgit2 --test equiv -- \
      --skip src::libgit2::indexer::a_progress_callback_sees_and_cancels_as_a_c_callback &
    run bsan-ub_safe timeout 3600 $v bsan test -p libgit2 --test ub_safe &
    run bsan-ub_unsafe timeout 3600 $v bsan test -p libgit2 --test ub_unsafe &
    run bsan-unit_pub timeout 3600 $v bsan test -p libgit2 --test unit_pub &
    wait
    ;;
esac
echo "== $(date -u +%T) stream $stream done" >>"$logs/summary.txt"
