#!/bin/bash
# Archive the objects every link unit links into one static archive, so that campaign mode
# links the C statically and reaches the internal functions the shared library hides
# (crustify's setup skill, Builds; bindgen, Build modes).
#
#   static-archive.sh <archive> <link unit>...
#
# Run from a variant's build dir, as the last step of its `build` command, once the build is
# current: <archive> is the archive written, relative to the build dir, as the variant's
# `static` records it in crustify/artifacts/build.json, and each <link unit> names a target
# CMake links: a shared library by its name (libgit2, for libgit2.so) or an executable (git2,
# the CLI). The objects are taken from the build system, the explicit inputs of the edge that
# links each, found through its symlink edges: every TU of the link unit itself and of the
# object libraries it bundles, and nothing else (no test or other target). An object two
# link units both link (the CLI links libgit2's) is archived once, where it first appears.
# Nothing is compiled; the archive is replaced atomically and is deterministic (no
# timestamps), so the same objects give the same bytes.
set -euo pipefail
usage='usage: static-archive.sh <archive> <link unit>...'
archive=${1:?$usage}
shift
[ $# -gt 0 ] || { echo "$usage" >&2; exit 2; }
[ -f build.ninja ] || { echo "static-archive.sh: no build.ninja in $PWD" >&2; exit 2; }

# `ninja -t query` prints a target's rule on its `input:` line, then its explicit inputs
# indented by four spaces, its implicit ones after `| ` and its order-only ones after `|| `.
objects=() names=()
declare -A seen=()
for unit in "$@"; do
  target=$unit.so
  ninja -t query "$target" >/dev/null 2>&1 || target=$unit
  for _ in 1 2 3 4; do
    query=$(ninja -t query "$target")
    rule=$(sed -n 's/^  input: //p' <<<"$query")
    case $rule in
      *_LINKER*) break ;;
      '') echo "static-archive.sh: no edge builds $target" >&2; exit 1 ;;
    esac
    # A symlink edge (libgit2.so -> libgit2.so.1.9 -> libgit2.so.1.9.0): follow its input.
    target=$(awk '/^    [^|]/ {print $1; exit}' <<<"$query")
  done
  case $rule in
    *SHARED_LIBRARY_LINKER*|*EXECUTABLE_LINKER*) ;;
    *) echo "static-archive.sh: $target is built by $rule, not a shared library or executable link" >&2; exit 1 ;;
  esac
  mapfile -t inputs < <(awk '/^  outputs:/ {exit} /^    [^|]/ {print $1}' <<<"$query")
  [ ${#inputs[@]} -gt 0 ] || { echo "static-archive.sh: $target links no objects" >&2; exit 1; }
  for o in "${inputs[@]}"; do
    case $o in
      *.o) [ -f "$o" ] || { echo "static-archive.sh: $o is missing: build first" >&2; exit 1; } ;;
      *) echo "static-archive.sh: $target links $o, which is not an object" >&2; exit 1 ;;
    esac
    [ -n "${seen[$o]:-}" ] && continue
    seen[$o]=1
    objects+=("$o")
  done
  names+=("$target")
done

mkdir -p "$(dirname "$archive")"
tmp=$archive.tmp.$$
rm -f "$tmp"
trap 'rm -f "$tmp"' EXIT
ar qcD "$tmp" "${objects[@]}"
ranlib -D "$tmp"
mv -f "$tmp" "$archive"
trap - EXIT
echo "static-archive.sh: $archive holds the ${#objects[@]} objects ${names[*]} link"
