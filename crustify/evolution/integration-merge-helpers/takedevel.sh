#!/bin/bash
# takedevel.sh <wave>: merge crustify/devel into the unchecked-out wave branch, under its landing lock.
# A conflict in the dashboard alone is settled by dashmerge.py (this wave's the later landing).
set -euo pipefail
W=$1; REPO=/target/libgit2-crustify; A=$REPO/crustify/tmp/assistant; cd $REPO
G=(git -c user.name='Marius Momeu' -c user.email='marius.momeu@berkeley.edu')
LOCK="$(git rev-parse --git-common-dir)/crustify/landing/crustify/waves/wave-$W.lock"; mkdir -p "$(dirname "$LOCK")"
exec 9>"$LOCK"; flock -w 600 9
WT=$A/wt-wave-$W; git -C $REPO/c worktree remove --force "$WT/c" 2>/dev/null || true; git worktree remove --force "$WT" 2>/dev/null || true; rm -rf "$WT"; git worktree prune
git worktree add -q --detach "$WT" refs/heads/crustify/waves/wave-$W
cd "$WT"; D=crustify/artifacts/dashboard.md; NOTE="the merge conflicted nowhere"
if ! "${G[@]}" merge --no-ff --no-commit crustify/devel >/tmp/takedevel-$W.log 2>&1; then
  for f in $(git diff --name-only --diff-filter=U); do
    if [ "$f" = "$D" ]; then
      git show :1:$D > /tmp/d$W.base; git show :2:$D > /tmp/d$W.ours; git show :3:$D > /tmp/d$W.theirs
      python3 $A/dashmerge.py /tmp/d$W.ours /tmp/d$W.base /tmp/d$W.theirs ours || { echo "dashboard unsettled"; exit 3; }
      cp /tmp/d$W.ours $D; git add $D
    elif [[ "$f" == crustify/wavefront/ownership-store.json ]]; then
      git show :1:$f > /tmp/os$W.base; git show :2:$f > /tmp/os$W.ours; git show :3:$f > /tmp/os$W.theirs
      python3 $A/storemerge.py /tmp/os$W.ours /tmp/os$W.base /tmp/os$W.theirs || { echo "ownership store unsettled"; exit 3; }
      cp /tmp/os$W.ours $f; git add $f
    elif [[ "$f" == sys/*/allowlist.txt || "$f" == sys/*/external.txt ]]; then python3 $A/keepboth.py "$f"; git add "$f"
    elif [[ "$f" == sys/*/bindings.rs ]]; then git checkout -q --ours "$f"; git add "$f"; REGEN=1
    elif [[ "$f" == *.rs ]] && python3 $A/modmerge.py "$f"; then git add "$f"
    else echo "CONFLICT not additive: $f"; exit 3; fi
  done
  NOTE="the merge conflicted additively, in the dashboard, module declarations
or the raw bindings' allowlists: every row, declaration and entry both
sides added is kept, the Σ rows and the Overview's counts are ours plus
theirs less the merge base's, and the coverage percentages are this wave's"
fi
if git diff --cached --quiet; then echo "nothing to merge"; else
CSHA=$(git ls-files -s c | cut -d" " -f2); GEN=builds.gen1; [ "${CSHA:0:9}" = 0551dfd4a ] && GEN=builds
ln -s $REPO/crustify/$GEN crustify/builds 2>/dev/null || true
git -C $REPO/c worktree add -q --detach "$PWD/c" $CSHA
if [ -n "${REGEN:-}" ] || ! CARGO_BUILD_JOBS=24 cargo check -q --all-targets >/tmp/takedevel-$W.check 2>&1; then
  CRUSTIFY_BINDINGS_UPDATE=1 CARGO_BUILD_JOBS=24 cargo check -q --all-targets >/tmp/takedevel-$W.check 2>&1 || true
  CARGO_BUILD_JOBS=24 cargo check -q --all-targets >/tmp/takedevel-$W.check 2>&1 || { echo "CHECK FAILED"; tail -20 /tmp/takedevel-$W.check; exit 4; }
  git add sys/*/bindings.rs; NOTE="$NOTE; the raw bindings are regenerated from the merged allowlists"
fi
[ -L crustify/builds ] && rm -f crustify/builds
git -C $REPO/c worktree remove --force "$PWD/c"; git checkout -q -- c 2>/dev/null || true
"${G[@]}" commit -q -m "crustify: wave $W takes the working branch

${WHY:-For what has been integrated since it forked}; $NOTE.
The workspace checks on the merged tree.

Co-Authored-By: crustify <crustify@berkeley.edu>"
git push -q . HEAD:refs/heads/crustify/waves/wave-$W; fi
cd $REPO; git worktree remove --force "$WT"; git log --oneline -1 refs/heads/crustify/waves/wave-$W
