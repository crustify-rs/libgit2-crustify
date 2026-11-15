# A forked child's `exit` waits forever on the thread-local keys' registry lock

class: behaviour
evidence: the reproducer's failing exit, with the hung child's kernel wait channel (`futex_wait_queue`, syscall 202, `futex`); the same scenario over C's POSIX keys ends every child
conditions: Linux and glibc, any features: `TlsKey` reaches it now, and the exports under `ffi-export-src-util-thread-c` would from the module's switch. Reproduces on `d3d90f565`, the wave's tip, which batch 3 landed

- **finding**: `logic/forked-child-exit-waits-on-the-key-registry`
- **batch**: `wave-226/batch-5/run-1`, the review of `wave-226/batch-3` (`d3d90f565`)
- **items**: `mac::git_tlsdata_key` (`TlsKey`), `fn::git_tlsdata_init`, `_set`, `_get` and
  `_dispose`, and their exports
- **where**: `src/libgit2/src/util/thread.rs`, `mod tls`: `impl Drop for Table`, which locked
  `REGISTRY` to look up each value's destructor, and `Table::prune`, which locked it too
- **tiers**: caught by this review batch; `wave-226/batch-3`'s own gates had it in scope and
  passed it

## Finding

Each thread keeps its thread-local values in a `thread_local!` table. The table's `Drop` runs
the keys' destructors as the thread exits. Before it ran any, it locked the process-wide
registry of keys, a `std::sync::Mutex`, to find each value's destructor, whenever the thread
held a value. glibc runs a thread's `thread_local!` destructors at `exit()` too: for the main
thread, and for the one thread of a forked child.

A child forked from a threaded process holds a copy of every lock as it stood at the `fork`,
and no thread to release one. Where another thread of the parent held the registry's lock at
that moment, which it does while it makes or disposes of a key, stores a first value, or exits
itself holding values, the child's `exit()` waits on the lock forever.

libgit2 meets this case. `git_process_start` (`src/util/unix/process.c`) forks, and its child
calls `exit(0)` when `chdir` or `execve` fails. After the module's switch, `errors.c` keeps
each thread's error state under a key of this module, so the forking thread holds a value:
any thread that has set or read an error does. The parent then blocks too: it reads the
child's status pipe, whose write end the hung child keeps open. C's POSIX keys take no lock,
and glibc's `exit()` runs no `pthread_key_create` destructor, so with the C's keys the child
ends. POSIX lets a forked child of a threaded process call only async-signal-safe functions,
which `exit` is not: libgit2's `exit(0)` departs from that, and `_exit` would not. The C
itself still ends, though. The Rust's lock is what turns that `exit` into a hang.

## Evidence

`src/main.rs` here stores a value under a `TlsKey` on the main thread while a second thread
makes and disposes of keys. It then forks 100 times, and each child calls `exit(0)`. A child
that has not ended within two seconds is killed, and the program exits 1. From the repository
root:

```text
CARGO_TARGET_DIR=crustify/tmp/b5/target-repro crustify/scripts/cargo-variant.sh plain \
  run -q --manifest-path crustify/reviews/modules/libgit2/src/util/thread.c/wave-226/batch-5/logic/forked-child-exit-waits-on-the-key-registry/Cargo.toml
```

On `d3d90f565` it exits 1 (`run-affected.log`):

```text
fork 2 of 100: the child did not end within 2 s, blocked in futex_wait_queue (syscall 202)
```

The scratch probes this report rests on ran 40 forks a scenario, on the tip's tree. They forked
from a thread holding a Rust value: 36 and 37 children of 40 hung, in two runs, and 186 of
200 in a third. They forked from a thread holding a C value of C's POSIX keys, through the raw
bindings of `thread.c`, while another thread made and deleted POSIX keys: 0 of 40 hung. They
forked from a thread holding no value, beside the same churn of keys: 0 of 40 hung. So the hang
needs the table's destructor, and the lock another thread holds.

## Fix

In this batch's second commit, `mod tls` keeps each key's state in an `Arc<Key>`: its id, its
kind, its destructor, and a `live` flag that disposing of the key clears. The registry maps the
live ids to it. A thread's first value of a key takes the registry's lock once, to clone the
key's `Arc` into the value's entry. From then on, the table needs the lock no more:

- its `Drop` runs each live key's destructor from the entry, and drops the other values;
- `prune` drops the values of disposed keys by their flags; and
- `get_value` checks the flag where the table is borrowed.

Disposing of a key removes it from the registry and clears its flag under the lock, as
before. `DISPOSALS`, which the table's prune polled to know when to look, is gone. The
behaviour the tests check is unchanged, the race between a key's disposal and a thread's exit
included, which C's POSIX keys have too. The module documentation says that reading,
replacing and a thread's exit take no lock, and why. With the fix the reproducer prints
`100 forked children ended` and exits 0 (`run-fixed.log`).

## Tests

`a_forked_child_exits_while_other_threads_make_keys`, a `unit_priv` test in `ffi_import`,
beside the tests of C's keys, since it forks and takes C's keys. A thread makes and disposes of
C's and Rust's keys while the test's thread holds a value of each, the C one with a C
destructor, which ends a forked child with `_exit(0)` once it runs, so that no atexit handler
follows it. ASan's leak check is one such handler: in a child that lacks the parent's other
threads, it reports their memory as leaked. The test forks 40 children, each calling `exit(0)`,
and asserts that none is still running after two seconds. On the tip's `tls` module it fails,
with 36 of 40 hung, after 72 s; with the fix it passes in under a second, plain, under
ASan+UBSan and under TSan. Under Miri it is ignored (`forks`), and was seen to stop at `fork`.
`a_thread_that_calls_exit_destroys_its_values`, beside it, holds what the fix keeps: a forked
child's `exit` runs C's destructor on the value its thread inherited.
