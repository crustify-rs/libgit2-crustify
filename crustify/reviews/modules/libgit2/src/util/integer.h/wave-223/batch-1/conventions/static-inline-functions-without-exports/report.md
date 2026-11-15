class: nit
evidence: reading, against the conventions' Exports section and `porting-symbols`' Static inline functions section; `grep` of the C callers below
conditions: every configuration; `9b43c913d`, the wave's tip before this batch

# integer.h's six `static inline` functions are ported with no export for the C that stays

**Where.** `src/libgit2/src/util/integer_h.rs` as `wave-223/batch-0` landed it (`9b43c913d`).
Its module docs say: "its six `GIT_INLINE` functions and six macros are native Rust here,
reached by no C, so the module has no unsafe regions and no exports". They also say the header stays
"as porting's Macros and Static inline functions sections keep a header's items".

**What is wrong.** `git__is_sizet`, `git__is_ssizet`, `git__is_uint16`, `git__is_uint32`,
`git__is_ulong` and `git__is_int` are `static inline` functions of the module's header. 31
call sites in 19 C files outside the module call them, beside two in `win32/posix_w32.c`,
which this build does not compile. Among those files, `pack.c`, `pack-objects.c` and
`commit_list.c` stay C past the first ship point, and `posix.c`, `futils.c`, `object.c` and
`repository.c` stay C until their own switches:

```
grep -rn 'git__is_\(sizet\|ssizet\|uint16\|uint32\|ulong\|int\)\b' c/src | grep -v integer.h
```

Three rules say what the module owes those callers:

- The conventions' Exports section: a module's exports are "the module's functions and
  globals that C outside the module references, ..., the `static inline` functions its
  headers define included".
- `porting-symbols`' Static inline functions section: "at the module's switch, the header's
  definition is replaced by its prototype, and the remaining C calls the Rust module's
  export, under the function's bare name".
- `porting-workflow`'s One language per module section: "The headers bundled with the module
  stay while remaining C includes them, declaring the module's exports, its `static inline`
  functions included".

The Macros section, which the docs also cite, keeps a header's macros in the header,
unexported. It says nothing of its functions. As landed, the module has no gateway, so its
switch could not replace the six definitions. Past the switch the C would go on compiling
its own copies of the bodies, and the module would stay in both languages. Those copies
include `git__is_ssizet`'s, which accepts every count. The module's Rust corrects that
behaviour, and its docs record the correction.

The batch's task message said "An item the remaining C still expands from the header needs
no export". That sentence is right for the macros. The batch read it as covering the
functions too. The conventions' Export features section also names a feature only by the
module's translation unit, which a header module lacks. Both are filed as the proposal
`crustify/evolution/header-module-exports`.

**Fix.** `integer_h.rs` gains its `ffi_export` gateway. It is compiled under the export
feature `ffi-export-src-util-integer-h`, which `Cargo.toml` declares, named for the header,
and the module drops its `#![forbid(unsafe_code)]`. The six exports keep the C signatures,
each checked against its raw binding in a constant, and none can panic or read memory:

- `git__is_sizet` and `git__is_ulong` give C's answer for every `int64_t`, as
  `fits_usize` and `fits_c_ulong` give it for the `uint64_t` the bits hold. Most of their
  callers pass a `uint64_t` object or blob size through the `int64_t`. The gateway cannot
  tell those callers from the ones that pass a negative `int64_t` (`pack-objects.c`'s
  configured sizes), and that negative acceptance is C's contract for the former.
- `git__is_ssizet` gives `fits_isize`'s answer, the correction the module's docs record, so
  that the guards of `p_read` and `git_futils_readbuffer_fd`, and `p_mmap`'s under `NO_MMAP`,
  refuse a count past `SSIZE_MAX`.
- `git__is_uint16`, `git__is_uint32` and `git__is_int` give C's answer.

The module's docs say what the switch does with the header. Its six definitions become their
prototypes, and its macros stay. The allowlist's note on the six entries says they reach the
exports from the switch on. The region's `equiv_priv` compares each export with the C
function it replaces, through `static_fns.c`'s wrapper, over the bounds of its parameter.
Its `unit_priv` checks each export's range without the C, and Miri runs it. Both compile
only with the feature until the switch.

The switch was rehearsed in scratch (`crustify/artifacts/modules/libgit2/src/util/integer.h/wave-223/batch-1/run-1/switch-rehearsal.md`).
It replaced the header's six definitions with prototypes and built the `plain` variant
with the crate's Rust library linked, gateway included. `libgit2.so` then defined the six
functions from the Rust. The remaining C tests matched the baseline: 3442 passed and 55
skipped of 3497.

**Tiers.** Caught by the module pass (`wave-223/batch-1`), the module's only review. The
batch's own gates (`wave-223/batch-0`) had the code in scope and passed it, since no gate
checks for an export. It is an escape of that tier only in the sense that no gate there could
catch it.
