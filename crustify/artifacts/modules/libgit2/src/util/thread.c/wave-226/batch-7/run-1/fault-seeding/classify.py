#!/usr/bin/env python3
"""Tally mutate.py's results: which faults the tests that outlive the C catch, and which only
the equivalence tests (the `equiv` binary and the gateway's `equiv_priv`) catch."""
import json
import sys

res = json.load(open(sys.argv[1]))
rows, stay_n, equiv_n, both_missed, only_equiv = [], 0, 0, [], []
for mid, r in res.items():
    if r["build"] != 0:
        rows.append((mid, "build failed", "", ""))
        continue
    stay, equiv = [], []
    for b in ("lib", "unit_pub", "ub_safe", "ub_unsafe", "equiv"):
        x = r[b]
        for t in x["failed"]:
            (equiv if b == "equiv" or "::equiv_priv::" in t else stay).append(f"{b}:{t}")
        if x["rc"] not in (0, 101) or (x["rc"] == 101 and not x["failed"]):
            (equiv if b == "equiv" else stay).append(f"{b}:<{x['rc']}>")
    stay_n += bool(stay)
    equiv_n += bool(equiv)
    if not stay and equiv:
        only_equiv.append(mid)
    if not stay and not equiv:
        both_missed.append(mid)
    rows.append((mid, r["desc"], len(stay), len(equiv)))
print(f"{len(res)} faults: outliving tests catch {stay_n}, equivalence tests catch {equiv_n}")
print("caught by the equivalence tests alone:", only_equiv)
print("caught by none:", both_missed)
if "-v" in sys.argv:
    for row in rows:
        print(*row, sep=" | ")
