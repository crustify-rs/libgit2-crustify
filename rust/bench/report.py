#!/usr/bin/env python3
"""Post-processing for the libgit2 benchmark suite (standard library only).

  report.py json <builddir> <results.tsv>     results.tsv -> JSON on stdout
  report.py compare <outdir> <labelA> <labelB>
      Aggregate the per-round outputs <outdir>/<label>/round<r>-<NN>-<program>.out of two
      builds, print a table (component, workload, A median, B median, ratio
      B/A, ratio of minima, A/B median CPU time, CPU ratio, check), and write <outdir>/compare.tsv and compare.json.
      Exit status 1 if a check value differs between the builds, is
      inconsistent across repetitions/rounds, or a workload is missing.
"""
import glob
import json
import math
import os
import statistics
import sys

FIELDS = ["component", "workload", "reps", "median_ns", "min_ns", "ops", "check", "samples_ns",
          "cpu_median_ns", "cpu_min_ns", "cpu_samples_ns"]


def parse_lines(lines):
    for line in lines:
        line = line.rstrip("\n")
        if not line or line.startswith("#") or line.startswith("component\t"):
            continue
        parts = line.split("\t")
        if len(parts) not in (8, len(FIELDS)):
            continue
        rec = dict(zip(FIELDS, parts))
        for k in ("reps", "median_ns", "min_ns", "ops", "cpu_median_ns", "cpu_min_ns"):
            if k in rec:
                rec[k] = int(rec[k])
        for k in ("samples_ns", "cpu_samples_ns"):
            rec[k] = [int(x) for x in rec.get(k, "").split(",") if x]
        yield rec


def cmd_json(builddir, tsv):
    with open(tsv) as f:
        recs = list(parse_lines(f))
    json.dump({"build": builddir, "results": recs}, sys.stdout, indent=1)
    sys.stdout.write("\n")


def load_build(outdir, label):
    """{(component, workload): {"samples": [...], "checks": set, "ops": n}}"""
    res = {}
    order = []
    paths = glob.glob(os.path.join(outdir, label, "round*-*.out"))
    # program order (round<r>-<NN>-<prog>.out), then round
    paths.sort(key=lambda p: (os.path.basename(p).split("-", 1)[1], os.path.basename(p)))
    for path in paths:
        with open(path) as f:
            for r in parse_lines(f):
                key = (r["component"], r["workload"])
                if key not in res:
                    res[key] = {"samples": [], "cpu": [], "checks": set(), "ops": r["ops"]}
                    order.append(key)
                res[key]["samples"].extend(r["samples_ns"])
                res[key]["cpu"].extend(r["cpu_samples_ns"])
                res[key]["checks"].add(r["check"])
    return res, order


def fmt_ms(ns):
    return "%.3f" % (ns / 1e6)


