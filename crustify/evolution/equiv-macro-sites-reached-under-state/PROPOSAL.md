# A macro's expansion site counts as reached whatever state decides its result

Status: `open`.

Goal: Equivalence, its measure "the inputs and states on which the Rust is compared with the C
it replaces, with no difference left unexplained": a site the rule counts as unreached is
compared on no input, and its difference goes unexplained.

## Signal

A landed row's recurrence signal: `equiv-unreachable-macro-sites` (crustify `9780804`), "a task
message, report, commit message, dashboard note or plan correction that settles again what
stands in for a macro's equivalence tests". Two runs of one module settled it again:

- `wave-220/batch-0/run-1`, the port of `src/util/ctype_compat.h`'s eight macros, compared
  three of them where the C expands them and reported the other five's sites as unreached,
  by a criterion of its own: "the other macros' sites take no single byte from a bound
  function" (its dashboard note, and the module docs of `tests/equiv/src/util/ctype_compat_h.rs`).
  Functions the raw bindings reach run all five: `git_path_is_gitfile` runs `git__tolower`
  (`path.c`'s `next_hfs_char`), and `git_diff_from_buffer` runs `git__isxdigit`
  (`patch_parse.c`'s `parse_header_oid`) and `git__isdigit` (`parse.c`'s
  `git_parse_advance_digit`), each deciding a public result by one byte.
- `wave-220/batch-1/run-1`, its module pass, compared those three there (three equivalence
  tests, each failing on a mutant of its port), and met the two left: `git__toupper` and
  `git__isalpha` expand in `date.c`'s parser, which `git_signature_default_from_env` (a date
  from the environment) and `git_revparse_single` (a reflog selector) run, and `git__isprint`
  in `smart_pkt.c`'s `parse_len`, which a fetch over a smart transport runs. Each is reached;
  a byte decides the result only with state the test sets around the call, a child's
  environment, a repository's reflog or a local remote, which neither run set up. The pass
  settled it by hand once more: the replacement-list comparison stands in, and the sites are
  named as not compared, with why.

The rule has two cases, a site some bound function runs, compared there, and no such site,
checked against the replacement list; it does not say that a site run only under such state
is in the first, nor what a run that does not compare one records.

## Evidence

`wave-220/batch-0`'s commit `a3f4f5958` and its dashboard note; `wave-220/batch-1`'s tests
`to_lower_matches_git_path_is_gitfile`, `is_xdigit_matches_git_diff_from_buffer_index_ids` and
`is_digit_matches_git_diff_from_buffer_hunk_headers`, and its rewritten module docs of
`tests/equiv/src/util/ctype_compat_h.rs`. The campaign's equivalence tests already set such
state: `tests/equiv/src/libgit2/signature.rs` runs each case in a child process with its
environment (`isolated`), and `crustify/scripts/http-gate.sh` drives the HTTP client over a
local remote.

## Destination

The spec: `testing`'s Equivalence tests section (a protected path).

## Change

After "A macro that implements logic is compared where it expands, through the functions
that hold its expansions, since a test of their callers compares the callers' logic, not the
macro's.", add:

> A site counts as reached when a function the raw bindings reach runs it, whatever decides
> its result there: an argument, the environment, a repository's state or a remote. Its
> comparison sets that state, in a child process for the environment and over a local
> remote for a transport. A site a run leaves uncompared is named in its notes, with the
> functions that reach it and why it was left, and the macro's comparison with its
> replacement list does not stand in for it.

## Validation

Read against the two runs: under the change, `wave-220/batch-0`'s five sites would have counted
as reached, and the three its module pass compared show that the comparison is cheap where an
argument decides the result. The two left would be named, as the module pass names them, and
stay open for the review that sets up their state. No run was repeated.

Whether it weakens a check: it does not; it narrows what may be reported as unreached.

## Record

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a macro's expansion site counts as reached whatever state decides its result there, and is compared under that state; a site left uncompared is named, and the replacement list does not stand in for it | `libgit2-crustify`, `equiv-macro-sites-reached-under-state` | — | `equiv-unreachable-macro-sites`'s recurrence: two runs of `ctype_compat.h` settled again which sites count as reached | a task message, report, commit message, dashboard note or plan correction that settles again which of a macro's expansion sites an equivalence test must compare | — |

## Review

None yet.
