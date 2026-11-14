# The C library's link keeps Rust that the C never calls

Status: `open`. Split from `module-switch-procedure` on 2026-11-14. No change is made yet.

Goal: Performance, The emitted Rust: the emitted Rust costs no more than native Rust would.
Its measure is not taken yet, so this goal is the nearest, not an exact fit.

## Signal

No row of the Signals table fits exactly. The nearest is "cost or time outliers, in the
dashboard", which suggests waste in tooling; the figures here come from `nm`, not from the
dashboard. The evolution skill also accepts "a measured waste" as a proposal's ground.

## Evidence

Met at the second switch, `src/cli/opt.c`'s (`modules/git2/src/cli/opt.c/integration/run-1`,
the fork's `f57e1a616`, dashboard Notes entry `git2/src/cli/opt.c` integration).

**The C library's link pulls Rust it never calls (item 9).** With `opt.rs`'s gateway
compiled, `libgit2.so`, which calls only `utf8.c`'s exports, holds 6337 local Rust functions
where gen1's held 2608, and imports about 75 more glibc functions: the linker takes whole
archive members of the dev-profile `staticlib`, and does not collect unused sections. Its
exported API does not change.

- `nm` of each generation's plain `libgit2.so.1.9.0`: 5878 and 9607 local and global text
  symbols, 2608 and 6337 of them Rust's (`_R`), `cli_opt_parse` in neither; the dynamic
  symbol tables differ only in undefined glibc imports. `-Wl,--gc-sections` on the link that
  takes the Rust library, or a library built with one codegen unit per export, would keep it
  to what the C calls; not tried.
- `deps/llhttp/api.c`'s switch (`modules/libgit2/deps/llhttp/api.c/integration/run-1`):
  gen3's `libgit2.so` defines the same 907 dynamic symbols as gen2's, and holds 6747 local
  Rust functions where gen2's held 6337.

## Destination

From the original's Destination list:

- `bindgen`, Build modes, or the fork's `SelectRust.cmake` (item 9): the C library's link
  collects the Rust it does not call.

Protected: none. `bindgen` (`skills/rust/bindgen/SKILL.md`) is not protected. The fork's
`SelectRust.cmake` is the campaign's own file, not Crustify's: a change there alone would be
the campaign's, and no proposal.

## Change

Not made, and not tried in the campaign.

The change would be one of two, which the original leaves open:

1. The link that takes the Rust library collects unused sections (`-Wl,--gc-sections`).
   `bindgen`, Build modes, or `porting-workflow`'s Builds section, where the first switch
   changes the fork's C build, would say so.
2. The Rust library is built with one codegen unit per export, so that the linker takes
   only the archive members the C references.

Either would be tried first on one variant, and the count of Rust functions in the C
library compared before and after. No script moves with this proposal.

## Validation

None. The original records for item 9 only: "Items 5 to 9 are its evidence, and are not
made." Neither option was tried.

## Review

None yet.
