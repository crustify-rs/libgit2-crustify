# http.c's port landed tests that fail the workspace's lint gate

- **finding**: `misc/lint-gate-fails-on-the-batchs-own-tests`
- **batch**: `wave-197/batch-1/run-1`, `deps/llhttp/http.c`'s module pass
- **items**: the tests of `wave-197/batch-0` (`a81dc3103`), which ported
  `fn::llhttp_message_needs_eof`, `fn::llhttp_should_keep_alive`,
  `fn::llhttp__before_headers_complete`, `fn::llhttp__after_headers_complete` and
  `fn::llhttp__after_message_complete`
- **affected commit**: `a81dc3103`, the tip of `crustify/waves/wave-197`
- **files**: `tests/unit_pub/deps/llhttp/http.rs`, `tests/equiv/deps/llhttp/http.rs`

## Finding

`testing-gates`' Regressions section has every batch lint the workspace on the tree it lands,
with warnings denied, and `toolchain`'s Lints section sets `clippy::pedantic` to `warn` and
`multiple_unsafe_ops_per_block` to `deny`. The tree `wave-197/batch-0` landed fails that gate,
with the export feature off and on, in two of its own test files:

- `tests/unit_pub/deps/llhttp/http.rs`, `after_headers_complete_on_a_zeroed_parser`: the
  function is 102 lines (`clippy::too_many_lines`, pedantic, limit 100), and its cases' array
  is typed by a five-element tuple written inline (`clippy::type_complexity`);
- `tests/equiv/deps/llhttp/http.rs`, `State::parser`: one `unsafe` block both reads the zeroed
  parser (`raw.read()`, in the struct update) and writes it (`*raw = …`), two unsafe
  operations (`clippy::multiple_unsafe_ops_per_block`, denied).

The batch's notes record that "`clippy --workspace --all-targets` passes with warnings denied
with the export feature off and on". The tree it committed does not: the gate was not run on
the committed tree, or not on these files' last version. Every later landing on the wave, and
the module's switch, whose gates lint the merged tree, would fail on these lines.

## Evidence

On `a81dc3103`, from an `end-gates` snapshot of it (`crustify/tmp/gates-base`, a copy of the
commit's tree with `c/` and `crustify/builds` linked), from its root:

```
CARGO_TARGET_DIR=<scratch> crustify/scripts/cargo-variant.sh plain clippy --workspace --all-targets -- -D warnings
```

exits 101 (`clippy-affected.log`, the `Compiling` lines left out):

```text
error: this function has too many lines (102/100)
   --> tests/unit_pub/deps/llhttp/http.rs:104:1
error: very complex type used. Consider factoring parts into `type` definitions
   --> tests/unit_pub/deps/llhttp/http.rs:106:16
error: could not compile `libgit2-crustify` (test "unit_pub") due to 2 previous errors
error: this `unsafe` block contains 2 unsafe operations, expected only one
  --> tests/equiv/deps/llhttp/http.rs:65:9
error: could not compile `libgit2-crustify` (test "equiv") due to 1 previous error
```

The same command with `--features ffi-export-deps-llhttp-http-c` reports the same three
errors. No other file of the workspace fails.

## Fix

In this batch's second commit:

- `tests/unit_pub/deps/llhttp/http.rs`: the case tuple is the alias `HeadersCase`, and the
  loop that decides each case on a zeroed parser, with its read-only checks, is the helper
  `assert_after_headers_complete`, which `after_headers_complete_on_a_zeroed_parser` and the
  new `after_headers_complete_on_a_zeroed_parser_s_rarer_states` share. The cases are the
  same.
- `tests/equiv/deps/llhttp/http.rs`: `State::parser` reads the zeroed parser in one `unsafe`
  block and writes the described one in another, each justified.

On the fixed tree, `cargo fmt --all --check` passes, and `clippy --workspace --all-targets`
passes with warnings denied, with the export feature off and on, and in the scratch tree with
the module's equivalence file retired, as the switch will have it. The tests are unchanged in
what they assert: `unit_pub`'s and `equiv`'s `deps::llhttp::http` tests pass on plain.
