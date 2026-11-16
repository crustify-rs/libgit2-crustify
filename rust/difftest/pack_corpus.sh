#!/usr/bin/env bash
# Build the corpus of pack_diff.c: packs and indexes of every kind the pack
# reader parses, made with git from the test fixtures and a synthetic repo.
#   usage: pack_corpus.sh <outdir>     (writes <outdir>/<set>.{pack,idx,type})
# Sets: t2 (testrepo, index v2), t1 (index v1), t2big (v2 with every offset
# above 0x100 in the 64-bit table), s2 (SHA-256, v2), chain_ofs / chain_ref
# (delta chains up to ~150 deep, OFS_DELTA / REF_DELTA bases).
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RES="$HERE/../../tests/resources"
out=$1
rm -rf "$out"; mkdir -p "$out"
export GIT_CONFIG_NOSYSTEM=1 HOME="$out/home" GIT_AUTHOR_DATE="2020-01-01T00:00:00Z" GIT_COMMITTER_DATE="2020-01-01T00:00:00Z"
mkdir -p "$HOME"

set1() { # name pack idx type
	cp "$2" "$out/$1.pack"; cp "$3" "$out/$1.idx"; echo "$4" > "$out/$1.type"; chmod 644 "$out/$1".*
}

p=$(ls "$RES"/testrepo.git/objects/pack/pack-a81e*.pack)
set1 t2 "$p" "${p%.pack}.idx" 1
cp "$p" "$out/tmp.pack"
git index-pack --index-version=1 -o "$out/tmp1.idx" "$out/tmp.pack" >/dev/null
set1 t1 "$p" "$out/tmp1.idx" 1
git index-pack --index-version=2,0x100 -o "$out/tmp2.idx" "$out/tmp.pack" >/dev/null
set1 t2big "$p" "$out/tmp2.idx" 1
rm -f "$out"/tmp*

p=$(ls "$RES"/testrepo_256.git/objects/pack/pack-e2f07f*.pack)
set1 s2 "$p" "${p%.pack}.idx" 2

# Long delta chains: one file, many small edits, packed deep.
r="$out/chainrepo"
git init -q "$r"
( cd "$r"
  for i in $(seq 1 400); do echo "line $i of the file, which is long enough to delta well"; done > f
  git add f; git -c user.name=a -c user.email=a@b commit -qm c0
  for c in $(seq 1 160); do
	sed -i "$(( (c * 7) % 400 + 1 ))s/.*/edited in commit $c/" f
	git -c user.name=a -c user.email=a@b commit -qam "c$c"
  done
  git -c pack.threads=1 repack -adfq --depth=250 --window=250
  pk=$(ls .git/objects/pack/*.pack)
  cp "$pk" "$out/chain_ofs.pack"; cp "${pk%.pack}.idx" "$out/chain_ofs.idx"
  git -c pack.threads=1 -c repack.useDeltaBaseOffset=false repack -adfq --depth=250 --window=250
  pk=$(ls .git/objects/pack/*.pack)
  cp "$pk" "$out/chain_ref.pack"; cp "${pk%.pack}.idx" "$out/chain_ref.idx"
)
echo 1 > "$out/chain_ofs.type"; echo 1 > "$out/chain_ref.type"
chmod 644 "$out"/chain_*
rm -rf "$r" "$HOME"
ls "$out"/*.type | sed 's|.*/||; s|\.type$||' > "$out/sets.txt"
