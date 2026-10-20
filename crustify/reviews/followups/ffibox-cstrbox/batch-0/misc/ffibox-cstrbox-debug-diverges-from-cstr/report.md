# ffibox's `CStrBox` `Debug` no longer formats like `CStr`'s

- **where**: ffibox, `src/refs.rs`, `impl fmt::Debug for CStrBox<D>` (line 727). This is
  the uncommitted change in `/opt/venv/share/ffibox` on top of `4722185`, which
  `crustify/rust/libgit2` depends on by path. The `sha256sum` of
  `git -C /opt/venv/share/ffibox diff` is `802b9393…a97ec`.
- **reached from this crate through**: `OdbDataStr` (`CStrBox<OdbDataStrFree>`): every
  string `BufMut::take_str`, `StrArrayMut::take_strings` (its elements),
  `SignatureMut::replace_name` / `replace_email` and `OdbDataStrFree::dup` hand out.
- **class**: misc, a defect in ffibox. It is filed here and not fixed, as this batch's
  instructions ask. It is a divergence from the documented behaviour and from the
  previous `CStrBox` (`debug_tuple("CStrBox").field(&self.as_c_str())`).
- **reproducer**: `src/main.rs`. Run
  `LD_LIBRARY_PATH=/target/crustify/builds/plain cargo run` from this directory. It
  exits 101 on `ac7dd20e5` with the ffibox change (`run-affected.log`).

## Finding

The new impl writes each byte through `core::ascii::escape_default`. Its doc comment
says "As `CStr`'s … so the output is the same", and the ffibox change was described
the same way ("`Debug` escapes bytes as `CStr`'s does"). That holds only for ASCII and
for bytes that are not UTF-8. `CStr`'s `Debug` (`core/src/ffi/c_str.rs:174`, on both
installed toolchains: stable 1.99.0 and nightly 1.101.0) goes through `ByteStr`'s
`Debug`. That impl decodes UTF-8 runs and prints valid characters with
`char::escape_debug`, so a valid multi-byte character prints as itself. Only invalid
bytes print as `\xNN`:

```
   same  CStr: CStrBox("caf\xe9")
         ffibox: CStrBox("caf\xe9")
DIFFERS  CStr: CStrBox("héllo")
         ffibox: CStrBox("h\xc3\xa9llo")
DIFFERS  CStr: CStrBox("José ✓")
         ffibox: CStrBox("Jos\xc3\xa9 \xe2\x9c\x93")
OdbDataStrVec: ["refs/heads/main", "q\"t\\", "caf\xe9", "héllo", "José ✓"]
first element: CStrBox("h\xc3\xa9llo")

thread 'main' panicked at src/main.rs:29:5:
2 of 5 strings format unlike CStr
```

So every non-ASCII UTF-8 string formats differently than it did before the change,
e.g. a signature name or a ref name. Within this crate, `OdbDataStrVec`'s `Debug`
(which still formats each element as a `CStr`, through a copy) and its elements'
own `Debug` now disagree, as the last two lines show.

ffibox's own test locks the divergent output in:
`tests/macros.rs::cstr_macro_bound_policy` asserts
`format!("{s:?}") == r#"CStrBox("h\xc3\xa9llo")"#`. libgit2's
`ub_safe::sys_odb_backend::str_copies_strings_of_every_length` compares against
`format!("CStrBox({src:?})")` on ASCII text only, so it did not notice.

## Suggested fix (in ffibox, not applied)

Format as `ByteStr` does, without a reference over the string. Decode the copied-out
bytes incrementally: at most 4 bytes of state for a pending UTF-8 sequence. Or, since
`Debug` is not hot, copy the bytes into a stack or heap buffer and format that with
`<[u8]>::utf8_chunks`, as `ByteStr` does. Then make the ffibox test assert
`format!("CStrBox({:?})", c"héllo")`.
