# `llhttp_execute`'s export moves an empty run's error position off `data`

- **finding**: `equiv/execute-export-moves-an-empty-runs-error-pos`
- **batch**: `wave-194/batch-7/run-1`, `deps/llhttp/api.c`'s module pass
- **items**: `fn::llhttp_execute`'s export under `ffi-export-deps-llhttp-api-c`, which serves
  `transports/httpparser.c`'s `git_http_parser_execute` once api.c switches from C
- **affected commit**: `9124cc5a6` (the export since `2a51ccb3f`)
- **files**: `src/libgit2/deps/llhttp/api.rs`

## Finding

api.c's `llhttp_execute(parser, data, len)` is `llhttp__internal_execute(parser, data,
data + len)`. The generated parser restarts a span the previous run left open at `data`, hands
the span's callback the run from there to `data + len`, and, when the callback refuses it,
stores `data + len` as the parser's `error_pos` (`llhttp.c`, `llhttp__internal_execute`).
`llhttp.h` documents that position as "relative to the `data` argument of
`llhttp_execute()`", and `git_http_parser_execute` computes its parsed length as
`llhttp_get_error_pos(parser) - data` (`c/src/libgit2/transports/httpparser.c:118`).

The export read a run of 0 bytes as `&[]` whatever `data` was:

```rust
let bytes: &[u8] = if len == 0 { &[] } else { unsafe { slice::from_raw_parts(data.cast::<u8>(), len) } };
```

An empty slice's address is the dangling `0x1`, so `parse` handed C the range `0x1..0x1`. When a
run of 0 bytes ends a header's name the previous run left open, and the name's callback refuses
it, the export leaves `error_pos` at `0x1`, where C leaves it at `data`. `error_pos - data` is
then no offset into the run. Its other doc line claimed C's `data + len` allows a NULL `data`,
which C does not: `NULL + 0` is undefined in C17.

libgit2's C does not reach this today: `git_http_parser_execute` calls `llhttp_finish`, not
`llhttp_execute`, for a run of 0 bytes. The export still keeps C's contract for every caller,
and the module's equivalence tests compared the safe `execute`, which takes a slice, never the
export.

## Reproducer

`Cargo.toml` and `src/main.rs` beside this report. It runs two parsers alike, one through the
export and one through api.c's body, `llhttp__internal_execute(parser, data, data + len)`, and
compares their returns and error positions. It links the export in place of api.c's C, so it
needs a build whose archive lacks `api.c.o`: from the repository root, each entry of
`crustify/builds/plain` symlinked into `crustify/tmp/linkcheck/plain` but
`crustify/libcrustify.a`, which is copied and then `ar d crustify/libcrustify.a api.c.o`
(211 objects). From this directory:

```
W=$(git rev-parse --show-toplevel)
side=$(realpath -m "$W/crustify/builds/plain/../../../..")
CRUSTIFY_BUILD_VARIANT=plain CRUSTIFY_BUILD_DIR="$W/crustify/tmp/linkcheck/plain" \
PKG_CONFIG_PATH="$side/pcre2-bindings/crustify/builds/plain/install/lib/pkgconfig" \
cargo run -q
```

On `9124cc5a6` it exits 101 (`run-affected.log`):

```text
llhttp_execute: 7, api.c's body: 7
error_pos: export 0x1, api.c's body 0x7fffefa54091, data 0x7fffefa54091
git_http_parser_execute's parsed_len: export 18446603336495578992, api.c's body 0
thread 'main' panicked at src/main.rs:81:5:
assertion `left == right` failed: the export's error position is not relative to `data`
  left: 0x1
 right: 0x7fffefa54091
```

## Fix

In this batch's second commit, the export reads a non-NULL `data` as the `len` bytes there,
an empty run included, which `slice::from_raw_parts` allows for any non-NULL `u8` pointer.
`parse` then hands C the range `data..data + len`, as api.c does. A NULL `data`, which C
cannot take, stays an empty run, and the doc says so. The reproducer then exits 0, both error
positions at `data`. The regression test is the gateway's `unit_priv`
`llhttp_execute_keeps_an_empty_runs_error_pos_at_data`, which compiles with the feature and
runs with the exports linked in place of `api.c.o`.
