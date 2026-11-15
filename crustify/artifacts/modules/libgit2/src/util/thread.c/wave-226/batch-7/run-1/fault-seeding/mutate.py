#!/usr/bin/env python3
"""Seed one fault at a time in src/libgit2/src/util/thread.rs of a scratch tree and run the
module's tests over each, by binary, with the module's export feature on, as the switch
compiles it.

    mutate.py <scratch tree> <mutants.json> <out.json> [ids...]

Each mutant is {"id", "kind", "old", "new"}: `old` must occur exactly once in the pristine
file. Each binary's thread tests run alone under a timeout; a test that fails, or a binary
that times out or aborts, catches the mutant. The equivalence tests (the `equiv` binary and
the gateway's `equiv_priv`) retire at the switch; every other test outlives the C.
"""
import json
import os
import re
import subprocess
import sys
import time

tree, mutants_path, out_path = (os.path.abspath(a) for a in sys.argv[1:4])
only = set(sys.argv[4:])
src = os.path.join(tree, "src/libgit2/src/util/thread.rs")
pristine = open(src).read()
mutants = json.load(open(mutants_path))
runner = [os.path.join(tree, "crustify/scripts/cargo-variant.sh"), "plain", "test",
          "--features", "ffi-export-src-util-thread-c"]
binaries = {"lib": ["--lib"], "unit_pub": ["--test", "unit_pub"], "ub_safe": ["--test", "ub_safe"],
            "ub_unsafe": ["--test", "ub_unsafe"], "equiv": ["--test", "equiv"]}
results = json.load(open(out_path)) if os.path.exists(out_path) else {}


def run(args, timeout):
    try:
        p = subprocess.run(args, cwd=tree, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout + p.stderr
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or b"").decode(errors="replace") + (e.stderr or b"").decode(errors="replace")
        return "timeout", out


try:
    for m in mutants:
        if only and m["id"] not in only:
            continue
        if m["id"] in results and not only:
            continue
        assert pristine.count(m["old"]) == 1, (m["id"], pristine.count(m["old"]))
        open(src, "w").write(pristine.replace(m["old"], m["new"]))
        t0 = time.time()
        rc, out = run(runner + [b for v in binaries.values() for b in v] + ["--no-run"], 1800)
        r = {"kind": m["kind"], "desc": m.get("desc", ""), "build": rc}
        if rc != 0:
            r["build_log"] = out[-3000:]
        else:
            for name, sel in binaries.items():
                brc, bout = run(runner + sel + ["--", "util::thread"], 240)
                failed = sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED", bout, re.M)))
                passed = len(re.findall(r"^test \S+ \.\.\. ok", bout, re.M))
                r[name] = {"rc": brc, "failed": failed, "passed": passed}
                if brc != 0 and not failed:
                    r[name]["tail"] = bout[-1500:]
        r["secs"] = round(time.time() - t0)
        results[m["id"]] = r
        json.dump(results, open(out_path, "w"), indent=1)
        print(m["id"], {k: (v["rc"], len(v["failed"])) for k, v in r.items() if isinstance(v, dict)},
              r["build"], flush=True)
finally:
    open(src, "w").write(pristine)
