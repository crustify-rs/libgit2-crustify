# A macro whose expansions no binding reaches is checked against its body

Status: `landed` as crustify `9780804` (record `58a4115`) on `libgit2-crustify/devel`, with the user's
approval on 2026-11-14, which its protected path needs: the approved `e99130a` rebased, its
file the same in the commit and its parent.

Goal: Equivalence: its tests compare the Rust with the C.

## Signal

An agent had to work out a rule by hand that a skill should have stated. A translation batch
and then its review had to settle it, and the review's task message had to raise it.

## Evidence

`testing`'s Equivalence tests section compares a port with its C, and "a C function the raw
bindings do not reach, one with internal linkage, is compared through the functions that
call it". A header macro has no symbol, so the C can be run only where it expands, inside
the functions that expand it. When the raw bindings reach none of those functions, the
section says nothing about what stands in for the comparison.

- `wave-200/batch-0/run-1` ported `mac::MIN` (`deps/ntlmclient/compat.h`). Its only
  expansions are in `ntlm.c`'s static `des_key_from_password` and `generate_lm_hash`. These
  are reached only through `ntlm_client_response`, which the raw bindings do not declare
  (`grep ntlm_client sys/libgit2` finds nothing), and no suite reaches them (`ntlm.c` is at
  `0.00`% in every suite). The batch wrote no `equiv` test. Its six `unit_pub` tests check
  the function against the macro's replacement list, `(((x) < (y)) ? (x) : (y))`, at
  `ntlm.c`'s three sites, ties and unordered pairs. Its commit message argues the choice.
- Its module pass, `wave-200/batch-1/run-1`, was asked by its task message to "judge both
  against `porting-symbols`' Macros section and `testing`". It found no rule to judge them
  by. It accepted the batch's reading: an equivalence test needs a Rust side that
  translates the C it runs, and the functions that expand `MIN` have none until `ntlm.c` is
  ported. A test reaching them through new bindings would compare DES and OpenSSL, not the
  macro.
- `wave-225/batch-1/run-1` (`src/util/strnlen.h`) shows the other case. `p_strnlen`'s
  expansion in `git__strndup` is reached through `StrFree::dup_prefix`, so its `equiv`
  tests compare it there.

## Destination

The spec: `testing`'s Equivalence tests section, which the reviewer role evolves.

## Change

Append to the paragraph that ends "is compared through the functions that call it":

> A macro is compared where it expands, through the functions that hold its expansions.
> When the raw bindings reach none of them, no equivalence test compares it: its `unit_pub`
> tests check it against the replacement list at each expansion site's arguments, and the
> batch reports the sites as unreached, until the port of the module that holds them
> compares them.

Its record entry's recurrence signal: a task message, report or plan correction that settles
again what stands in for a macro's equivalence tests.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a macro that implements logic is compared where it expands; one whose expansions no function the raw bindings reach runs has no equivalence test, is checked by unit tests against its replacement list, and is reported as unreached until its sites' module is ported | `libgit2-crustify`, `equiv-unreachable-macro-sites` | crustify `9780804` | two runs settled by hand what stands in for a macro's equivalence tests when the bindings reach none of its expansion sites | a task message, report, commit message, dashboard note or plan correction that settles again what stands in for a macro's equivalence tests | watched |

## Validation

The two batches above, read under the sentence. `wave-200/batch-0`'s tests and report meet
it as they stand: unit tests over the replacement list at `ntlm.c`'s sites, and the sites
reported unreached in its dashboard notes. `wave-225/batch-0` falls under the sentence's
first half and keeps its `equiv` tests. No run was repeated, since neither batch failed.

## Review

None yet.

## Review of the change

`review-1` (Opus 5.5, `$1.44`), on `73b731e`: **rejected**, three fixes, made as `e99130a`. "Reach" had two
senses, and in the preceding sentence's sense the text exempted any macro whose expansions sit
in internal functions, though bound callers run them: it now says no function the raw
bindings reach runs any expansion, directly or through what it calls. `unit_pub` clashed with
where tests of private items go, and is "unit tests". "A macro" covered constant macros, and
is "a macro that implements logic". The rule now carries its reason. With the first fix it
weakens no check: it puts the comparison off to the port of the module that holds the sites.

`review-2` (Opus 5.5, `$0.41`), on `e99130a`: **approved**; it weakens no check. It edits `testing`, a
protected path, and waits for the user's approval.
