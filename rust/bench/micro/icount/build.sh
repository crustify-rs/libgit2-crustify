#!/bin/bash
# Instruction / syscall counting without perf(1) (perf WP).
#
#   ./build.sh                      build the tools and every harness against
#                                   $C_BUILD (default /work/build-c) and
#                                   $RUST_BUILD (default /work/builds/perf)
#   ./icount ./ic_buf_rs 100        user-mode instructions between the
#                                   harness's raise(SIGSTOP) markers (ptrace
#                                   single-step; deterministic, load-free)
#   REGION=1 ./iprof ./ic_buf_rs    the same, plus a per-address histogram
#                                   (prof.N.txt, maps.N.txt) for:
#   python3 -I resolve.py prof.1.txt maps.1.txt [top]   per function
#   python3 -I lines.py prof.1.txt maps.1.txt libgit2.so <fn substring>   per source line
#   python3 -I annot.py prof.1.txt maps.1.txt libgit2.so '<symbol>'      per instruction
#   ./sccount ./sc_fs_rs | ./scname.sh                  syscalls per region
#   taskset -c 3 ./tm_buf_c         CPU time per call (minimum of repetitions)
#
# Single-stepping runs at about 1M instructions/s: keep regions small.
set -e
cd "$(dirname "$0")"
C_BUILD=${C_BUILD:-/work/build-c}
RUST_BUILD=${RUST_BUILD:-/work/builds/perf}
SRC=$(cd ../../../.. && pwd)
for t in icount iprof sccount; do clang -O2 -o $t $t.c; done
for f in ic_*.c tm_*.c sc_*.c; do
	b=${f%.c}
	for v in c:$C_BUILD rs:$RUST_BUILD; do
		clang -O2 -g -I"$SRC/include" -include unistd.h "$f" -o "${b}_${v%%:*}" -L"${v#*:}" -lgit2 -Wl,-rpath,"${v#*:}"
	done
done
