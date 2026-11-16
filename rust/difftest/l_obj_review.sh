#!/usr/bin/env bash
# The l-obj review drivers (review l-obj-r1, LO-16), adopted: the object API
# (lookup, prefix, peel, short ids, bypath, __is_valid, object ids from
# buffers, files, descriptors and symlinks, SHA-1 and SHA-256), blobs
# (binary detection, creation from buffer/disk/workdir/paths/stream,
# filtering), signatures (parser behind a guard page, new/dup/now/default,
# the environment), tags (parse behind a guard page; create, delete, list,
# foreach with the reference and reflog files compared) and grafts. Each is
# built against the C reference and the Rust build and the outputs compared.
#   usage: RUST_BUILD=/work/builds/l-obj rust/difftest/l_obj_review.sh
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$(cd "$HERE/../.." && pwd)"
D="$HERE/l_obj"; OUT="$HERE/out/l_obj"; mkdir -p "$OUT"
R="$SRC/tests/resources"
fail=0
cmpout() { # name
	if cmp -s "$OUT/$1.c.txt" "$OUT/$1.rust.txt"; then echo "$1: same output ($(wc -l < "$OUT/$1.c.txt") lines)"
	else echo "$1: DIFFERENT"; diff "$OUT/$1.c.txt" "$OUT/$1.rust.txt" | head -20; fail=1; fi
}
for d in objapi_diff blob_diff sig_diff sig_env_diff tagapi_diff tagguard_diff grafts2_diff; do
	"$HERE/link.sh" "$D/$d.c" "$OUT" >/dev/null 2>&1 || { echo "$d: build failed"; fail=1; }
done
for v in c rust; do
	# object API on copies (it sets core.abbrev)
	rm -rf /tmp/lobjrepos; mkdir -p /tmp/lobjrepos
	for r in testrepo.git duplicate.git peeled.git bad_tag.git short_tag.git testrepo_256.git; do cp -r "$R/$r" /tmp/lobjrepos/; done
	(cd "$SRC" && IDLIST="$D/ids256.txt" "$OUT/objapi_diff_$v") 2>&1 | sed 's/lobj_r1_id[A-Za-z0-9]*/lobj_r1_idX/g' > "$OUT/objapi.$v.txt"
	# blobs
	T="$OUT/blob_tmp_$v"; rm -rf "$T"; mkdir -p "$T/home"
	(cd "$SRC" && env HOME="$T/home" XDG_CONFIG_HOME="$T/home" GIT_CONFIG_NOSYSTEM=1 "$OUT/blob_diff_$v" "$T") 2>&1 | sed "s#$T#TMP#g" > "$OUT/blob.$v.txt"
	# signatures
	(cd "$SRC" && TZ=UTC "$OUT/sig_diff_$v") > "$OUT/sig.$v.txt" 2>&1
	T="$OUT/env_tmp"; rm -rf "$T"; mkdir -p "$T/home"
	env -i PATH="$PATH" HOME="$T/home" XDG_CONFIG_HOME="$T/home" GIT_CONFIG_NOSYSTEM=1 "$OUT/sig_env_diff_$v" "$T/repo" > "$OUT/sigenv.$v.txt" 2>&1
	# tags
	S="$OUT/tag_scratch"; rm -rf "$S"; mkdir -p "$S"
	cp -r "$R/testrepo.git" "$S/repo.git"; cp -r "$R/testrepo2" "$S/other"; mv "$S/other/.gitted" "$S/other/.git" 2>/dev/null
	git -C "$S/repo.git" config core.logAllRefUpdates always
	(cd "$S" && "$OUT/tagapi_diff_$v" repo.git other; echo "exit=$?") > "$OUT/tagapi.$v.txt" 2>&1
	(cd "$S/repo.git" && find refs logs -type f | sort | while read -r f; do echo "== $f"; cat "$f"; done; echo "== packed-refs"; cat packed-refs) >> "$OUT/tagapi.$v.txt" 2>&1
	(cd "$SRC" && "$OUT/tagguard_diff_$v") > "$OUT/tagguard.$v.txt" 2>&1
	(cd "$SRC" && "$OUT/grafts2_diff_$v") > "$OUT/grafts2.$v.txt" 2>&1
done
# Known, recorded differences (rust/DEFECTS.md) are normalised out:
# - "oom" cases of sig_diff: C returns 0 with a NULL name or e-mail when a
#   copy fails; the Rust fails with -1.
# - sig_env_diff: times that are "now" (they differ between the runs), and
#   the offset C leaves uninitialized for an approximate GIT_*_DATE.
for v in c rust; do
	grep -v '^oom ' "$OUT/sig.$v.txt" > "$OUT/sig.$v.tmp" && mv "$OUT/sig.$v.tmp" "$OUT/sig.$v.txt"
	sed -E -i 's/t=1[789][0-9]{8} off=-?[0-9]+/t=NOW off=X/g' "$OUT/sigenv.$v.txt"
done
for n in objapi blob sig sigenv tagapi tagguard grafts2; do cmpout $n; done
rm -rf /tmp/lobjrepos
exit $fail
