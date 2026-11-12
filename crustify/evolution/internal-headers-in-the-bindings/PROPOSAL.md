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
