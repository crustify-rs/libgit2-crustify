#!/bin/bash
# Archive the objects a shared library links into one static archive, so that campaign mode
# links the C statically and reaches the internal functions the shared library hides
# (crustify's setup skill, Builds; bindgen, Build modes).
#
#   static-archive.sh <library> <archive>
#
# Run from a variant's build dir, as the last step of its `build` command, once the build is
# current: <library> names the shared library CMake links (libgit2, for libgit2.so), and
# <archive> is the archive written, relative to the build dir, as the variant's `static`
# records it in crustify/artifacts/build.json. The objects are taken from the build system,
# the explicit inputs of the edge that links <library>.so, found through its symlink edges:
# every TU of the library itself and of the object libraries it bundles, and nothing else
# (no test, CLI or other target). Nothing is compiled; the archive is replaced atomically and
# is deterministic (no timestamps), so the same objects give the same bytes.
set -euo pipefail
lib=${1:?usage: static-archive.sh <library> <archive>}
archive=${2:?usage: static-archive.sh <library> <archive>}
[ -f build.ninja ] || { echo "static-archive.sh: no build.ninja in $PWD" >&2; exit 2; }

# `ninja -t query` prints a target's rule on its `input:` line, then its explicit inputs
# indented by four spaces, its implicit ones after `| ` and its order-only ones after `|| `.
target=$lib.so
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
  *SHARED_LIBRARY_LINKER*) ;;
  *) echo "static-archive.sh: $target is built by $rule, not a shared library link" >&2; exit 1 ;;
esac

mapfile -t objects < <(awk '/^  outputs:/ {exit} /^    [^|]/ {print $1}' <<<"$query")
[ ${#objects[@]} -gt 0 ] || { echo "static-archive.sh: $target links no objects" >&2; exit 1; }
for o in "${objects[@]}"; do
  case $o in
    *.o) [ -f "$o" ] || { echo "static-archive.sh: $o is missing: build first" >&2; exit 1; } ;;
    *) echo "static-archive.sh: $target links $o, which is not an object" >&2; exit 1 ;;
  esac
done

mkdir -p "$(dirname "$archive")"
tmp=$archive.tmp.$$
rm -f "$tmp"
trap 'rm -f "$tmp"' EXIT
ar qcD "$tmp" "${objects[@]}"
ranlib -D "$tmp"
mv -f "$tmp" "$archive"
trap - EXIT
echo "static-archive.sh: $archive holds the ${#objects[@]} objects $target links"
