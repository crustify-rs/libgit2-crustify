class: nit
evidence: the script and its recorded results contradict the lesson's description
conditions: `crustify/scripts/http-gate.sh` at `0c1bd7a0d` and `46055d7c3`; `crustify/artifacts/modules/libgit2/deps/llhttp/http.c/integration/run-1/http-gate-results.txt`

# The provisional lesson `http-exports-gated-through-a-local-remote` miscounts the gate's failing clones

**Where.** `crustify/LESSONS.md`, `http-exports-gated-through-a-local-remote`, its rule: "It
runs a variant's `git2 clone` of two repositories through each framing, three clones that
fail, and `libgit2_tests …`".

**What is wrong.** `http-gate.sh` has run four failing clones since it was added
(`testrepo-close`, `testrepo-malformed`, `missing`, `unauthorised`), and its successful clones
and `libgit2_tests` runs go through three framings, `length`, `chunked` and `chunked-ext`, not
"each framing": `close` and `malformed` are framings of the failing clones. A switch that
followed the rule as written would expect three failing-clone lines and find four. The rest
of the entry holds: no offline C test drives the HTTP client (the module pass's finding);
the procedure (old generation beside new on plain, ASan+UBSan and TSan, `results.txt`
compared with `diff`, exports called counted on the coverage variant) is what both switches
ran; its general form is the open proposal `switch-gate-beyond-c-tests`, which answers the
generality test; and its scope names the modules whose exports only the HTTP client reaches.

**Reproduce.**

```
for c in 0c1bd7a0d 46055d7c3; do git show $c:crustify/scripts/http-gate.sh | grep -c '^clone .* fail$'; done   # 4, 4
grep -c '^clone .* rc=128' crustify/artifacts/modules/libgit2/deps/llhttp/http.c/integration/run-1/http-gate-results.txt   # 4
```

**Fix.** The rule names the three framings libgit2 reads to their end and the four failing
clones, and says `libgit2_tests` runs through each of the three; the evidence cites the
check. Changed, the entry stays `provisional`, for the next review batch.

**Tiers.** Caught by this review batch, the first to read the lesson; no earlier tier had it.
