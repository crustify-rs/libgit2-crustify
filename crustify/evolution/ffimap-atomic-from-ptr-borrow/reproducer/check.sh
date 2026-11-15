#!/bin/bash
# Scans this crate with ffimap and expects both borrows counted: exits 1 when
# `raw_ptr.casted_to.std.borrow` counts fewer than 2.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
out=$(mktemp)
CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$here/target} ffimap --sites raw_ptr.casted_to.std.borrow -- "$here" > "$out"
python3 -I - "$out" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
n = d["counts"]["raw_ptr"]["casted_to"]["std"]["borrow"]
print("raw_ptr.casted_to.std.borrow =", json.dumps(n), "sites:", json.dumps(d["sites"]["raw_ptr.casted_to.std.borrow"]))
total = n if isinstance(n, int) else n.get("total", n)
sys.exit(0 if total >= 2 else 1)
PY
