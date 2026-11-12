# Where the bindings find an internal header's include path and definitions

Status: `open`

## Signal

A rule an agent had to work out by hand that a skill should state. Crustify `20500ce` makes
internal functions link, through a variant's static archive. `bindgen`'s Extending section
says that `wrapper.h` gains "an internal header only when one of [the batch's items] is
declared there". No skill says how the bindings then find that header, the headers it
includes, or the definitions it is compiled with. Build modes adds that `shims.c` and
`static_fns.c` are compiled "against the headers the bindings match" in every mode. Both
stop working once `wrapper.h` names an internal header.

## Evidence

`fixes/static-campaign-link/run-1` (this run) allowlisted `reftable_buf_init`,
`reftable_buf_release` and `reftable_buf_add`, which `deps/reftable/basics.h` declares, and
added `#include "basics.h"` to `libgit2-sys/bindgen/wrapper.h`.

- With campaign mode's include path as it stood (`c/include` only), bindgen failed:
  `basics.h` includes `system.h`, `reftable-system.h`, `src/util/posix.h` and `util.h`, then
  `git2_util.h`, which includes `git2_features.h`. That header is generated into the build
  dir (`<variant>/gen_headers`), and `<zlib.h>` comes from the side campaign's install
  (`-isystem .../zlib-bindings/crustify/builds/<variant>/install/include`). Neither
  exists outside a configured build.
