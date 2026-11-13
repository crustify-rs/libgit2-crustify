# An adopted spec, outcome or parser lends its borrows for any lifetime its adopter names

- **finding**: `contracts/ffi_import/adopted-handles-outlive-their-borrows`
- **batch**: `wave-186/batch-3/run-1`, the review of `wave-186/batch-0` (`8a149ac08`) and
  `wave-186/batch-2` (`fe6b85bfa`), `opt.h`'s types
- **items**: `st::cli_opt_spec`, `st::cli_opt`, `st::cli_opt_parser`
- **affected commit**: `fe6b85bfa`
- **where**: `src/git2/src/cli/opt.rs`, `mod ffi_import`: the handles `define_ctype!` mints
  for `Spec<'p>`, `Opt<'p>` and `Parser<'p>`, their public `unsafe` seams
  `CRefSeam::from_ptr` / `CMutSeam::from_ptr`, and the getters whose `SAFETY:` comments rest
  on "its handle's adoption": `SpecRef::{target, name, value_name, help}`,
  `OptRef::{spec, arg, value}`, `ParserRef::{specs, args}`, `ParserMut::args_mut` and
  `Arg::as_c_str`

## Finding

The three layouts carry `'p`, the lifetime of the strings, variables, spec table and
argument array they point at. Their getters lend those for `'p`, not for the handle's
borrow `'a`. For a handle built in Rust that is sound: each setter takes a `&'p` borrow.
For a handle adopted from a pointer, the getters' `SAFETY:` comments say "a spec C built
holds, by its handle's adoption, `NULL` or a string that lives unchanged for `'p`". No
contract asks an adopter for that. The only public adoption is ffibox's `CRefSeam::from_ptr`
and `CMutSeam::from_ptr`, whose `# Safety` is `CCell::ref_from_ptr`'s: "`ptr` must address a
live, initialised `Self::C` (or be null) that outlives `'a`, and nothing may write it while
the handle lives except through a `Mut` derived from the same borrow". It says nothing of
`'p`, which the adopter picks freely, nor of the `value` pointer matching its `type`, which
`target()` casts by. A caller who honours that contract still gets dangling borrows from
safe getters.

## Evidence

The reproducer, `src/main.rs` beside this report, builds a spec in Rust storage whose name
is a short-lived `CString`. It adopts the spec through `SpecRef::<'_, 'static>::from_ptr`,
keeping every clause of the stated contract: the spec is live and initialised, it outlives
the handle, and nothing writes it. It reads `name()`, a `&'static CStr`, then drops the spec
and the string. On `fe6b85bfa` Miri stops at the use (`run-affected.log`):

```
error: Undefined Behavior: constructing invalid value of type &std::ffi::CStr: encountered a dangling reference (use-after-free)
  --> src/main.rs:27:22
   |
27 |     println!("{:?}", name.to_bytes());
   |                      ^^^^ Undefined Behavior occurred here
```

The same holds for `OptRef::arg`, `OptRef::value` and `OptRef::spec`, and for
`ParserRef::specs` and `ParserRef::args`, which lend `'p` borrows read from the layout in
the same way.

## Reproducer

`Cargo.toml` and `src/main.rs` beside this report. From this directory, in a checkout of the
affected commit:

```
CRUSTIFY_BUILD_VARIANT=plain cargo +nightly-2026-10-08 miri run
```

It exits 1 with the report above. It calls no C, so Miri reaches it; an ASan build reports
the same use after free.

## Fix

The generic contract is ffibox's, so the general fix is proposed there:
`crustify/evolution/ffibox-adoption-of-lifetime-cells`. On the crate's side, `Spec`, `Opt`
and `Parser` each gain an `# Adoption` section stating what an adopter vouches beyond the
seams' contract: that the pointee is a valid `Spec<'p>`, `Opt<'p>` or `Parser<'p>` for the
`'p` it names. Its strings are `NULL` or NUL-terminated and live unchanged for `'p`. A spec's
`value` is `NULL` or a live variable of the type its `type` writes. An outcome's `spec`, and a
parser's table, are valid specs that live unchanged for `'p`, the table ended by a spec of
kind `CLI_OPT_TYPE_NONE`. A parser's arguments are `args_len` live non-null strings in an
array only the parser reaches. The getters' "by its handle's adoption" now refers to that
section. The reproducer adopts at `'static` a spec whose name lives shorter, so it no longer
honours the contract.

The regression test is `tests/ub_unsafe/src/cli/opt.rs`. It adopts each handle, shared and
exclusive, within the extended contract, then reads every getter and writes through the
exclusive ones. `an_adopted_spec_lends_its_strings_for_their_lifetime` is this scenario
adopted at its strings' lifetime. They pass plain, under ASan/UBSan and under Miri
(`run-fixed.log`).
