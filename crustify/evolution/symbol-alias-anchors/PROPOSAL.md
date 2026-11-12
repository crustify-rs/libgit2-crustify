# Where a symbol alias's anchor goes

Status: `landed`, Crustify `107cc09` (`devel`)

## Signal

A rule an agent had to work out by hand that a skill should state. `binding-symbols`'
Macros section binds a symbol alias "through the safe wrapper of the symbol it names", and
the conventions' What is annotated section puts a macro's anchor on "the generic, `const`,
function or `macro_rules!` that stands in for it". Neither says where the anchor of an
alias goes when the symbol it names has no wrapper of its own: a lifecycle primitive of a
foundational module, which the Lifecycle strategies section binds as strategy methods and
never as an ordinary safe function.

## Evidence

`foundational/libgit2/src/util/alloc.c/wave-177/batch-0/run-1` bound xdiff's
`xdl_malloc`, `xdl_calloc`, `xdl_realloc` and `xdl_free` (`deps/xdiff/git-xdiff.h`), each
`#define`d as one of `git__malloc`, `git__calloc`, `git__realloc` and `git__free`. The
first, second and fourth stand in Rust as `CAlloc::c_alloc`, `CLenAlloc::c_alloc_len`,
`CLenClone::c_clone_len`, `CAllocZeroed::c_alloc_zeroed`, `ArrayFree::new_zeroed` and the
three `c_drop`s of `src/libgit2/src/util/alloc.rs`. The run anchored each macro beside its
primitive's anchor on every one of those stand-ins (`mac::xdl_malloc` beside
`fn::git__malloc`, and so on), and `xdl_realloc` beside `fn::git__realloc` on
`ArrayFree::resize_bytes`; `ffimap api` keys all four as `new` items with those stand-ins.

## Destination and change

The spec, `binding-symbols`' Macros section, which the binder evolves: a symbol alias's
stand-ins are those of the symbol it names, each carrying the alias's anchor beside the
symbol's own, the strategy methods where the symbol is a lifecycle primitive.

## Validation

By the evidence that motivated it: the run above bound the four macros so, and its
`ffimap api` measure attributes each to its stand-ins.
