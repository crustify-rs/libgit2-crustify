# `ffimap api` reads a type and a function of one path as one item

Status: `open` (filed by `wave-194/batch-4/run-1`; no change made).

## Signal

A tool that reports what the source contradicts (`evolution` §2): `ffimap api diff` reports a
function as changed into a type alias when a type of the same name is added beside it, in
Rust's other namespace, though the function is still there, unchanged.

## Evidence

`wave-194/batch-4/run-1` allowlisted `type llhttp_finish` (llhttp.h's `enum llhttp_finish`)
beside the function `llhttp_finish` already bound, so bindgen emits both `pub type
llhttp_finish = c_uint;` and `pub fn llhttp_finish(…)` in `sys::libgit2`. `ffimap api diff`
of the tip `f58d38aa9` against the batch's tree (nightly `2026-10-08`, ffimap `9aa2ef1`)
lists, under the unattributed `libgit2_crustify::sys::libgit2`, `{"item":
"libgit2_crustify::sys::libgit2::llhttp_finish", "part": "kind", "before": "function",
"after": "type_alias", "severity": "changed"}`, and so counts the module `changed`, while
the gateway's `[sys::llhttp_finish, llhttp_finish]` check still compiles against the
function. `reproducer/` is a two-version crate that shows it alone: `before` has `pub fn
llhttp_finish()`, `after` adds `pub type llhttp_finish = u32;`; `cargo +nightly-2026-10-08
rustdoc --lib -- -Z unstable-options --output-format json` in each, then `ffimap api diff
before.json after.json --format json`, reports the same `kind` change, where `extended` (one
item added) is the right reading.

## Destination

ffimap: `ffimap api`'s locators (`docs/api.md` §3), which key a free item by its canonical
path alone.

## Change

Not made. A locator of a free item would carry its namespace, the type namespace (types,
traits, modules, type aliases) or the value namespace (functions, constants, statics), so
that `llhttp_finish` the alias and `llhttp_finish` the function are two items; a record whose
path holds one of each is then an addition, not a change of kind. The reproducer becomes the
regression test.

## Validation

None run.

## Review

None yet.
