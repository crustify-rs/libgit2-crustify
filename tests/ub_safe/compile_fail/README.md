Tests that must fail to compile: one `.rs` file per illegal safe path, each with its
expected diagnostic in a sibling `.stderr` file (`TRYBUILD=overwrite` records it).

The plain variant compiles them on the pinned stable, and the sanitizer variants on the pinned
nightly, so a case's diagnostic must read the same on both. Where it can, a case is written so
that it does: a bound is checked through a local function (`fn send<T: Send>`), not through a
standard library one, whose note quotes the library's source on a toolchain with `rust-src`.

A case whose diagnostic the two toolchains word differently, and that no such rewording keeps
proving what it proves, is written once under `toolchain/`, and linked from `stable/` and
`nightly/`, each holding that toolchain's `.stderr`; the suite checks the directory of the
toolchain trybuild compiles with. Record each with `TRYBUILD=overwrite`, under the plain
variant for `stable/` and under a sanitizer variant for `nightly/`.
