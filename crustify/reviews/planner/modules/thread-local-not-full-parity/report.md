# `thread_local` was recorded as a full-parity equivalent of src/util/thread.c

## What the earlier plan decided

`modules.json`, module `src/util/thread.c` (libgit2, index 29), `objective.rust_equivalent`:

```json
"std": {"library": ["std::thread", "std::sync::Mutex", "std::sync::RwLock", "std::sync::Condvar",
        "std::sync::atomic", "std::thread_local"], "feature_parity": "partial",
        "notes": "thread_local! keys are static; git_tlsdata keys are created at runtime with a destructor"},
"crates": [{"crate": "thread_local", "feature_parity": "full", "license": "MIT OR Apache-2.0",
            "notes": "per-object thread-local values, as git_tlsdata_* provide"}]
```

Under the `wrap` campaign every module was `wrap` whatever its equivalents, so the entry
decided nothing. Under `port`, a `full` crate makes the module `wrap`.

## Evidence

A crate's feature parity is judged for that crate alone, against everything the module
provides (`specs/artifacts/modules.md`, `rust_equivalent.crates`: "established crates that
could replace the module outright"). src/util/thread.c with its headers provides threads
(`git_thread_create`, `git_thread_join`, `git_thread_exit`, `git_thread_currentid`), mutexes,
read-write locks and condition variables (`git_mutex_*`, `git_rwlock_*`, `git_cond_*`, macros
over pthreads in src/util/unix/pthread.h), atomics (`git_atomic32_*`,
`git_atomic64_*`, `git_atomic_compare_and_swap`) and runtime-created thread-local keys
(`git_tlsdata_init`, `git_tlsdata_set`, `git_tlsdata_get`, `git_tlsdata_dispose`).

The thread_local crate (1.1.8) provides only `ThreadLocal<T>`: `new`, `with_capacity`,
`get`, `get_or`, `get_or_try`, `get_or_default`, `iter`, `iter_mut`, `clear`
(`~/.cargo/registry/src/*/thread_local-1.1.8/src/lib.rs`). It covers the `git_tlsdata_*`
family and nothing else:

```sh
grep -n "pub fn " ~/.cargo/registry/src/*/thread_local-1.1.8/src/lib.rs
grep -n "GIT_INLINE\|^int\|^void" c/src/util/thread.h c/src/util/thread.c c/src/util/unix/pthread.h
```

## Edit

`thread_local` is `partial`, with what it lacks in its notes. No equivalent has full parity,
so under the `port` campaign the module's objective is `port`. Downstream: its functions
(the `git_tlsdata_*`, the `git_atomic32_*`/`git_atomic64_*` inlines, `git_threads_global_init`, ...;
on Linux `git_thread_create`, `git_mutex_*` and the rest are macros over pthreads) are `port` items
of its waves in `schedule.json`, and the `git_thread` struct, whose layout pack-objects.c
uses, is `wrap` where the order places it and `port` after pack-objects.c's integration; no
module or item moves to another module.
