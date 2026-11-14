# A spec is built one exclusive-handle setter at a time, where C writes one initialiser

- **finding**: `ergonomics/spec-built-one-setter-at-a-time`
- **batch**: `wave-186/batch-3/run-1`, the review of `wave-186/batch-0` (`8a149ac08`) and
  `wave-186/batch-2` (`fe6b85bfa`), `opt.h`'s types
- **items**: `st::cli_opt_spec`
- **affected commit**: `fe6b85bfa`
- **where**: `src/git2/src/cli/opt.rs`, `Spec::new` and `SpecMut`'s setters

## Finding

C declares a command's options as one array of initialisers, each spec where it is
declared, as `cmd_*.c` do (`c/src/cli/cmd_cat_file.c:25`):

```c
static const cli_opt_spec opts[] = {
	CLI_COMMON_OPT,

	{ CLI_OPT_TYPE_SWITCH,     NULL,    't', &display,    DISPLAY_TYPE,
	  CLI_OPT_USAGE_REQUIRED,  NULL,    "display the type of the object" },
	…
	{ 0 },
};
```

The wrap's safe API builds a spec from its `Target` alone (`Spec::new`). Every other field
takes a setter on the exclusive handle, reached through `as_mut()` on a mutable binding, so a
table must be declared `mut` and patched by index after it is built:

```rust
let mut specs = [Spec::new(Target::Bool(Some(&debug))), Spec::zeroed()];
specs[0].as_mut().set_name(Some(c"debug"));
specs[0].as_mut().set_alias(NonZero::new(b'd'));
specs[0].as_mut().set_help(Some(c"show debugging output"));
```

This is `reviewing`'s `ergonomics` objective: no builder where C takes a long argument list.
The spec has eight fields, and its consumers are the module's ported parser, its tests and
the CLI commands a port will bring to Rust, all of which declare tables. The batch's own
tests show the cost. `tests/equiv/src/cli/opt.rs` spent sixteen lines of
`specs[i].as_mut().set_…` on four tables, `tests/ub_safe/src/cli/opt.rs` four lines on two,
each spec's fields set apart from where the spec is declared. The alias also has to be
wrapped as `NonZero::new(b'd')`. The wrap is the first batch to give the struct a safe API,
which its `port/layout` keeps, so its ergonomics are reviewed here.

## Fix

`Spec<'p>` gains by-value builders in the module's native code: `with_name`, `with_alias`,
`with_switch_value`, `with_usage`, `with_value_name` and `with_help`. Each takes the spec and
returns it with one field set through its setter, and each is anchored to its field as its
setter is. `with_alias` takes the byte, `0` leaving the spec without an alias as C's `'\0'`
does. A spec now reads as its C initialiser does:

```rust
let specs = [
    Spec::new(Target::Bool(Some(&debug)))
        .with_name(c"debug")
        .with_alias(b'd')
        .with_help(c"show debugging output"),
    Spec::default(),
];
```

The setters stay, for changing a spec in place. The builders' consumers in the crate follow:
the spec tables of `tests/equiv/src/cli/opt.rs` and `tests/ub_safe/src/cli/opt.rs` are
declared with them, and no longer `mut`. `tests/unit_pub/src/cli/opt.rs`'s
`builders_set_what_the_setters_set` checks that each builder sets what its setter sets, and
that a zero alias sets none. The impl block's doctest builds a table with them. The change
adds API and changes none: `ffimap api diff` against `fe6b85bfa` counts the six fields' keys
(`fld::cli_opt_spec.name`, `.alias`, `.switch_value`, `.usage`, `.value_name`, `.help`) as
`extended`, and every other item of the batch's worklist as `kept`.

No reproducer: an ergonomics finding is shown by the API, not by a failure.
