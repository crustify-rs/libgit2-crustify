# Lifecycle primitives the library does not export

Status: `declined`, superseded by the static campaign link (crustify `20500ce`), under which unexported symbols link and the primitives bind directly.

## Signal

A rule an agent had to work out by hand that a skill should have stated. The
`binding-symbols` skill's Lifecycle strategies section has a foundational module's items
become "a generic policy or strategy that calls the primitive", and `bindgen`'s Extending
section allowlists an internal header's item as any other. Neither says what a batch does
when the primitive cannot be called: when the library compiles it with hidden visibility,
or with internal linkage, so that the shared library the campaign's builds link does not
export it and no raw binding of it links.

## Evidence

- `wave-176/batch-0/run-1` of `crustify-rs/libgit2-crustify`, which binds
  `src/util/allocators/failalloc.c` (foundational): libgit2 compiles under
  `-fvisibility=hidden` (`c/cmake/DefaultCFlags.cmake:131`) and declares the module's three
  routines `extern`, not `GIT_EXTERN`, so `nm crustify/builds/plain/libgit2.so` lists
  `git_failalloc_malloc`, `git_failalloc_realloc` and `git_failalloc_free` as local (`t`)
  and `nm -D` not at all. Every item of the batch is unreachable through the raw bindings.
- The same holds for the next foundational batch, `src/util/alloc.c` (wave 178): its
  `git__allocator` (`d`), `git__calloc`, `git__strdup`, `git__strndup` and
  `git__substrdup`, and the `static inline` `git__malloc`, `git__realloc` and `git__free`,
  which read the hidden global.
- The legacy wrap campaign met the same gap and filled it differently each time: its
  review report `crustify/reviews/legacy-wave-3/batch-0/wavefront/string-cloners-unrecorded/`
  records that the string strategy "rebuilds `git__strdup` rather than binding it", over
  the exported `git_odb_backend_data_alloc`; the dashboard's `libgit2/config` note records a
  batch that marked two unexported constructors `GIT_EXTERN` in the C to link them, which
  the campaign later forbade.
- This batch reproduced the three constant bodies in Rust (`FailAlloc`, `FailArrayAlloc`,
  with `Replaces` anchors) and documented the one thing the reproduction cannot keep: C
  compares `git__allocator.gmalloc` with `git_failalloc_malloc`'s address in
  `git_allocator_global_init`, which a Rust function never equals.

## Destination

The spec: `binding-symbols`, Lifecycle strategies section, which the binder role evolves.

## Change

After the section's second paragraph ("In a foundational module's batch, …"), add:

> A primitive the library does not export, compiled with hidden visibility or internal
> linkage, has no raw binding that links, and the C is not changed to export it. Its
> strategy is composed over the exported routines that reach it on every path, as a
> strategy over another family's routines is; where none does, and its body depends on
> no state of the library, such as a constant result or a computation on its arguments
> alone, the strategy reproduces the body in Rust, annotated `Replaces`, while the C stays
> for the C that calls it. What the reproduction cannot keep is documented on the
> strategy: the routine's address, where C compares a function pointer with it, as
> Function pointers describes. A primitive that neither route reaches is reported, with
> the consumers that need it. Its equivalence tests reach the C through the exported
> functions that call it, as `testing` describes for a function the raw bindings do not
> reach.

## Validation

Not rerun: the change states the rule this batch followed, which its review checks. The
wave-178 batch of `src/util/alloc.c` is the next run it applies to.
