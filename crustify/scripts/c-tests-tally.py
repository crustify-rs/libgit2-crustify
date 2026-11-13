"""Tally a c-tests.sh run and compare it with the C test baseline.

    python3 -I c-tests-tally.py <out dir> <c-baseline.json> <variant>

Writes <out dir>/tally.json: the cases passed, failed and skipped over every suite's TAP
output, and every suite that failed a case or exited non-zero, with the sanitizer reports
its stderr holds. Exits 0 when the counts and the non-zero suites (binary, suite, exit
status) match the variant's record in the baseline, 1 otherwise, printing each difference.
"""

import glob
import json
import os
import re
import sys


def tally(out):
    ok = notok = skip = 0
    bad = []
    for rc in sorted(glob.glob(f"{out}/suites/*/*.rc")):
        base = rc[:-3]
        tap = open(base + ".tap", errors="replace").read()
        sk = len(re.findall(r"^ok \d+ .*# SKIP", tap, re.M))
        ok += len(re.findall(r"^ok \d+", tap, re.M)) - sk
        skip += sk
        failed = re.findall(r"^not ok \d+ - (\S+)", tap, re.M)
        notok += len(failed)
        code = open(rc).read().strip()
        if code != "0" or failed:
            err = open(base + ".err", errors="replace").read()
            sig = sorted(set(re.findall(
                r"ThreadSanitizer: [\w\- ]+?(?= \(|$)|error: Undefined Behavior"
                r"|AddressSanitizer: \S+|runtime error", err, re.M)))
            bad.append({"binary": os.path.basename(os.path.dirname(base)),
                        "suite": os.path.basename(base), "exit": int(code),
                        "failed_cases": failed, "signals": sig})
    return {"passed": ok, "failed": notok, "skipped": skip,
            "total": ok + notok + skip, "nonzero_suites": bad}


def main():
    out, baseline, variant = sys.argv[1:4]
    result = tally(out)
    with open(f"{out}/tally.json", "w") as f:
        json.dump(result, f, indent=1)
        f.write("\n")
    record = json.load(open(baseline))["variants"].get(variant)
    if record is None:
        print(f"c-tests-tally: the baseline records no {variant!r}")
        return 1
    diffs = []
    for key in ("passed", "failed", "skipped", "total"):
        if result[key] != record[key]:
            diffs.append(f"{key}: {result[key]}, baseline {record[key]}")
    key = lambda s: (s["binary"], s["suite"], s["exit"])
    got = {key(s) for s in result["nonzero_suites"]}
    want = {key(s) for s in record["nonzero_suites"]}
    for s in sorted(got - want):
        diffs.append(f"non-zero suite not in the baseline: {s}")
    for s in sorted(want - got):
        diffs.append(f"baseline's non-zero suite now passes or exits otherwise: {s}")
    print(f"c-tests-tally: {variant}: {result['passed']} passed, {result['failed']} failed, "
          f"{result['skipped']} skipped of {result['total']}; "
          f"{len(result['nonzero_suites'])} non-zero suites")
    for d in diffs:
        print(f"c-tests-tally: {variant}: {d}")
    print(f"c-tests-tally: {variant}: {'matches' if not diffs else 'differs from'} the baseline")
    return 1 if diffs else 0


if __name__ == "__main__":
    sys.exit(main())
