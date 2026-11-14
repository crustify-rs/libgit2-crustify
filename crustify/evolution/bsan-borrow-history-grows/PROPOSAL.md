# BorrowSanitizer's cost per access to one allocation grows with its borrow history

Status: `superseded` by `bsan-sized-tests` (crustify `c18c8a8`), on 2026-11-14: a test BSan cannot
finish is sized for it, which answers the campaign's side. What remains asks nothing of
Crustify: a report to BorrowSanitizer upstream, the user's to send, with `reproducer/` here.

## Signal

A tool that reports what the source contradicts, here in time rather than in a report: a
loop of constant work per iteration slows down as it runs, under BorrowSanitizer alone.

## Evidence

`wave-225/batch-1/run-1`, the module pass of `src/util/strnlen.h`, ran the module's end
gates on the wave tip `7ef895eb2`. Under BSan (`bsan 0.1.0 (de6ede0149 2026-10-07)`, debug
test builds, `BSAN_OPTIONS=gc_hang_threshold_ms=120000`), `unit_pub` and `equiv` each ran
every test but one to completion, and timed out at 5400 s on the module's two exhaustive
tests, `src::util::strnlen_h::every_nul_position_in_every_bound` and
`src::util::strnlen_h::every_nul_position_matches_the_c` (logs
`crustify/tmp/gates/tip/bsan-{unit_pub,equiv}.log` of that worktree). Each sweeps 8 word
offsets by 2,556 runs of a slice of one 80-byte stack buffer, and passes in 0.01 s plain.

Alone under BSan, the `unit_pub` test kept one core busy and its resident memory grew by
about 8 MB/s (415 MB at 20 s, 1.06 GB at 100 s). The reproducer here, the same loop with no
dependency, run with `cargo bsan run -- <starts> [fresh]` from its directory:

| run | offset 0 | offset 1 |
|---|---:|---:|
| debug, one buffer reused (`-- 2`) | 90.2 s | 420.4 s |
| debug, a buffer per offset (`-- 2 fresh`) | 104.8 s | 103.2 s |
| release, one offset (`--release -- 1`) | 1.6 s | — |

Each offset does the same work, so the reused buffer's growth from 90 s to 420 s is BSan
state that accumulates with the borrows made of one allocation and is not collected while
it lives. Even a fresh buffer costs about 40 ms per run in debug.

## Destination

A tool: BorrowSanitizer, upstream, which this campaign does not evolve. The user reports it
with the reproducer, if they choose to. The campaign's side of it is
`crustify/evolution/bsan-sized-tests`, a rule of `testing-gates` for tests BSan cannot finish.

## Change

None in the campaign. Upstream: collect a live allocation's borrow history, or keep its
per-access cost bounded as the history grows.

## Validation

The reproducer's table above, on this host. The module's two tests now give each offset its
own buffer and sweep bounds up to 24 bytes under `cfg(bsan)`, and finish under BSan.
