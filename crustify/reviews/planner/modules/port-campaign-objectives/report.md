# Thirteen modules of the port campaign were planned `wrap`

## What the earlier plan decided

Planner run 4 gave each module the objective its `rust_equivalent` suggested: `wrap` when
the standard library or a crate replaces it with full parity, `port` otherwise
(`crustify/tmp/planner-run-4/py/common.py`, `objective()`: `'wrap' if x['foundational'] or
parity_full(x) else 'port'`). Besides the seven foundational modules, thirteen modules of
`libgit2` became `wrap` in `modules.json`, for example:

```json
{"index": 68, "name": "src/util/vector.c", "foundational": false, ...,
 "objective": {"objective": "wrap",
               "rust_equivalent": {"std": {"library": ["alloc::vec::Vec"], "feature_parity": "full", "notes": ""}, "crates": []}}}
```

The thirteen: `deps/ntlmclient/util.c` [11], `src/util/bitvec.h` [24], `src/util/integer.h`
[27], `src/util/staticstr.h` [28], `src/libgit2/strarray.c` [32], `src/util/array.h` [33],
`src/util/hash/collisiondetect.c` [34], `src/util/hash/openssl.c` [35],
`src/util/hashmap_str.h` [37], `src/util/unix/realpath.c` [52], `src/libgit2/oidarray.c`
[56], `src/util/tsort.c` [66] and `src/util/vector.c` [68].

`schedule.json` followed: their todo waves held only `wrap` items, and only those other
modules can reach (statics and enums planned for none), and two types of port modules were
kept wrapped for good because these modules' C was to stay:

- `git_oid` (`src/libgit2/oid.c`), whose layout `src/libgit2/oidarray.c` uses (`sizeof`
  and indexing in `git_oidarray__add`, `__remove`, `__reverse`): wrap only, no `port`;
- `SHA1_CTX` (`src/util/hash/sha1dc/sha1.c`), embedded by value in
  `git_hash_sha1_ctx` (`c/src/util/hash/collisiondetect.h:15-16`): wrap only.

Run 4's `memmap2-not-full-parity` and `thread-local-not-full-parity` reports state the same
rule ("Under `port`, a `full` crate makes the module `wrap`").

## Evidence

`crustify/CRUSTIFY.md` answers `campaign` with `port`. The decomposition spec's Objectives
section sets each module's objective from that answer alone: "in a port campaign, every
module is `port`: it is reimplemented in safe Rust. The foundational modules are the
exception, `wrap` in every campaign". The modules schema (6, `objective`) repeats it: "`port`
in a port campaign and `wrap` in a wrap campaign, and `wrap` for every foundational module in
either". `rust_equivalent` records what a port builds on, not whether the module is
ported: "A port builds on them where they serve: a standard-library operation that is
equivalent is used directly". None of the thirteen is foundational
(`"foundational": false` on each).

```sh
cd /target/libgit2-bindings
git show d0d04711f:crustify/artifacts/modules.json | python3 -c '
import json,sys; m=json.load(sys.stdin)
print([(x["index"],x["name"]) for lu in m["link_units"] for x in lu["modules"]
       if x["objective"]["objective"]=="wrap" and not x["foundational"]])'
grep -n "objective" crustify/tmp/planner-run-4/py/common.py
```

## Edit

- `modules.json`: the thirteen modules' `objective` is `port`; their `rust_equivalent`
  stays, as what each port builds on. The `_comment` states the rule.
- `schedule.json`: the todo waves are re-planned with run 4's own composition
  (`crustify/artifacts/planner/run-5/compose/`, whose `README.md` gives the commands; it
  reproduces run 4's 1,325 todo waves exactly from run 4's inputs) under the corrected objectives. Every item of these modules is `port`,
  statics included, and a struct or union is `port` where the order places it, or `wrap`
  there and `port` in a deferred wave once its later layout users are integrated. No type
  stays wrapped for good: the only C that stays is the foundational modules', and none of
  them uses another module's layout.

| module | run 4's todo waves | run 5's todo waves |
|---|---|---|
| `deps/ntlmclient/util.c` | 210 (2 wrap items) | 1529 (2 items) |
| `src/util/bitvec.h` | 290, 299 (6 wrap items) | 290, 1533, 1564 (7 items) |
| `src/util/integer.h` | 217 (6 wrap items) | 1530 (6 items) |
| `src/util/staticstr.h` | 218 (1 wrap item) | 1531 (1 item) |
| `src/libgit2/strarray.c` | 361 (1 wrap item) | 1539, 1545, 1566 (3 items) |
| `src/util/array.h` | 330, 347 (4 wrap items) | 1534, 1540 (4 items) |
| `src/util/hash/collisiondetect.c` | 332, 349, 366 (7 wrap items) | 1535, 1541, 1546 (7 items) |
| `src/util/hash/openssl.c` | 333, 350, 367 (7 wrap items) | 1536, 1542, 1547 (7 items) |
| `src/util/hashmap_str.h` | 334, 351, 368, 378 (29 wrap items) | 1537, 1543, 1548, 1550, 1565 (31 items) |
| `src/util/unix/realpath.c` | 292 (1 wrap item) | 1532 (1 item) |
| `src/libgit2/oidarray.c` | 479, 486 (6 wrap items) | 479, 1558, 1567 (9 items) |
| `src/util/tsort.c` | 386, 392 (2 wrap items) | 1538, 1544, 1549, 1551-1553 (14 items) |
| `src/util/vector.c` | 407, 417, 426, 434, 439 (31 wrap items) | 417, 1554-1557, 1569 (35 items) |

Waves 290, 417 and 479 keep their indices: their items, the `wrap` of a type whose layout
later modules' C uses (`git_bitvec`, `git_vector`, `git_array_oid_t`), are unchanged, the
type now `wrap` there before its deferred `port`. The deferred `port` waves are 1564 (`git_bitvec`, after
`src/libgit2/pathspec.c`), 1565 (`git_hashmap_str`, `git_hashset_str`), 1566
(`git_strarray`), 1567 (`git_array_oid_t`, `git_oidarray`) and 1569 (`git_vector`). The two
types that were wrapped for good are ported:

- `git_oid`: `port` in wave 1568 of `src/libgit2/oid.c`, after the last modules whose C uses
  its layout (its wrap landed in wave 5);
- `SHA1_CTX`: `port` in wave 1563 of `src/util/hash/sha1dc/sha1.c`, after
  `src/util/hash/collisiondetect.c`'s last wave, 1546 (its wrap stays in wave 264).

Downstream, the waves whose items depend on these modules' units now depend on the new
waves: with the [closed-waves-unplanned-wrap-types](../closed-waves-unplanned-wrap-types/report.md)
correction, 296 todo waves keep their items and indices and change their `depends_on`
alone. The order of the modules, their layers and their dependency records are unchanged:
objectives do not enter the order.

## Questions for the user

None: the settings' `campaign` answer decides it. Porting a module onto `Vec`,
`checked_add` or the `sha1-checked` crate is still a port, as its `rust_equivalent` notes.
