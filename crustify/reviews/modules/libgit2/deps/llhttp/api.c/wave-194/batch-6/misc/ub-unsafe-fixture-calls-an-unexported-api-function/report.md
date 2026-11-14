# A `ub_unsafe` fixture calls an api.c function that has no export, unlinking the suite at the switch

- **finding**: `misc/ub-unsafe-fixture-calls-an-unexported-api-function`
- **batch**: `wave-194/batch-6/run-1`, the `symbol` review of `deps/llhttp/api.c`'s folded
  waves
- **items**: `fn::llhttp_set_error_reason`, ported by `wave-194/batch-3` (`2a51ccb3f`) as
  `ParserMut::set_reason` and not exported, since no C outside api.c calls it; the test
  `ub_unsafe`'s `deps::llhttp::llhttp::set_settings_ptr::a_reason_its_callbacks_set_is_read_while_the_parser_holds_it`
  (`wave-194/batch-5`, `f58d38aa9`)
- **affected commit**: `b3d6b9b37`
- **files**: `tests/ub_unsafe/deps/llhttp/llhttp.rs`

## Finding

Once api.c switches from C, `api.c.o` leaves the variants' archives and the gateway's 31
exports are all that define api.c's symbols. Every suite but `equiv`, whose comparisons call
api.c's C by design and retire with it, must then link against the exports alone.
`wave-194/batch-4` checked that with private builds whose archive copy lacked `api.c.o`, and
moved two fixtures off `llhttp_settings_init`, which has no export. Its link check ran before
its rebase onto `f58d38aa9`, which added batch 5's regression test, so it did not see that
test's fixture `refuse_url`, a C-style `on_url` that stores its reason through
`sys::llhttp_set_error_reason`. With the archive lacking `api.c.o` and the export feature on,
`ub_unsafe` does not link:

```text
rust-lld: error: undefined symbol: llhttp_set_error_reason
>>> referenced by llhttp.rs:292 (tests/ub_unsafe/deps/llhttp/llhttp.rs:292)
>>>               …/ub_unsafe-a7f69842e392650b…rcgu.o:(ub_unsafe::deps::llhttp::llhttp::set_settings_ptr::refuse_url)
>>> did you mean: llhttp_get_error_reason
error: could not compile `libgit2-crustify` (test "ub_unsafe") due to 1 previous error
```

So api.c's switch, whose gates link every suite against the exports, would fail on it. No
other suite but `equiv` calls an api.c function without an export: the remaining `sys::`
calls outside `equiv` are `llhttp_init` and `llhttp_execute`, both exported, and
`llhttp__internal_init` and `llhttp__internal_execute`, llhttp.c's.

## Reproducer

Not a UB, equivalence or logic finding, so the evidence is the link check itself. On
`b3d6b9b37`, from the repository root, with a scratch copy of the `plain` variant's build
whose archive lacks `api.c.o` (each entry of `crustify/builds/plain` linked into
`crustify/tmp/linkcheck/plain` but `crustify/libcrustify.a`, copied and then
`ar d crustify/libcrustify.a api.c.o`, 211 objects):

```
CRUSTIFY_BUILD_DIR=$PWD/crustify/tmp/linkcheck/plain \
CARGO_TARGET_DIR=$PWD/crustify/tmp/target-linkcheck-plain \
crustify/scripts/cargo-variant.sh plain test --features ffi-export-deps-llhttp-api-c \
    --lib --test ub_safe --test ub_unsafe --test unit_pub --test hidden_symbols
```

It fails with the error above.

## Fix

In this batch's second commit, `refuse_url` stores the reason as `llhttp_set_error_reason`'s
body does, `(*parser).reason = reason`, a C callback's own write to the field, which keeps the
test's scenario (a C callback setting a reason its context owns) and needs no api.c symbol.
The link check above then passes on `plain` and on `asan-ubsan`, every suite but `equiv`.
