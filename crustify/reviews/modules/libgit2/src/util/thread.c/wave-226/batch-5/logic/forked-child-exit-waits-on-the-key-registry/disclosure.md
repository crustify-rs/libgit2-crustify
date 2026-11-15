- status: DO NOT FILE: the defect is libgit2-crustify's own, in code no release has shipped,
  and the review that found it fixed it in its second commit
- channel: libgit2-crustify's issue tracker (https://github.com/crustify-rs/libgit2-crustify/issues),
  as a public issue: a hang, with no security impact we know of
- upstream: unchecked: the crate is the campaign's own, with no release to rerun on
- title: A forked child hangs in `exit()` when it holds a `TlsKey` value and another thread
  makes or disposes of keys

---

Each thread's `TlsKey` values live in a `thread_local!` table, whose destructor locked the
process-wide key registry, a `std::sync::Mutex`, before it ran the keys' destructors. On glibc
that destructor also runs at `exit()`, including in the one thread of a forked child. When
another thread of the parent held the registry's lock at the moment of `fork`, while making or
disposing of a key, or exiting itself, the child waited on the lock forever. libgit2's
`git_process_start` forks and calls `exit(0)` in the child when `execve` fails. Its parent then
blocks too, on the child's status pipe.

The reproducer stores one value on the main thread while a second thread loops on
`TlsKey::<u8>::new(None)?.dispose()?`. It then forks 100 times, each child calling `exit(0)`:

```text
fork 2 of 100: the child did not end within 2 s, blocked in futex_wait_queue (syscall 202)
```

It ran on `libgit2-crustify` at `d3d90f565`, over its fork of libgit2 at `230c9dbdb`
(`v1.9.0-553`), with rustc 1.100.0 and glibc 2.39, on Linux 6.14.

A fix keeps each key's state, with its destructor and a liveness flag, in an `Arc` that every
stored value holds. Then a thread's exit reads the flag and takes no lock.

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness
auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed
and independently reproduced.
