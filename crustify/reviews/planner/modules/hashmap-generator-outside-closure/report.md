# `src/util/hashmap.h`, home of the type generator GIT_HASHMAP_STRUCT, was in no module

## What the earlier plan decided

Runs 4 to 7 listed `src/util/hashmap.h` in no module of `modules.json`, so the
configuration's closure, which holds the files of every module, left it out. Their
schedule's comment states that "every function, global, callback, enum and type generator
is port where the order places it", and the T3 table run 4 wrote names the header's macro
as the generator of 25 instance types (`crustify/wavefront/t3/type_generators.csv`):

```csv
GIT_HASHMAP_STRUCT,src/util/hashmap.h,type,git_attr_cache_filemap,src/libgit2/attrcache.c
GIT_HASHMAP_STRUCT,src/util/hashmap.h,type,git_cache_oidmap,src/libgit2/cache.h
...
```

## Evidence

Outside the closure, the generator is an external macro to wavefront, never a unit, so no
earlier schedule held it, nor the header's other macros (GIT_HASHSET_STRUCT, the
GIT_HASHMAP_FUNCTIONS and GIT_HASHSET_FUNCTIONS that write every instance's functions,
GIT_HASHMAP_INIT, GIT_HASHMAP_ITER_INIT, ...) or its typedef `git_hashmap_iter_t`, which
53 units use:

```sh
cd /target/libgit2-bindings
git show 1aa1ee98e:crustify/wavefront/configs/wavefront-config.json | grep -c '"src/util/hashmap.h"'   # 0
git show 1aa1ee98e:crustify/artifacts/schedule.json | grep -c GIT_HASHMAP_STRUCT                       # 0
grep -c '^GIT_HASHMAP_STRUCT,src/util/hashmap.h,type' crustify/wavefront/t3/type_generators.csv      # 25
grep '"src/util/hashmap.h"' crustify/wavefront/codeql/t1/macros.csv crustify/wavefront/codeql/t1/types.csv | wc -l
```

The header implements the generic open-addressing map and set that no translation unit
implements; `src/util/hashmap_str.h`, already a header module, instantiates it for string
keys, and `src/libgit2/hashmap_oid.h` for oid keys.

## Edit and what it moves

`src/util/hashmap.h` is a header module of its own, `libgit2` module 196, owned whole
(`crustify/artifacts/planner/run-8/compose/decomp8.py`), and the closure holds it
(`config8.py`). Its 31 API macros and its typedef are port items of its waves
(GIT_HASHMAP_STRUCT, with its 25 instances in `type_generator_for`, in the module's first
wave); GIT_HASHMAP_INLINE and GIT_HASHMAP_EMPTY are attribute macros, not planned. The 25
instance types and the hashmap_str.h and hashmap_oid.h macros now depend on its waves, and
`tests/util/hashmap.c`, which instantiates the map through GIT_HASHMAP_STR_SETUP, is tied to
it as well as to hashmap_str.h (`compose/areas.py`). The module's place in the order and
its waves are this run's: see the commit's re-planned waves.

## Questions

None.
