# The switch's measurements, on `plain`, gen7 beside gen8

- `run.sh`: batch 6's `bench/run.sh` (`wave-226/batch-6/run-1/bench/`, with its `pair.c`),
  its "before" generation `crustify/builds.gen7` (`BEFORE_GEN`). It was run as
  `run.sh builds.gen8/c builds.gen8/plain builds.gen8/plain/crustify/liblibgit2_crustify.a <out> 6 50000000`.
  The output is `results.txt`, in ns per pair.
- `timeit.py`: wall time, and the user and system CPU of every thread, from `wait4`. The
  output is `workload.txt`: five interleaved rounds of `git2 clone file://<bare copy of fork/main>`
  and `git2 blame src/util/thread.h` in the clone.
