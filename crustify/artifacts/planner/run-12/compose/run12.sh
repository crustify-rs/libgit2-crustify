#!/bin/sh
# Planner run 12's composition, from the worktree's root, with wavefront e529c26 and the
# campaign's CodeQL tables re-extracted under it (t2/macro_references.csv). The inputs go
# under the ignored crustify/tmp/planner-run-12/. The scripts are run 11's, their paths
# moved to this worktree, but for: write12.py (review batches in the started waves, the
# comment, carries_over from carryover12.py), carryover12.py (new: run 11's carryover11.py,
# the assistant's per-item split of 9237f3a18 and its end of the C suite of 5d23cc8e9, and
# the schema's whole if_still_in_c), check12.py (review batches, carries_over), counts12.py
# (this run's counts and the first ship point's waves), counters12.py (skips a comment
# replacement already made) and deps12.py (new: each entity's used_by from this schedule).
# decomp8.py is not run: the decomposition is unchanged, and config8.py reproduces the
# committed configuration from it byte for byte.
set -e
R=$(pwd); T=$R/crustify/tmp/planner-run-12; C=crustify/artifacts/planner/run-12/compose
mkdir -p $T
python3 -I $C/config8.py                 # the configuration, as committed (git diff: none)
python3 -I -c 'import json; c=json.load(open("crustify/wavefront/configs/wavefront-config.json")); c.pop("schedule_seeds"); json.dump(c, open("crustify/tmp/planner-run-12/config-noseeds.json","w"), indent=1)'
wavefront $R/c --config $R/crustify/wavefront/configs/wavefront-config.json schedule --output $T/schedule-port.json
wavefront $R/c --config $T/config-noseeds.json schedule --output $T/schedule-deps.json
python3 -I $C/empty11.py                 # empty.json (as run 11's)
python3 -I $C/testfacts8.py $T/schedule-deps.json
python3 -I $C/ties11.py                  # ties.json, ties-why.json (as run 11's)
/opt/venv/bin/python $C/users8.py        # users.json
python3 -I $C/enums8.py                  # enums.json (as run 11's)
/opt/venv/bin/python -I $C/order8.py     # order.json
python3 -I $C/counters12.py              # modules.json's tests, depends_on, counters (unchanged)
python3 -I $C/stages11.py                # the verdicts, for reading
python3 -I $C/write12.py                 # schedule.v12.json, wave-map.json, carryover.json
python3 -I $C/carryover12.py             # the carry-over counts
python3 -I $C/check12.py                 # OK
python3 -I $C/counts12.py                # the commit message's counts
python3 -I $C/deps12.py                  # dependencies.json's used_by
cp $T/schedule.v12.json crustify/artifacts/schedule.json
