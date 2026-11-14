# A switch checks its static archive by the removed unit's symbols, not by member names

Status: `open`. Split from `module-switch-procedure` on 2026-11-14. No change is made yet.

Goal: Verification. What a switch claims about its archive is checked by a deterministic
command whose answer is not ambiguous.

## Signal

"One procedure repeated by hand: the same script written again, the same queries run in
sequence": each switch compared the archive's object count and its member list by hand. At
the fourth switch that comparison could not say which object had left.

## Evidence

**An archive's member names are not unique (item 15).** Met at the fourth switch,
`deps/llhttp/http.c`'s (`modules/libgit2/deps/llhttp/http.c/integration/run-1`, the fork's
`230c9dbdb`, dashboard Notes entry `libgit2/deps/llhttp/http.c` integration).
`static-archive.sh` archives the objects `libgit2.so` links by their file names, and
`libcrustify.a` held two `http.c.o`, llhttp's and the HTTP transport's
(`src/libgit2/transports/http.c`). The count the earlier switches compared (210 to 209) and
the diff of `ar t` (one `http.c.o` fewer) do not say which left: `nm -A` over the archive
does, the member left defining `git_smart_subtransport_http` and no `llhttp_*` symbol. The
module pass's link check had to remove the member by position (`ar dN 2`). The archive's
check would be unambiguous by object path, as `build.ninja` names them
(`deps/llhttp/CMakeFiles/llhttp.dir/http.c.o`), or by the symbols the removed TU defined,
which the switch checks anyway.

- `ar t` of gen4's and gen5's plain `libcrustify.a` differ in one line, an `http.c.o`;
  `nm -A` of gen5's shows its one `http.c.o` defining `git_smart_subtransport_http`, where
  gen4's second also defined the three hooks and both predicates.

What the earlier switches compared:

- `src/cli/opt.c`'s switch (`crustify/builds.gen2`): each archive holding 211 objects where
  gen1's held 212, `opt.c.o` gone, and each `git2` defining the five `cli_opt_*` exports as
  global Rust symbols (gen1's were the C's local ones).
- `deps/llhttp/api.c`'s switch (`crustify/builds.gen3`): each archive holding 210 objects
  where gen2's held 211, `api.c.o` gone. In every variant, `libgit2.so`, `git2` and
  `libgit2_tests` define the 31 exports (`addr2line` puts `llhttp_execute` at `api.rs:1634`)
  and none of api.c's 23 unexported functions.
- `deps/llhttp/http.c`'s switch (`crustify/builds.gen5`): each archive holding 209 objects
  where gen4's held 210. In every variant, `libgit2.so`, `git2` and `libgit2_tests` define
  the four exports (`addr2line` puts `llhttp_should_keep_alive` at `http.rs:545`) and no
  `llhttp_message_needs_eof`.

## Destination

From the original's Destination list:

- `testing-gates`, Regressions (item 15): a switch checks its archive by the symbols the
  removed TU defined, or by object path, not by member names.

Protected: all of it. `testing-gates` (`skills/verification/testing/gates/SKILL.md`) is under
`skills/verification/**`.

The original names no script here. `static-archive.sh` writes the members by file name; the
original does not ask to change it. A module pass that removes a member from a private copy
of the archive meets the same ambiguity, and `reviewing` (protected) or the campaign's
lesson on the link check would follow; the original does not list either.

## Change

Not made.

`testing-gates`, Regressions, would state how a switch checks each variant's static archive
and link units:

1. No member of the archive defines a symbol that the removed translation unit defined. The
   command lists each member's defined symbols (`nm -A`).
2. Each link unit defines every export of the module, from the Rust, and defines none of the
   module's functions that had no export.
3. The object count and the member list are not the check. Two translation units in
   different directories can share a file name, so a member name does not identify an
   object. Where an object must be named, it is named by its path in the build system's
   files.

No script moves with this proposal. `static-archive.sh` could later name members by object
path; the original only says the check "would be unambiguous by object path".

## Validation

Copied from the original:

- `deps/llhttp/http.c`'s switch: "Items 14, 15 and 17 are evidence, and are not made."

The `nm -A` check was run by hand at that switch, as the Evidence records.

## Review

None yet.
