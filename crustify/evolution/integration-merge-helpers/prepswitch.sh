#!/bin/bash
# prepswitch.sh <module dir> <wave> <body file>: promote the wave to the module branch, merge devel into
# it in the module's integration worktree (additive conflicts settled by the helpers), run fmt, clippy
# and the plain tests, and commit the merge as `integrate <module>`. The switch follows on the branch.
set -euo pipefail
MDIR=$1; W=$2; BODY=$3; REPO=/target/libgit2-crustify; A=$REPO/crustify/tmp/assistant
G=(git -c user.name='Marius Momeu' -c user.email='marius.momeu@berkeley.edu'); cd $REPO
MB=crustify/$MDIR; MOD=${MDIR#modules/}
git push -q . refs/heads/crustify/waves/wave-$W:refs/heads/$MB
python3 - "$W" <<'PY'
import json,sys
sys.path.insert(0,'/target/libgit2-crustify/crustify/tmp/assistant'); import schedenc
p='/target/libgit2-crustify/crustify/artifacts/schedule.json'; d=json.load(open(p)); first=int(sys.argv[1])
for w in d['waves']:
    if w['index']==first or w.get('packed_with')==first: w['status']='done'
open(p,'w').write(schedenc.enc(d)+'\n')
PY
git add crustify/artifacts/schedule.json
git diff --cached --quiet || "${G[@]}" commit -q -m "progress: wave-$W

Its module pass landed and the wave is promoted to its module's branch.

Co-Authored-By: crustify <crustify@berkeley.edu>"
find crustify/artifacts/$MDIR \( -name task.json -o -name task.md \) | while read f; do git ls-files --error-unmatch "$f" >/dev/null 2>&1 || rm -f "$f"; done
WT=crustify/.worktrees/$MDIR/integration
git worktree add -q "$WT" "$MB"; cd "$WT"; mkdir -p crustify/tmp
D=crustify/artifacts/dashboard.md; NOTE="The merge conflicted nowhere."
if ! "${G[@]}" merge --no-ff --no-commit crustify/devel >crustify/tmp/merge.log 2>&1; then
  for f in $(git diff --name-only --diff-filter=U); do
    if [ "$f" = "$D" ]; then
      git show :1:$D > /tmp/pd.base; git show :2:$D > /tmp/pd.ours; git show :3:$D > /tmp/pd.theirs
      python3 $A/dashmerge.py /tmp/pd.ours /tmp/pd.base /tmp/pd.theirs ours || { echo "dashboard unsettled"; exit 3; }
      cp /tmp/pd.ours $D; git add $D
    elif [[ "$f" == crustify/wavefront/ownership-store.json ]]; then
      git show :1:$f > /tmp/ps.base; git show :2:$f > /tmp/ps.ours; git show :3:$f > /tmp/ps.theirs
      python3 $A/storemerge.py /tmp/ps.ours /tmp/ps.base /tmp/ps.theirs || { echo "store unsettled"; exit 3; }
      cp /tmp/ps.ours $f; git add $f
    elif [[ "$f" == Cargo.toml ]] && python3 $A/cargomerge.py "$f"; then git add "$f"
    elif [[ "$f" == sys/*/allowlist.txt || "$f" == sys/*/external.txt ]]; then python3 $A/keepboth.py "$f"; git add "$f"
    elif [[ "$f" == sys/*/bindings.rs ]]; then git checkout -q --ours "$f"; git add "$f"; REGEN=1
    elif [[ "$f" == *.rs ]] && python3 $A/modmerge.py "$f"; then git add "$f"
    else echo "CONFLICT not additive: $f"; exit 3; fi
  done
  NOTE="The merge conflicted additively only: every dashboard row, module declaration, allowlist
entry and ownership record both sides added is kept, the Σ rows and the Overview's counts
are ours plus theirs less the merge base's, and the coverage percentages are this module's."
fi
CSHA=$(git ls-files -s c | cut -d" " -f2); GEN=${GEN:-builds}
ln -s $REPO/crustify/$GEN crustify/builds; git -C $REPO/c worktree add -q --detach "$PWD/c" $CSHA
export CARGO_BUILD_JOBS=${JOBS:-32} RUST_TEST_THREADS=${JOBS:-32}
if [ -n "${REGEN:-}" ] || ! cargo check -q --all-targets >crustify/tmp/check.log 2>&1; then
  CRUSTIFY_BINDINGS_UPDATE=1 cargo check -q --all-targets >crustify/tmp/check.log 2>&1 || true
  cargo check -q --all-targets >crustify/tmp/check.log 2>&1 || { echo "CHECK FAILED"; tail -20 crustify/tmp/check.log; exit 4; }
  git add sys/*/bindings.rs
fi
cargo +nightly-2026-10-08 fmt --all --check >crustify/tmp/fmt.log 2>&1 || { echo "FMT FAILED"; head -20 crustify/tmp/fmt.log; exit 4; }
cargo clippy --all-targets -q -- -D warnings >crustify/tmp/clippy.log 2>&1 || { echo "CLIPPY FAILED"; tail -20 crustify/tmp/clippy.log; exit 4; }
cargo test --no-fail-fast >crustify/tmp/test.log 2>&1 || { echo "TESTS FAILED"; grep -E "FAILED|failed|panicked" crustify/tmp/test.log | head -20; exit 4; }
SUM=$(grep -E "^test result" crustify/tmp/test.log | awk '{p+=$4; f+=$6; i+=$8} END {print p" passed, "f" failed, "i" ignored"}')
{ echo "integrate $MOD"; echo; cat "$BODY"; echo "$NOTE"; echo "Format, clippy with warnings denied and the plain workspace's tests ($SUM)"; echo "pass on the merged tree."; echo; echo "Co-Authored-By: crustify <crustify@berkeley.edu>"; } > crustify/tmp/msg
"${G[@]}" commit -q -F crustify/tmp/msg
echo "gates ok: $SUM"; git log --oneline -1
