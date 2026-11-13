# A macro whose expansions no binding reaches is checked against its body

Status: `open`.

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

## Validation

The two batches above, read under the sentence. `wave-200/batch-0`'s tests and report meet
it as they stand: unit tests over the replacement list at `ntlm.c`'s sites, and the sites
reported unreached in its dashboard notes. `wave-225/batch-0` falls under the sentence's
first half and keeps its `equiv` tests. No run was repeated, since neither batch failed.

## Review

None yet.
