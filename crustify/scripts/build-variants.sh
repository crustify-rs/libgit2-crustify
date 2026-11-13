#!/bin/bash
# Build a generation of the target's variants from the C that remains (crustify's setup
# skill, Builds; porting-workflow, Builds): run after a module's switch removes C, and
# whenever the shared variants are rebuilt.
#
#   build-variants.sh <generation dir> [<variant>...]
#
# A generation is a directory beside crustify/builds, as crustify/builds.gen<n>, which a
# checkout's crustify/builds links to once it is built: it holds `c`, a detached worktree
# of this checkout's `c/` repository at the commit `c/` has checked out, which `c/` must
# hold without changes, and one build dir per variant of crustify/artifacts/build.json
# (every one by default), named as its `dir` names it. Each variant's `configure` and
# `build` commands run from its dir, physically resolved, so that the relative paths they
# take (the generation's `../c`, the side campaigns' installs four levels up) resolve as
# for the shared builds, with CRUSTIFY_ROOT set to this checkout, whose scripts the
# commands run and whose Rust the C consumers link. The variants build side by side, each
# logging to <dir>/crustify/build.log; the exit status is 1 when any fails, naming it.
# An existing generation is not rebuilt: a variant whose dir exists is refused.
set -euo pipefail
usage='usage: build-variants.sh <generation dir> [<variant>...]'
gen=${1:?$usage}
shift
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
spec=$root/crustify/artifacts/build.json
mapfile -t all < <(python3 -I -c 'import json,sys; print("\n".join(json.load(open(sys.argv[1]))["variants"]))' "$spec")
variants=("$@")
[ ${#variants[@]} -gt 0 ] || variants=("${all[@]}")
for v in "${variants[@]}"; do
  printf '%s\n' "${all[@]}" | grep -qx -- "$v" || { echo "build-variants.sh: no variant $v in $spec" >&2; exit 2; }
done

mkdir -p "$gen"
gen=$(cd -P "$gen" && pwd)
commit=$(git -C "$root/c" rev-parse HEAD)
if [ ! -e "$gen/c" ]; then
  [ -z "$(git -C "$root/c" status --porcelain --untracked-files=no)" ] || {
    echo "build-variants.sh: $root/c has changes: a generation is built from a commit" >&2
    exit 1
  }
  git -C "$root/c" worktree add --detach "$gen/c" "$commit" >/dev/null
fi
held=$(git -C "$gen/c" rev-parse HEAD)
[ "$held" = "$commit" ] || {
  echo "build-variants.sh: $gen/c holds $held, not $root/c's $commit" >&2
  exit 1
}

field() { python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["variants"][sys.argv[2]][sys.argv[3]])' "$spec" "$1" "$2"; }

pids=() names=()
for v in "${variants[@]}"; do
  dir=$gen/$(basename "$(field "$v" dir)")
  [ ! -e "$dir" ] || { echo "build-variants.sh: $dir exists: a generation is built once" >&2; exit 1; }
  mkdir -p "$dir/crustify"
  configure=$(field "$v" configure)
  build=$(field "$v" build)
  (
    cd -P "$dir"
    export CRUSTIFY_ROOT=$root
    {
      echo "+ $configure"
      eval "$configure"
      echo "+ $build"
      eval "$build"
    } >"$dir/crustify/build.log" 2>&1
  ) &
  pids+=($!) names+=("$v")
done
failed=()
for i in "${!pids[@]}"; do
  wait "${pids[$i]}" || failed+=("${names[$i]}")
done
if [ ${#failed[@]} -gt 0 ]; then
  echo "build-variants.sh: ${failed[*]} failed: see each <dir>/crustify/build.log under $gen" >&2
  exit 1
fi
echo "build-variants.sh: $gen holds ${variants[*]}, from c/ at $commit"
