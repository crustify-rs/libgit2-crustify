# The variants must let the Rust link the internal symbols it binds

Status: `open`

## Signal

A rule that every binding and port batch of an internal module depends on and no skill
states. The binding skills bind an item through its raw binding, and the raw bindings'
campaign mode links the variant's shared library. The setup skill builds each variant in
the target's own configuration. Nothing makes sure that the symbols the batches call can be
linked from those builds. A target that builds its library with hidden visibility, as
libgit2 does (`-fvisibility=hidden` in `cmake/DefaultCFlags.cmake`, so only `GIT_EXTERN`
symbols are exported), exports none of its internal functions. In a port campaign, which
covers every translation unit, most of the C the batches bind is internal. None of it
links.

## Evidence

- `wave-169/batch-0` (`deps/reftable/basics.c`, foundational, this run). The batch binds
  `struct reftable_buf` and `enum reftable_hash`. The type route binds a type's lifecycle
  routines with it (`binding-types`, Lifecycle). Here those are `reftable_buf_init`
  (`init`) and `reftable_buf_release` (disposer), so the buffer's owner should be
  `CVal<Buf, _>` over `reftable_buf_release`. `nm -D crustify/builds/plain/libgit2.so`
  lists none of `basics.c`'s functions, and `nm` shows them as local (`t`):
  `reftable_buf_init`, `reftable_buf_release`, `reftable_malloc`, `reftable_set_alloc`.
  The batch could bind only the layout, the handles and the getters. The owner,
  `basics.c`'s 31 functions in batches 1 and 2 (waves 170 to 175), and every
  `deps/reftable/*.c` module after them cannot be linked. The one exception is the
  `static inline` functions, which `static_fns.c` compiles into the crate.
- `wave-176/batch-0` (`src/util/allocators/failalloc.c`, running beside this one) hit the
  same wall. Its submitted findings say that `libgit2.so` "does not export it, so Rust
  cannot link it; the Rust strategy FailAlloc reproduces the constant body". That is a port
  of an item scheduled as a wrap, done to get around the link.
- The campaign's memory records the same constraint from the wrap campaign (config wave
  4): `git_config_backend_from_string` was unexported, a batch marked it `GIT_EXTERN`, and
  the operator reverted it. The rule kept since then is to never edit compiled C to make a
  symbol link. That rule was harmless while the scope was the public API alone. Under a
  port's scope it blocks every internal module.
- The unmerged `crustify/migrations/bundled-ffi` branch moves the crate to bundled mode,
  which links the C statically. Hidden visibility does not stop static linking, so that
  mode would lift the block for Rust consumers. The campaign mode the gates use still
  links the variants' shared libraries.

## Destination

The spec: `setup`'s Builds section, with `bindgen`'s Build modes section, which says how
campaign mode links a variant. The campaign artifact it implies, `build.json`'s variants,
belongs to the assistant.

## Change

In `setup`, Builds, add one rule:

> Every symbol a batch binds or ports must be linkable from each variant. A target that
> builds with hidden visibility, or with a symbol map that exports only its public API,
> does not export its internal functions. For such a target, each variant also produces
> what campaign mode can link them from: the library's objects or a static archive, which
> keep hidden symbols linkable, or a shared library built with default visibility. The
> variant records which of these it is. The gates never link a module's C twice: campaign
> mode links either the static form or the shared one, never both.

In `bindgen`, Build modes, under campaign mode: the variant's static form, when it records
one, is linked statically, with no rpath to it.

One question is left to the assistant and the user. Linking the C statically in campaign
mode would make the gates link the C the way bundled mode does, which suits the
`bundled-ffi` migration. A default-visibility shared build keeps today's dynamic link but
changes the configuration away from the target's own. That conflicts with "each variant
is configured as the settings' `target-config` asks", unless the settings record it.

## Validation

I ran a probe in this run: a C program that calls `reftable_buf_add` and
`reftable_buf_release` on a zeroed buffer.

- Linked against `crustify/builds/plain/libgit2.so`, it fails: `undefined reference to
  'reftable_buf_release'`.
- Linked against the same build's object
  `deps/reftable/CMakeFiles/reftable.dir/basics.c.o`, it links and runs correctly: the
  buffer is filled, then released to `alloc == 0` with a NULL `buf`.

So the static route resolves the hidden symbols without changing the C's configuration.
Until a rule like this lands, batches 1 and 2 of wave 169's packing (`basics.c`'s symbols)
can bind only the `static inline` helpers and the macros.
