# `set_settings_ptr` admits callbacks that set a reason which dies before the parser

- **finding**: `contracts/ffi_import/settings-ptr-admits-a-reason-that-dies-first`
- **batch**: `wave-194/batch-5/run-1`, the `type` review of `deps/llhttp/api.c`'s folded waves,
  over `wave-194/batch-0` (`1ead111ee`) and `wave-194/batch-2` (`97dea9da3`)
- **items**: `fld::llhttp__internal_s.settings` (`ParserMut::set_settings_ptr`),
  `fld::llhttp__internal_s.reason` (`ParserRef::reason`)
- **affected commit**: `1ead111ee`, unchanged through `2a51ccb3f`
- **files**: `src/libgit2/deps/llhttp/llhttp.rs`

## Finding

`ParserRef::reason` is safe and follows the parser's `reason` pointer whenever it is called,
for the handle's borrow. Its `// SAFETY:` comment rests on who may set that pointer: llhttp
(its literals), `set_reason` (a `&'static CStr`) and "a handle built over C's parser". It
leaves out the callbacks of a table stored by `ParserMut::set_settings_ptr`, which run on a
parser Rust owns. llhttp lets every callback set any string as the reason:
`llhttp_set_error_reason` is to be "called in user callbacks right before returning the
errno" (`c/deps/llhttp/llhttp.h:730`), and its body stores the pointer
(`c/deps/llhttp/api.c:213`). `set_settings_ptr`'s `# Safety` asks only for a table "that stays
live and unchanged, and whose callbacks are sound to call with this parser and its `data`,
for as long as the parser executes with it". A callback that sets a reason its context owns
honours that, and so does a caller that frees the context once the parser is done executing.
The safe `reason()` then reads freed memory.

`Callback::from_fn`'s contract (`wave-194/batch-1`, `2c69341c3`), which the `Settings` getters
and `ParserMut::set_settings` rest on, is silent on the reason too. A C function it accepts
may serve any parser, so a reason it sets must live as long as the program. No caller of
`from_fn` passes a C function yet, so nothing reaches this one.

## Evidence

The reproducer stores a `static` table whose `on_url`, written as C writes one, reads its
`Context` from the parser's `data` and calls `llhttp_set_error_reason` with the context's
`CString`, returning `HPE_USER`. It sets the table and the context through the two `unsafe`
setters, parses `GET /refused HTTP/1.1\r\n`, clears the table and `data`, and frees the
context, every step within the setters' contracts. Then the safe `parser.as_ref().reason()`
reads the freed string (`run-affected.log`, exit 134):

```text
during the context: Some("URL refused by the caller\'s context")
==ERROR: AddressSanitizer: heap-use-after-free on address 0x700666de0010
READ of size 2 at 0x700666de0010 thread T0
    #0 in strlen
    #3 in <core::ffi::c_str::CStr>::from_ptr
    #4 in <libgit2_crustify::libgit2::deps::llhttp::llhttp::ffi_import::ParserRef>::reason src/libgit2/deps/llhttp/llhttp.rs:1862:27
    #5 in settings_ptr_admits_a_reason_that_dies_first::main src/main.rs:101
0x700666de0010 is located 0 bytes inside of 36-byte region
freed by thread T0 here: … in settings_ptr_admits_a_reason_that_dies_first::main src/main.rs:98
```

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

with `ASAN_OPTIONS=detect_leaks=0:abort_on_error=1` and an `ASAN_SYMBOLIZER_PATH`. It exits
134 on `2a51ccb3f`.

## Fix

In this batch's second commit, the contract is fixed. The code it guards does not change,
since the reason is the C callback's to keep alive:

- `ParserMut::set_settings_ptr`'s `# Safety` adds that a callback which sets the parser's
  `reason` sets it NULL or to a NUL-terminated string that stays live and unchanged while
  the parser holds it: until it is replaced or the parser is initialised again, or else for
  as long as the parser lives;
- `Callback::from_fn`'s, which `DataCallback::from_fn`'s repeats, adds that a reason such a
  function sets lives as long as the program, since it may serve any parser;
- `ParserRef::reason`'s `// SAFETY:` comment and the module documentation name both.

The reproducer still crashes on the fixed tree, but it now breaks the new obligation, since
it frees the reason while the parser holds it, so it is no longer a broken contract. Its
regression test is `ub_unsafe`'s
`deps::llhttp::llhttp::set_settings_ptr::a_reason_its_callbacks_set_is_read_while_the_parser_holds_it`.
It is the same scenario within the new contract: the reason is read through the safe getter
after the parse, then given up with `set_reason(None)` before its context goes. It passes
under ASan+UBSan, and alone under BSan.