- What libgit2's own TUs compile with is recorded in each compile edge of `build.ninja`:
  `INCLUDES` (src/libgit2, src/util, include, gen_headers, deps/*, the side installs'
  include dirs), `DEFINES` (`GIT_DEPRECATE_HARD`, `_FILE_OFFSET_BITS=64`) and the `-D`
  flags of `FLAGS` (`_GNU_SOURCE`, and `NDEBUG` in the `RelWithDebInfo` variants but not
  in the `Debug` coverage variant). The run takes them from the compile edge of the link
  unit's first own object (`CMakeFiles/libgit2.dir/`), mapping the configured source dir
  (`CMAKE_HOME_DIRECTORY`) to the checkout's `c/`, and generates and compiles the wrappers
  with them. The public bindings came out byte-identical, and the three functions bound.
- Three consequences that no skill covers:
  1. A definition that differs between variants, such as `NDEBUG`, can change an
     allowlisted internal struct's layout, so the bindings regenerated against one variant
     would fail the comparison with those committed from another. The public headers do
     not depend on it, so this has not happened yet.
  2. `static_fns.c` includes `wrapper.h` and is compiled in every mode. The first
     allowlisted `static inline` function of an internal header, such as `git__malloc`
     (`src/util/alloc.h`, which would now link, since it reads the hidden
     `git__allocator`), makes it fail to compile wherever only the installed headers are
     present: system mode, and the shipped build of this tree. Bundled mode would need the
     generated headers too, in `OUT_DIR`.
  3. The `bindgen` feature outside campaign mode regenerates from an install's headers,
     so it cannot regenerate once `wrapper.h` names an internal header.

### `wave-169/batch-1` (`deps/reftable/basics.c`)

The batch's nine `static inline` functions of `basics.h` (`reftable_put_be16` to `_be64`,
`reftable_get_be16` to `_be64`, `reftable_alloc_size`) were ported rather than allowlisted,
as `binding-symbols` allows for short, configuration-independent header functions:
allowlisting any of them would have generated `static_fns.c`, which includes `wrapper.h` and
so `basics.h`, `system.h` and `git2_util.h`, and consequence 2 would have broken the shipped
build. With the ports, `cargo package -p libgit2-sys --offline` against a staged `plain`
install (`CRUSTIFY_BUILD_VARIANT=` and `LIBGIT2_DIR`) still builds: `shims.c` includes only
`shims.h`, and no `static_fns.c` exists. The cost is that the ported helpers have no C to be
compared with: their tests check the bytes the C bodies write. The two extern functions the
batch allowlisted, `names_equal` and `names_length`, generate declarations only, which link
in campaign mode and not against an install's `libgit2.so`, as `internal-symbols-unlinkable`
records.

### Further evidence: the first internal `static inline` function (wave-12/batch-2)

`foundational/libgit2/src/util/alloc.c/wave-12/batch-2/run-1` allowlisted `git__malloc`
and `git__free`, `static inline` in `src/util/alloc.h`, with `#include "alloc.h"` in
`wrapper.h`, and the five functions `alloc.c` defines. Campaign mode generated
`libgit2-sys/src/static_fns.c` (`git__malloc__extern`, `git__free__extern`), compiled it
with the variant's flags and include dirs, and the strategies of
`src/libgit2/src/util/alloc.rs` call all seven on `plain` and `asan-ubsan`. Consequence 2
above then happened as predicted: the packaging gate, `cargo package -p libgit2-sys
--offline` with `CRUSTIFY_BUILD_VARIANT=` and `LIBGIT2_DIR` set to a `cmake --install` of
the `plain` build, fails to verify the tarball, since `static_fns.c` includes `wrapper.h`,
whose `#include "alloc.h"` the installed headers cannot satisfy:

```text
bindgen/wrapper.h:31:10: fatal error: 'alloc.h' file not found
```

A fourth consequence follows, which the change above does not cover: once the safe crate
calls an internal function, as `alloc.rs` now calls `git__calloc`, system mode cannot link
it either, whatever the wrappers do, since an installed `libgit2.so` exports none of them.
The shipped build then needs the static archive, as bundled mode would build it, or the
internal functions the safe crate calls exported, or system mode gated off once a safe
wrapper reaches an internal symbol, as it is gated off once a module is integrated.

### Further evidence: an internal header with no `static inline` function (wave-182/batch-0)

`foundational/libgit2/src/util/allocators/win32_leakcheck.c/wave-182/batch-0/run-1`
allowlisted `git_win32_leakcheck_init_allocator` and added `#include
"allocators/win32_leakcheck.h"` to `wrapper.h`, after `alloc.h`. Campaign mode generated
its binding and linked it from the static archive (`nm -D libgit2.so` lists none of it,
`libgit2.a`'s `win32_leakcheck.c.o` defines it). The packaging gate, run as above against a
prefix assembled from the `plain` build (`c/include`, the build's `include/`, its
`libgit2.so*` and `libgit2.pc`), fails exactly as before, at `wrapper.h:31` (`'alloc.h' file
not found`) while compiling `static_fns.c`: the new header, on line 32, is never reached, and
would fail the same way, since no install carries `src/util`. The fourth consequence holds
too: the safe crate now calls a second internal function an installed `libgit2.so` does not
export.

## Destination

The spec: `bindgen`, Generation and Build modes, which the binder does not own.

## Change

In `bindgen`, Generation, after the settings list:

> An internal header's declarations are generated as the link unit's own TUs see them: in
> campaign mode, with the include dirs and preprocessor definitions the link unit's own
> translation units compile with, as the variant's build records them, its generated
> headers and its out-of-tree libraries' include dirs among them, the source dirs taken
> from the checkout's `c/`. An item whose declaration depends on a definition that differs
> between variants is generated from the `plain` variant's configuration in every mode.

In Build modes, after "the bindings are the committed ones unless the `bindgen` feature
regenerates them":

> The wrappers of `static inline` functions declared in internal headers are compiled
> only where the internal headers and the build's generated headers are found: campaign
> mode, and bundled mode, which generates them into `OUT_DIR`. System mode leaves them
> out, and the `bindgen` feature regenerates there only from the public headers.

## Validation

This run's tree validates the campaign-mode half. `libgit2-sys/build.rs` reads the edge
and generates the reftable bindings, which link from the static archive and run in
`libgit2-sys/tests/hidden_symbols.rs` on `plain`, `asan-ubsan`, `tsan` and `bsan`. The
committed public bindings were unchanged. The `static inline` and system-mode half is
untested, since no internal `static inline` function is allowlisted yet.

## Further evidence

`foundational/libgit2/deps/reftable/basics.c` `wave-169/batch-2/run-1` met consequence 2.
Allowlisting `reftable_alloc_grow`, a `static inline` function of `deps/reftable/basics.h`,
made bindgen write `libgit2-sys/src/static_fns.c`, which includes `wrapper.h` and so
`basics.h`. Packaging `libgit2-sys` in installed mode then failed while verifying the
tarball. The prefix held `c/include`'s headers and the `plain` variant's `libgit2.so`:

```text
CRUSTIFY_BUILD_VARIANT= LIBGIT2_DIR=<prefix> cargo package -p libgit2-sys --offline --allow-dirty
  cargo:warning=.../libgit2-sys-0.1.0/bindgen/wrapper.h:31:10: fatal error: 'basics.h' file not found
error: failed to verify package tarball
```

The batch therefore did not allowlist the function. It reimplemented it in Rust instead, as
`binding-symbols`' Functions section allows for a short `static inline` function: it is
`ReftableArrayFree::grow` (`Replaces fn::reftable_alloc_grow`), over `reftable_realloc`.
Without `static_fns.c` the package verifies again. Until the change above lands, the
exception is the only way to bind an internal `static inline` function and keep the
packaging gate passing. A function that depends on the build's configuration has no way
at all.

## Further evidence: `wave-176/batch-1`

`wave-176/batch-1/run-1` allowlisted `git_failalloc_malloc`, `_realloc` and `_free`,
which `src/util/allocators/failalloc.h` declares, and which the safe crate's
`FailAlloc` and `FailArrayAlloc` call. It added `#include "allocators/failalloc.h"` to
`wrapper.h`. They are the first internal functions the safe crate itself, rather than a
`-sys` test, references. Against a staged install of the `plain` variant (`LIBGIT2_DIR`,
no `CRUSTIFY_BUILD_VARIANT`):

- `cargo package -p libgit2-sys` passes, since shipped mode compiles only `shims.c` and
  the crate's verification links no binary.
- A dependent binary that uses the family, `Allocator::new::<FailAlloc>()` in a probe
  crate's `main`, fails to link: `rust-lld: error: undefined symbol:
  git_failalloc_malloc`, and the same for `_realloc` and `_free`. `libgit2.so` exports
  none of the three (`nm -D` lists no `failalloc`). A dependent that does not use the
  family links and runs. So consequence 2 reaches past `static inline` wrappers. Every
  internal function the safe crate calls is unreachable in system mode, and the safe API
  over it is unusable there.
- Any build that enables the `bindgen` feature outside campaign mode fails on the first
  internal include of `wrapper.h` (`fatal error: 'allocators/failalloc.h' file not
  found`). This includes the safe crate's own tests through its dev-dependency.
  That is consequence 3.

The Build modes change should therefore also say what system mode does with the safe API
over internal functions: whether to leave it out, or link a static archive as campaign
mode does.
