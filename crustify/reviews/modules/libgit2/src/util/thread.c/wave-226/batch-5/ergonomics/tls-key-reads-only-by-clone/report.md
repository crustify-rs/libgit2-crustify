# A thread-local value can only be read by cloning it

class: nit
evidence: reading; `TlsKey::<Vec<u8>>::get` needs `Vec<u8>: Clone` and copies the vector, and `TlsKey::<T>::get` does not compile for a `T` that is not `Clone` (E0599)
conditions: any; on `d3d90f565`

- **finding**: `ergonomics/tls-key-reads-only-by-clone`
- **batch**: `wave-226/batch-5/run-1`, the review of `wave-226/batch-3` (`d3d90f565`)
- **items**: `mac::git_tlsdata_key` (`TlsKey<T>`), `fn::git_tlsdata_get` (`TlsKey::get`)
- **where**: `src/libgit2/src/util/thread.rs`, `impl<T: 'static> TlsKey<T>`
- **tiers**: caught by this review batch; `wave-226/batch-3`'s own gates had it in scope and
  passed it. The key's safe API is fixed by this, its first landing, so its ergonomics are
  reviewed here.

## Finding

`TlsKey::get` returns a clone of this thread's value, `Option<T>` for `T: Clone`, and nothing
else reads one. A value whose type is not `Clone` can be stored but never read, other than by
`set` taking it out. A value that is `Clone` is copied whole on every read. That is the shape
of the values C keeps under its keys, which Rust would hold under a `TlsKey`: `errors.c`'s
error state, a message buffer and the last error, and `oid.c`'s formatting buffer. Reading
one would copy the buffer, or wrap it in an `Rc` only to read it. The standard library's
`LocalKey::with` lends a thread-local to a closure, which C's `git_tlsdata_get` stands closer
to: it hands back the pointer, not a copy.

## Fix

In this batch's second commit, `TlsKey::with(self, f: impl FnOnce(Option<&T>) -> R) -> R`
lends the value to `f`, or `None` where `get` gives `None`, with neither a clone nor a move.
It reaches the table as `get` does, through `get_value`: `f` runs while the table is
borrowed, so it may read any key's value, and a store from it fails with
`TlsError::Unavailable`, as `get`'s `Clone` does. `get` is unchanged. The method has a doctest.
It is an addition, which `ffimap api` counts as extending `TlsKey`.

## Tests

`tests/unit_pub/src/util/thread/tlsdata.rs`: `with_lends_the_value_where_get_would_clone_it`
stores a value that is not `Clone` and lends it. It reads another key's value inside `with`,
and finds a store from inside refused. It checks that the value stays in place, and that
another thread, a removed value and a disposed key each lend `None`.
