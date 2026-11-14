# `set_settings_ptr` and `set_data` bound their obligations by later safe calls

- **finding**: `contracts/ffi_import/settings-ptr-bounds-a-table-by-later-safe-calls`
- **batch**: `wave-194/batch-6/run-1`, the `symbol` review of `deps/llhttp/api.c`'s folded
  waves, over `wave-194/batch-1` (`2c69341c3`), `wave-194/batch-3` (`2a51ccb3f`) and
  `wave-194/batch-4` (`b3d6b9b37`)
- **items**: `fld::llhttp__internal_s.settings` (`ParserMut::set_settings_ptr`),
  `fld::llhttp__internal_s.data` (`ParserMut::set_data`), whose contracts batch 4 widened for
  the 23 dispatchers it made safe (`fn::llhttp__on_url` and the others, `ParserMut::on_url`…)
- **affected commit**: `b3d6b9b37`
- **files**: `src/libgit2/deps/llhttp/llhttp.rs`, `src/libgit2/deps/llhttp/api.rs`

## Finding

Batch 4 made api.c's dispatchers the parser's safe methods: `ParserMut::on_url(&[u8])`,
`on_message_complete()` and the 21 others read the table the parser's `settings` addresses
and call the slot's callback with the parser, at any time, not only from a parse
(`src/libgit2/deps/llhttp/api.rs`, `ffi_import`'s `table`, `call_event`, `call_span`). It
widened the two `unsafe` setters' contracts for them, but kept their duration as a span of
*execution*:

- `set_settings_ptr`: the table "stays live and unchanged, and whose callbacks are sound to
  call with this parser and its `data`, for as long as the parser executes with it: api.c's
  dispatchers read it on every event of `llhttp_execute` and `llhttp_finish`";
- `set_data`: "while the parser executes with those settings, `data` must be what they
  expect", the parser executing "whenever one of its parses, its `finish` or one of api.c's
  dispatchers … calls them, which the safe API allows at any time".

An obligation bounded by when the parser *executes* is one that later safe code, not the
`unsafe` caller, decides the end of: a caller who keeps the table for every parse and finish
it runs, and frees it once it has done executing the parser, reads the first sentence as
satisfied, yet any safe code that later holds the parser calls a dispatcher, which reads the
freed table. What the dispatchers rely on is what `table()`'s `// SAFETY:` comment states, and
the contract did not: that the table lives and stays unchanged "for as long as the parser
holds it", until it is replaced or the parser is initialised again (`reset` keeps it). The
same holds of `data` beside a table whose callbacks read it.

## Evidence

The reproducer (`src/main.rs`) stores a heap table whose `on_message_complete`, written as C
writes one, counts messages in the `u32` its parser's `data` addresses. It sets `data` and the
table through the two setters, parses one response, which runs the callback once, and frees
the table, executing the parser no more. A safe function that holds the parser then calls
`on_message_complete()`, which reads the freed slot (`run-affected.log`, exit 134):

```text
messages after the parse: 1
==ERROR: AddressSanitizer: heap-use-after-free on address 0x76f4d3fe0098
READ of size 8 at 0x76f4d3fe0098 thread T0
    #0 in <…::llhttp::api::EventSlot>::read src/libgit2/deps/llhttp/api.rs:976:48
    #1 in <…::llhttp::llhttp::ffi_import::ParserMut>::call_event src/libgit2/deps/llhttp/api.rs:1000:33
    #2 in <…::ParserMut>::callback_maybe src/libgit2/deps/llhttp/api.rs:830:14
    #3 in <…::ParserMut>::on_message_complete src/libgit2/deps/llhttp/api.rs:814:26
    #4 in settings_ptr_bounds_a_table_by_later_safe_calls::safe_code src/main.rs:39:12
0x76f4d3fe0098 is located 88 bytes inside of 184-byte region [0x76f4d3fe0040,0x76f4d3fe00f8)
freed by thread T0 here: … in settings_ptr_bounds_a_table_by_later_safe_calls::main src/main.rs:63:5
```

Offset 88 of the 184-byte table is its twelfth slot, `on_message_complete`.

## Reproducer

`Cargo.toml` and `src/main.rs` beside this report. From this directory, with the
`asan-ubsan` variant's flags, as `crustify/scripts/cargo-variant.sh` sets them:

```
side=$(realpath -m ../../../../../../../../../../../../crustify/builds/asan-ubsan/../../../..)
CRUSTIFY_BUILD_VARIANT=asan-ubsan \
PKG_CONFIG_PATH="$side/pcre2-bindings/crustify/builds/asan-ubsan/install/lib/pkgconfig" \
RUSTFLAGS="-Zsanitizer=address -Cforce-frame-pointers=yes" \
cargo +nightly-2026-10-08 run --target x86_64-unknown-linux-gnu
```

with `ASAN_OPTIONS=detect_leaks=0:abort_on_error=1` and
`ASAN_SYMBOLIZER_PATH=/usr/lib/llvm-18/bin/llvm-symbolizer`. It exits 134 on `b3d6b9b37`.

## Fix

In this batch's second commit, the contracts are fixed; the code they guard does not change,
since what it relies on was already the table's and the context's liveness while the parser
holds them:

- `ParserMut::set_settings_ptr`'s `# Safety` asks for the table, and its callbacks'
  soundness with the parser and its `data`, "for as long as the parser holds it: until
  another table or NULL replaces it (this setter, `set_settings`, `init`) or `reinit` clears
  it, which `reset` does not, or else for as long as the parser lives", and says that the
  dispatchers read it and call its callbacks at any time while it is held, a parse or none;
- `ParserMut::set_data`'s asks that `data` be what the callbacks expect for as long as the
  parser holds both it and the table whose callbacks read it, and says how a context that goes
  first is given up: the table cleared or replaced first, then `data`.

The reproducer still crashes on the fixed tree, but it now breaks the obligation, since it
frees the table while the parser holds it. Its regression tests are `ub_unsafe`'s
`deps::llhttp::llhttp::set_settings_ptr::the_dispatchers_call_its_callbacks_at_any_time_while_the_parser_holds_it`,
the same scenario within the new contract: a heap table whose callbacks the dispatchers call
outside any parse, in an order llhttp never uses and with runs of the test's own, `reset`
keeping it, given up with `set_settings_ptr(NULL)` before it is freed; and
`deps::llhttp::llhttp::set_data::the_dispatchers_read_the_context_it_stores_outside_a_parse`,
a context replaced between two dispatcher calls and given up after the table. Both pass on
plain and under ASan+UBSan; run alone under Miri, each stops at `llhttp_init` (`calls C`).
