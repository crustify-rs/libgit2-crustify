# pcre2-sys's vendored PCRE2 shadows the side campaign's for libgit2's C

Status: `open`

## Signal

The wrong dependency is used without anyone noticing. The variants link libgit2 against
the side campaigns' installs of OpenSSL, PCRE2 and zlib (`build.json` version 3). In every
Rust test binary, though, libgit2's `src/util/regexp.c` calls a different PCRE2: the copy
that the `pcre2-sys` crate vendors and builds.

## Evidence

From `fixes/static-campaign-link/run-1`, at `crustify/devel` `5d0e779f8` and on that run's
tree:

- The safe crate depends on `pcre2-sys = "0.2"` (0.2.10 in `Cargo.lock`), which
  `src/libgit2/src/libgit2/config.rs` uses directly. Its build script links a system
  `libpcre2-8` only when `pkg-config` finds one. The host has no `libpcre2-8.pc`
  (`pkg-config --modversion libpcre2-8` fails), so it compiles its vendored `upstream/`
  PCRE2 into a static library.
- Base, where libgit2 links shared: `ub_safe`'s binary defines `pcre2_compile_8` and six
  more `pcre2_*_8` symbols, and exports them in its dynamic symbol table, since
  `libgit2.so` references them (`nm -D`: `T pcre2_compile_8`). At run time `libgit2.so`'s
  `U pcre2_compile_8` binds to the executable's copy, which comes first in the lookup
  scope. `/target/pcre2-bindings/.../install/lib/libpcre2-8.so` is loaded, as
  `libgit2.so`'s own `NEEDED` entry, but none of its symbols is used.
- This run, where libgit2 links statically: `libgit2.a`'s references resolve at link time
  to the same vendored copy. `libpcre2-8.so` is linked `--as-needed` and dropped. OpenSSL
  and zlib do come from the side campaigns' installs (`ldd`: `libssl.so.3`,
  `libcrypto.so.3` and `libz.so.1` under `/target/*-bindings/crustify/builds/<variant>/install/lib`).
- So the sanitizer variants' C calls a PCRE2 that `cc` builds without the variant's
  instrumentation (`cargo-variant.sh` sets no `CFLAGS`). The side campaign's PCRE2
  (10.42, built per variant) is not the one under test. Its future port would not be
  reached by libgit2's C.

## Destination

A campaign artifact: the safe crate's dependency on `pcre2-sys`, and how campaign mode
builds it, which the assistant owns (`dependencies.json`, `crustify/scripts/cargo-variant.sh`).
It is not a rule of the spec yet. It could become one in `dependencies`' Sources section
("an adopted crate's own copy of an out-of-tree library never replaces the side
campaign's install in the link") if a second campaign meets it.

## Change

In campaign mode, point `pcre2-sys` at the variant's side-campaign install, so that the
safe crate's Rust and libgit2's C share the side campaign's `libpcre2-8.so`. For example,
`cargo-variant.sh` and `.cargo/config.toml` could export
`PKG_CONFIG_PATH=<pcre2-bindings>/crustify/builds/<variant>/install/lib/pkgconfig` and
`PCRE2_SYS_STATIC=0`. Its lib dir is already on every test binary's rpath. Alternatively,
record the vendored copy as a decision in `dependencies.json`, with the reason.

## Validation

Not validated: the change is the assistant's to choose. The evidence above is reproduced
with `nm -D --defined-only <test binary> | grep pcre2_compile_8` and
`readelf -d <test binary> | grep NEEDED`, on base and on this run's tree.
