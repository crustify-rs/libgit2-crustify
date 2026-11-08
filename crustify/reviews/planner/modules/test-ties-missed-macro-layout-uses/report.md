# C tests that use a type's layout through a macro, tied to no owner

## What the earlier plan decided

Planner runs 4 to 6 tied a test to the module whose types' layout it uses. They found
those uses with `crustify/tmp/planner/py/tests_gate.py`, which read `t2/local_type_uses.csv`
and kept a local variable only when the source line declares it by the type's own name:

```python
    if r['use_kind']=='local_var':
        ...
        if not re.search(r'\b'+re.escape(r['type_name'])+r'\s+(const\s+)?[A-Za-z_]\w*\s*(\[|=|;|,)',l): continue
```

A test that initializes a type by value inside another type's initializer macro names
no such variable, so these uses were missed. For example, `git_clone_options opts =
GIT_CLONE_OPTIONS_INIT;` initializes `git_checkout_options`, `git_fetch_options`,
`git_remote_callbacks` and `git_proxy_options` by value, each in the macro's nested
braces. The same holds for a `git_strarray` member set by a designated initializer, and
for a struct a test header embeds by value. For each of 61 such test files, its owner's
`tests` left it out. Some examples, as the earlier `modules.json` listed them:

- `src/libgit2/proxy.c` listed 4 test files, not `tests/libgit2/clone/nonetwork.c`, `clone/local.c` or
  `clone/empty.c`;
- `src/libgit2/checkout.c` listed no `clone/local.c`, `clone/transport.c` or `network/fetchlocal.c`;
- `src/libgit2/strarray.c` listed no `apply/both.c`, `apply/check.c` or `apply/fromdiff.c`.

## The evidence against it

The Bundling rule ties a test to each module whose types' layout it uses, "macro
expansions included". Wavefront 2a95e3c's `t2/layout_uses.csv` records each by-value use
with its kind (`value`, `field`, `size`, `index`, `embed`), macro expansions included.
That table did not exist when run 4 tied the tests.

```sh
cd /target/libgit2-bindings
grep '"tests/libgit2/clone/nonetwork.c"' crustify/wavefront/codeql/t2/layout_uses.csv | grep git_proxy_options | head -3
python3 -I crustify/artifacts/planner/run-7/compose/testfacts.py    # internals: layout:<type>:<kind>
python3 -I crustify/artifacts/planner/run-7/compose/ties.py
```

The ties the earlier tables missed, by owner, with the use that makes them:

| module | test files | use |
|---|---:|---|
| `src/libgit2/strarray.c` | 20 | `git_strarray` by value |
| `src/libgit2/proxy.c` | 17 | `git_proxy_options` by value, in `GIT_FETCH_OPTIONS_INIT` and the initializers that embed it |
| `src/libgit2/checkout.c` | 12 | `git_checkout_options` by value, in `GIT_CLONE_OPTIONS_INIT`, `GIT_SUBMODULE_UPDATE_OPTIONS_INIT`, ... |
| `src/libgit2/oid.c` | 3 | `git_oid` by value, embedded and sized |
| `src/libgit2/remote.c` | 3 | `git_fetch_options` and `git_remote_callbacks` by value |
| `src/libgit2/merge.c` | 3 | `git_merge_options` by value, in `GIT_REBASE_OPTIONS_INIT` |
| `src/util/vector.c` | 2 | `git_vector` by value and embedded |
| `src/libgit2/odb.c` | 1 | `git_odb_backend` embedded by `tests/libgit2/odb/backend/backend_helpers.h` |

## The edit and what it moves

The tests are re-derived from `t2/layout_uses.csv` along with the rest of their ties,
under the current Bundling rule (`crustify/artifacts/planner/run-7/compose/ties.py`). The
61 ties are in `modules.json`. A tied test leaves with the first of its modules to be
integrated, so the `port/unexport` stages of functions these tests call are re-planned
with the rest. These ties are internals ties, so they change no layout stage: a test's
use of a module's internals is a use within that module, so it never keeps a type
wrapped.

## Questions for the user

None.
