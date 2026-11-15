class: nit
evidence: failing assertion (the probe below), and git 2.43's `update-index` under `core.protectHFS`
conditions: any target but `GIT_WIN32`, glibc in the C locale; the documentation of `a3f4f5958`

# The port's documentation tells C callers' wider `int`s to be reduced to their low byte

- **finding**: `misc/code-points-reduced-to-their-low-byte`
- **batch**: `wave-220/batch-1/run-1`, the module pass of `libgit2/src/util/ctype_compat.h`,
  its only review. Tier: the module pass; no tier had the code in scope before it.
- **items**: the module documentation of `src/libgit2/src/util/ctype_compat_h.rs`, and
  `tests/unit_pub/src/util/ctype_compat_h.rs`'s `a_c_callers_int_is_its_byte`, for
  `mac::git__tolower` and the other seven macros (`wave-220/batch-0`, `a3f4f5958`).

## Finding

The module's documentation stated the rule for porting a C caller of the eight macros:
"a C caller's `git__isspace(c)` is `is_space(c as u8)`, the cast its own", and the unit test
`a_c_callers_int_is_its_byte` asserted it for an `int` past a byte,
`to_lower(0x141_i32.to_le_bytes()[0]) == b'a'`. For a `char`, a byte or `EOF` the reduction
is the macro's own cast, and both of the header's branches agree on it. For a wider value
they do not, and one caller passes one: `src/libgit2/path.c`'s `next_hfs_char` returns
`git__tolower((int)codepoint)` for a decoded Unicode code point. `GIT_WIN32`'s inline function
leaves U+0147 (`Ň`) as it is; every other build casts it to its low byte, `G`, and folds it to
`g`. A port of `path.c` that follows the documentation, `to_lower(codepoint as u8)`, copies
this build's result, which departs from the code's own comment ("this will only fold
characters in the ASCII range, which is perfectly fine, because the git folder name can only
be composed of ascii characters") and from git, which under `core.protectHFS` refuses the
path `.Git/x` but accepts `.Ňit/x` and `.ŧit/x`.

The departure in the C itself is `path.c`'s, outside this module's workset, and is handed to
that module's review: through `git_path_is_gitfile` with HFS+'s rules, libgit2 takes
`.Ňitmodules` (U+0147, through this build's cast) and `.ŧitmodules` (U+0167, through
`validate_dotgit_hfs_generic`'s `char c`, which truncates the folded code point on every
build) for `.gitmodules`, and `Įgitmodules` (U+012E, whose low byte is `.`) too: it refuses
names git accepts.

No consumer observes the documentation, so the class is `nit`; the C's departure would be a
`behaviour` finding of `path.c`'s.

## Evidence

The probe, run as a test of the `equiv` binary on `plain` (`crustify/scripts/cargo-variant.sh
plain test --test equiv -- zz_probe --nocapture`), called the safe wrapper of the C,
`path::is_gitfile`:

```rust
for (s, label) in [(".\u{147}itmodules", "U+0147"), (".\u{167}itmodules", "U+0167"),
    (".Gitmodules", "G"), (".\u{1E7}itmodules", "U+01E7"), ("\u{12E}gitmodules", "U+012E dot")] {
    let c = CString::new(s).unwrap();
    println!("{label}: hfs={} generic={}",
        is_gitfile(&c, GitFile::Gitmodules, Filesystem::Hfs),
        is_gitfile(&c, GitFile::Gitmodules, Filesystem::Generic));
}
```

```text
U+0147: hfs=true generic=true
U+0167: hfs=true generic=true
G: hfs=true generic=true
U+01E7: hfs=false generic=false
U+012E dot: hfs=true generic=true
```

git 2.43, in a scratch repository with `core.protectHFS` set, `git update-index --add
--cacheinfo 100644,<blob>,<path>`: `.Ňit/x` and `.ŧit/x` accepted, `.Git/x` and `.‌git/x`
(U+200C before `git`) rejected.

## Fix

The module documentation keeps the reduction for a `char`, a byte or `EOF`, and says that a
value wider than a character is not reduced so: a Rust caller classes and maps it only where
it is a byte (`u8::try_from`), and otherwise leaves it in no class and unmapped, as
`GIT_WIN32` does, with `path.c`'s site as the example. The unit test drops its assertion over
`0x141`. The new equivalence test `to_lower_matches_git_path_is_gitfile` compares `to_lower`
with the C at that site for the code points up to `0xFF`, where both branches agree.
