# wavefront lists no pointer parameter declared as an array

Status: `open`.

## Signal

A tool that reports what the source contradicts. C adjusts a parameter declared as an array
of `T` to a pointer to `T` (C11 6.7.6.3p7): `void ubc_check(const uint32_t W[80], uint32_t
dvmask[1])` takes two pointers. wavefront's function records list neither among their
`ptr_args`, and `query --update` then refuses their findings, so the analysis of such a
parameter cannot be submitted to the ownership store: its nullability, its cardinality (an
array of 80, an out-parameter of one) and its borrow, which the binding or the port shapes
its Rust from, are recorded nowhere.

## Evidence

`wave-221/batch-1/run-1` (`src/util/hash/sha1dc/ubc_check.c`), on the campaign's CodeQL
database, wavefront `bf21e16`:

```bash
wavefront "$PWD/c" --config ../crustify/wavefront/configs/wavefront-config.json \
  query functions --name ubc_check SHA1DCFinal sha1_compression_states
```

| function | `type` | `ptr_args` |
|---|---|---|
| `ubc_check` | `void ubc_check(const uint32_t[80], uint32_t[1])` | none |
| `SHA1DCFinal` | `int SHA1DCFinal(unsigned char[20], SHA1_CTX *)` | `ctx` only |
| `sha1_compression_states` | `void sha1_compression_states(uint32_t[5], const uint32_t[16], uint32_t[80], uint32_t[80][5])` | none |

Submitting `ptr_args` for `W` and `dvmask` (`array.by_val`, borrowed from themselves, not
nullable, `dvmask` mutable) was rejected: `ptr_args: no pointer arg named 'W' in
'ubc_check'; expected one of []`. The batch submitted `lifetime: null` alone, and records the
two parameters' analysis in its dashboard notes. The cause is `src/pointers.qll`'s
`reachesPointer`, which holds for a `PointerType`, a function-pointer type, and through
qualifiers and typedefs, but not for the `ArrayType` CodeQL gives a parameter's declared
type; `sha1.c`'s port batches, which hold every function of the table above but
`ubc_check`, will meet it again.

## Goal

Verification, and Equivalence: every pointer the C passes is analysed and its finding kept,
so that a binding or a port is shaped from recorded facts. Fitness: the pointer parameters a
batch's analysis finds that the ownership store cannot hold, zero once fixed.

## Destination

A tool: wavefront, its pointer queries (`src/pointers.qll`, `src/edges/function_pointer_args.ql`)
and its tests, with a fixture holding array parameters of one and two dimensions, `const`
and not, and through a typedef of an array type.

## Change

Not made: the defect does not block this batch, whose port reads the C directly. A
parameter (a `Parameter`'s declared type, not a field's or a global's, whose arrays are
storage) whose unspecified type is an `ArrayType` counts as a pointer to its element type,
at one level more than the element's depth, so that `uint32_t states[80][5]` is a pointer to
`uint32_t[5]`, and the composer lists it in `ptr_args` with its declared size in the record.

## Record

The row for `EVOLUTION.md`, copied in when the change lands.

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| a parameter declared as an array is a pointer parameter in wavefront's records | `libgit2-crustify`, `wavefront-array-parameters-are-pointers` | — | `ubc_check`'s and `SHA1DCFinal`'s array parameters were missing from `ptr_args`, and their findings refused | a function record whose `ptr_args` leaves out a parameter C adjusts to a pointer | watched |

## Validation

The reproducer above, before the change: the three records as tabled. After it, each lists
every array parameter, and the rejected submission is accepted.

## Review

Pending.
