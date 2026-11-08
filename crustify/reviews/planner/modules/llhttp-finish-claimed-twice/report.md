# `llhttp_finish` in deps/llhttp/llhttp.h was claimed by two modules

## What the earlier plan decided

`modules.json`: deps/llhttp/llhttp.c owns `deps/llhttp/llhttp.h` with an `excluded` list
of the functions api.c and http.c define and of `llhttp_settings_s`, which does not name
`llhttp_finish`; deps/llhttp/api.c lists the same header with

```json
"included": ["llhttp_errno_name", "llhttp_execute", "llhttp_finish", "llhttp_get_errno", ...]
```

and the wavefront configuration carried it as `{"name": "llhttp_finish", "kind": "type"}`
in api.c's `deps/llhttp/llhttp.h` entry.

## Evidence

`llhttp_finish` names two entities of the header: the enum `enum llhttp_finish`
(`c/deps/llhttp/llhttp.h:131`, typedef `llhttp_finish_t`) and the function
`llhttp_errno_t llhttp_finish(llhttp_t* parser)` (line 679), defined in deps/llhttp/api.c.
A file entry's names select every entity of that name, so the enum was owned by llhttp.c
(not excluded) and by api.c (included). It went unnoticed while enums were no units; a port
campaign schedules enums, and wavefront refuses the configuration:

```sh
cd /target/libgit2-bindings
git show HEAD:crustify/wavefront/configs/wavefront-config.json > /tmp/cfg.json   # the earlier configuration
wavefront /target/libgit2-bindings/c --config /tmp/cfg.json schedule --with-enums --output /tmp/s.json
# schedule: 1 unit(s) belong to several modules:
#   - type llhttp_finish (deps/llhttp/llhttp.h): libgit2/deps/llhttp/llhttp.c, libgit2/deps/llhttp/api.c
```

The function needs no header entry: api.c's translation unit is its home. The enum belongs
with the header's other enums, which llhttp.c owns (`llhttp_errno`, `llhttp_flags`,
`llhttp_method`, ...); `parser->finish` is set by llhttp.c's state machine and read by
api.c's `llhttp_finish`.

## Edit

api.c's `deps/llhttp/llhttp.h` entry no longer names `llhttp_finish` in `modules.json`, and
its configuration entry drops the `type` item: the enum is llhttp.c's, the function stays
api.c's through its TU. Downstream: the enum `llhttp_finish` is a `port` item of
deps/llhttp/llhttp.c's waves; nothing else moves.