def cmd_compare(outdir, a, b):
    ra, order_a = load_build(outdir, a)
    rb, order_b = load_build(outdir, b)
    order = order_a + [k for k in order_b if k not in ra]
    rows = []
    bad = 0
    for key in order:
        x, y = ra.get(key), rb.get(key)
        row = {"component": key[0], "workload": key[1]}
        flags = []
        for lab, r in ((a, x), (b, y)):
            if r is None:
                flags.append("MISSING(%s)" % lab)
                continue
            row[lab] = {
                "median_ns": int(statistics.median(r["samples"])),
                "min_ns": min(r["samples"]),
                "n": len(r["samples"]),
                "cpu_median_ns": int(statistics.median(r["cpu"])) if r["cpu"] else None,
                "ops": r["ops"],
                "checks": sorted(r["checks"]),
            }
            if len(r["checks"]) != 1 or "INCONSISTENT" in r["checks"]:
                flags.append("INCONSISTENT(%s)" % lab)
        if x and y:
            row["ratio"] = row[b]["median_ns"] / max(row[a]["median_ns"], 1)
            row["ratio_min"] = row[b]["min_ns"] / max(row[a]["min_ns"], 1)
            if row[a]["cpu_median_ns"] and row[b]["cpu_median_ns"]:
                row["ratio_cpu"] = row[b]["cpu_median_ns"] / row[a]["cpu_median_ns"]
            if x["checks"] != y["checks"]:
                flags.append("CHECK-MISMATCH")
        row["flags"] = flags
        if flags:
            bad += 1
        rows.append(row)

    hdr = ("component", "workload", "%s_median_ms" % a, "%s_median_ms" % b, "ratio",
           "%s_min_ms" % a, "%s_min_ms" % b, "ratio_min",
           "%s_cpu_median_ms" % a, "%s_cpu_median_ms" % b, "ratio_cpu", "check")
    tsv_lines = ["\t".join(hdr)]
    w = max([len(r["workload"]) for r in rows] + [8])

    def get(r, lab, k):
        v = r.get(lab, {}).get(k)
        return fmt_ms(v) if v is not None else "-"

    def rat(r, k):
        return "%.3f" % r[k] if k in r else "-"

    print("%-10s %-*s %11s %11s %6s %6s %11s %11s %6s  %s" % (
        "component", w, "workload", a + " ms", b + " ms", "ratio", "minrat",
        a + " cpu", b + " cpu", "cpurat", "check"))
    for r in rows:
        status = " ".join(r["flags"]) if r["flags"] else "ok " + (r[a]["checks"][0] if a in r else "")
        print("%-10s %-*s %11s %11s %6s %6s %11s %11s %6s  %s" % (
            r["component"], w, r["workload"], get(r, a, "median_ns"), get(r, b, "median_ns"),
            rat(r, "ratio"), rat(r, "ratio_min"), get(r, a, "cpu_median_ns"),
            get(r, b, "cpu_median_ns"), rat(r, "ratio_cpu"), status))
        tsv_lines.append("\t".join([
            r["component"], r["workload"], get(r, a, "median_ns"), get(r, b, "median_ns"),
            rat(r, "ratio"), get(r, a, "min_ns"), get(r, b, "min_ns"), rat(r, "ratio_min"),
            get(r, a, "cpu_median_ns"), get(r, b, "cpu_median_ns"), rat(r, "ratio_cpu"), status]))

    ratios = [r["ratio"] for r in rows if "ratio" in r]
    summary = {}
    if ratios:
        logs = [math.log(x) for x in ratios]
        summary = {
            "workloads": len(rows),
            "geomean_ratio": math.exp(sum(logs) / len(logs)),
            "max_abs_dev": max(abs(x - 1) for x in ratios),
            "median_abs_dev": statistics.median(abs(x - 1) for x in ratios),
            "within_5pct": sum(1 for x in ratios if abs(x - 1) <= 0.05),
            "within_10pct": sum(1 for x in ratios if abs(x - 1) <= 0.10),
            "flagged": bad,
        }
        cr = [r["ratio_cpu"] for r in rows if "ratio_cpu" in r]
        if cr:
            summary["cpu_geomean_ratio"] = math.exp(sum(math.log(v) for v in cr) / len(cr))
            summary["cpu_median_abs_dev"] = statistics.median(abs(v - 1) for v in cr)
            summary["cpu_max_abs_dev"] = max(abs(v - 1) for v in cr)
            summary["cpu_within_5pct"] = sum(1 for v in cr if abs(v - 1) <= 0.05)
            summary["cpu_within_10pct"] = sum(1 for v in cr if abs(v - 1) <= 0.10)
        print()
        print("wall time, %d workloads: geomean ratio %.3f, median |ratio-1| %.3f, max |ratio-1| %.3f, "
              "within 5%%: %d, within 10%%: %d, flagged: %d" % (
                  len(rows), summary["geomean_ratio"], summary["median_abs_dev"],
                  summary["max_abs_dev"], summary["within_5pct"], summary["within_10pct"], bad))
        if "cpu_geomean_ratio" in summary:
            print("cpu time: geomean ratio %.3f, median |ratio-1| %.3f, max |ratio-1| %.3f, "
                  "within 5%%: %d, within 10%%: %d" % (
                      summary["cpu_geomean_ratio"], summary["cpu_median_abs_dev"],
                      summary["cpu_max_abs_dev"], summary["cpu_within_5pct"],
                      summary["cpu_within_10pct"]))
    with open(os.path.join(outdir, "compare.tsv"), "w") as f:
        f.write("\n".join(tsv_lines) + "\n")
    with open(os.path.join(outdir, "compare.json"), "w") as f:
        json.dump({"a": a, "b": b, "summary": summary, "rows": rows}, f, indent=1)
    return 1 if bad else 0


def main(argv):
    if len(argv) == 4 and argv[1] == "json":
        cmd_json(argv[2], argv[3])
        return 0
    if len(argv) == 5 and argv[1] == "compare":
        return cmd_compare(argv[2], argv[3], argv[4])
    sys.stderr.write(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
