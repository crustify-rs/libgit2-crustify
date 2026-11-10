# api.c's schedule seed of `deps/llhttp/llhttp.h` still named the enum `llhttp_finish`

## What the earlier plan decided

Run 4 corrected the enum `llhttp_finish` out of api.c's file entry of `deps/llhttp/llhttp.h`
(`crustify/reviews/planner/modules/llhttp-finish-claimed-twice/`): `modules.json` and the
configuration's `modules` record of `libgit2/deps/llhttp/api.c` now list only
`llhttp_settings_s` there. The configuration's `schedule_seeds`, which are meant to be the
files of every module, kept the entry as it was before that correction:

```json
{"path": "deps/llhttp/llhttp.h",
 "items": {"included": [{"name": "llhttp_finish", "kind": "type"},
                        {"name": "llhttp_settings_s", "kind": "type"}]}}
```

## Evidence

```sh
cd /target/libgit2-bindings
git show 1aa1ee98e:crustify/wavefront/configs/wavefront-config.json | python3 -c '
import json, sys; c = json.load(sys.stdin)
print([f for f in c["schedule_seeds"] if isinstance(f, dict) and f["path"] == "deps/llhttp/llhttp.h"])
print([f for m in c["modules"] for f in m["files"] if isinstance(f, dict) and f["path"] == "deps/llhttp/llhttp.h"])'
```

The first line prints the two seed entries of the header, the second the two module
entries: llhttp.c's `excluded` entry is identical in both, api.c's seed names
`llhttp_finish` and its module entry does not.

## Edit and what it moves

`schedule_seeds` are written from the modules' file entries again
(`crustify/artifacts/planner/run-8/compose/config8.py`), each module's entries in module
order, identical entries once. api.c's seed of the header now names `llhttp_settings_s`
alone. Nothing moves: llhttp.c's own seed selects every entity of the header but the three
it excludes, so `llhttp_finish` was seeded, and scheduled in llhttp.c, either way.

## Questions

None.
