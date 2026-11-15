class: nit
evidence: reading, and the header preprocessed for each target (`clang -E`); no reproducer, since the crate builds for Linux alone
conditions: any target but Windows is unaffected, this build included; on Windows (`*-pc-windows-msvc`, `*-pc-windows-gnu`) the constants differ from the header's expansion; reproduces on `f570d2a4e`

# `PRI_DZ`, `PRI_UZ` and `PRI_XZ` take one of the header's branches for every target

**Where.** `src/libgit2/src/util/cc_compat_h.rs`, `PRI_DZ`, `PRI_UZ` and `PRI_XZ`, the ports of
`mac::PRIdZ`, `mac::PRIuZ` and `mac::PRIxZ`, landed by `wave-219/batch-0` (`f570d2a4e`).

**What is wrong.** `src/util/cc-compat.h` defines the three formats by configuration: under
`_MSC_VER` or `__MINGW32__`, `"I64d"`, `"I64u"` and `"I64x"` where `SIZE_MAX == ULLONG_MAX` and
`"Id"`, `"Iu"` and `"Ix"` otherwise, and `"zd"`, `"zu"` and `"zx"` elsewhere. The port defined
each as this build's string for every target, and its docs said the other branches "format the
same values". `porting-symbols`' Macros section ports a macro whose expansion depends on the
configuration once only "where every configuration's expansion computes the same result for
every input". These branches compute different strings, and the strings are not
interchangeable: a Rust format would carry `"zu"` to the Windows C library the header avoids
it on. The conditions are ones Rust's `cfg` sees (`windows`, `target_pointer_width`), so
`binding-symbols`' shim, for a configuration "which the Rust does not see", does not apply
either: the port reimplements every branch.

No consumer of this crate observes it today. The crate builds and tests on Linux alone, where
the header and the constants agree, as `tests/equiv`'s `the_formats_are_the_headers` checks.

**Evidence.** The header, preprocessed for each target with clang's freestanding headers:

```
printf '#include <stdint.h>\n#include <limits.h>\n#include "cc-compat.h"\nconst char *d = PRIdZ, *u = PRIuZ, *x = PRIxZ;\n' > w.c
for t in x86_64-pc-windows-msvc i686-pc-windows-msvc x86_64-w64-mingw32 i686-w64-mingw32 x86_64-pc-linux-gnu aarch64-apple-darwin; do
  echo "$t: $(clang --target=$t -ffreestanding -E -Ic/src/util w.c | tail -1)"; done
```

```
x86_64-pc-windows-msvc: const char *d = "I64d", *u = "I64u", *x = "I64x";
i686-pc-windows-msvc: const char *d = "Id", *u = "Iu", *x = "Ix";
x86_64-w64-mingw32: const char *d = "I64d", *u = "I64u", *x = "I64x";
i686-w64-mingw32: const char *d = "Id", *u = "Iu", *x = "Ix";
x86_64-pc-linux-gnu: const char *d = "zd", *u = "zu", *x = "zx";
aarch64-apple-darwin: const char *d = "zd", *u = "zu", *x = "zx";
```

The Rust side was checked the same way, by a scratch `no_std` crate that includes the module's
file by `#[path]` and asserts, in a constant, the strings above for its target's `cfg`
(`cargo +nightly-2026-10-08 check -Zbuild-std=core --target <t>`). On `f570d2a4e`,
`x86_64-pc-windows-gnu` fails: `evaluation panicked: assertion failed: eq(cc::PRI_DZ, "I64d")
&& ...`. The crate cannot be built for Windows, so that check is evidence, not a reproducer.

**Fix.** A local `macro_rules!`, `size_format!`, defines each constant once per branch of the
header, under `cfg(not(windows))`, `cfg(all(windows, target_pointer_width = "64"))` and
`cfg(all(windows, not(target_pointer_width = "64")))`, from one invocation holding its docs,
anchor and alias. Each target compiles one item, whose value is the header's literal there.
The docs name every branch. On the fix, the scratch check passes for
`x86_64-unknown-linux-gnu`, `x86_64-pc-windows-gnu`, `i686-pc-windows-gnu`,
`x86_64-pc-windows-msvc` and `i686-pc-windows-msvc`, and `unit_pub`'s and `equiv`'s
`cc_compat_h` tests pass. On this build the values, types and paths are unchanged, and
`ffimap api diff` keeps all three. A first form, a `cfg!`-selected table, read as `changed`
there, since rustdoc renders a non-literal value as `_`.

**Tiers.** Caught by the module pass, `wave-219/batch-1`, the first tier after the batch's own
gates, which had it in scope and could not see it, Linux being the only build.
