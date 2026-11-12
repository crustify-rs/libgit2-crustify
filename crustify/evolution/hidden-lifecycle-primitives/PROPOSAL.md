# Lifecycle primitives the target's shared library hides

Status: `open`

## Signal

A rule the binder had to work out by hand that a skill should have stated. The
`binding-symbols` skill's Lifecycle strategies section has a foundational module's strategy
"call the primitive", and its Functions section has a function's FFI call "through the raw
bindings". Neither says what a binding does when the primitive cannot be called at all:
when the target's build compiles it with hidden visibility, so the shared library the
campaign mode links does not export it, and no raw binding of it would link.

## Evidence

- `crustify-rs/libgit2-crustify`, wave 12 batch 1 (`src/util/alloc.c`, foundational, run
  1): all seven items, `git__malloc`, `git__free`, `git__calloc`, `git__mallocarray`,
  `git__strdup`, `git__strndup` and `git__substrdup`, are hidden in `libgit2.so`
  (`nm` lists `git__calloc` and `git__strdup` as local `t`, and `git__allocator`, which the
  two `static inline` ones read, as local `d`). The campaign links the variants' shared
  libraries (`bindgen`'s campaign mode), so neither an allowlist entry nor a
  `static_fns.c` wrapper of the inline ones links.
- The batch reached the family through the two exported routines whose bodies are the
  primitives alone (`git_odb_backend_data_alloc`: `return git__malloc(len);`,
  `git_odb_backend_data_free`: `git__free(data);`) and composed the other five over them as
  `alloc.c` writes them, with two documented differences (`grealloc(NULL, n)` reached as
  `gmalloc(n)`; an empty array allocates nothing). Its equivalence test compares the
  allocator calls, file and line included, of the composed strategies with those of the
  exported C routines that call the hidden ones (`git_signature_new`, `_dup`, `_free`), and
  they match.
- The wrap campaign before it had already done the same by hand, once per consumer
  (`OdbDataFree`, `OdbDataStrFree` in `src/libgit2/src/libgit2/odb.rs`), and a review
  recorded the reason (`crustify/reviews/legacy-wave-3/batch-0/wavefront/string-cloners-unrecorded`).
- The same holds for the two batches of wave 169 and 176 running beside it (`deps/reftable/basics.c`,
  `src/util/allocators/failalloc.c`), whose items are hidden too.

## Destination

The spec: `skills/rust/binding/symbols/SKILL.md`, Lifecycle strategies (the binder's to
evolve, proposed since the settings' `evolution` answer has no `unattended` part). The
build question it leaves, whether campaign variants should export the hidden lifecycle
primitives instead, is the `setup` and `bindgen` skills' and is raised here, not changed.

## Change

At the end of Lifecycle strategies, add:

> A primitive the target's build does not export, such as one compiled with hidden
> visibility, is reached through an exported routine whose body is that primitive alone,
> and a primitive with no such routine is composed in the strategy from the ones reached,
> as its body writes it: the same size checks, errors and copies. The batch documents each
> difference from the body that remains, and its equivalence tests compare the composed
> strategy with the exported routines that call the hidden one. The exported routine's own
> module is then a dependency of the strategy: once that module is ported, the routine's
> export reaches the primitive through the strategy, so the plan gives the strategy
> another route before that integration.

## Validation

Not rerun: the change states what wave 12 batch 1 did, and the batches of waves 169 and
176 are the next to meet it.
