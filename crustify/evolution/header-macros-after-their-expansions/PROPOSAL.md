# A header macro that implements logic is scheduled after the symbols it expands to

Status: `landed`, change 2 as wavefront `e529c26` on its `devel`, with the user's approval,
and applied to the plan by planner run 12; change 1 declined, as the order now comes from
wavefront's graph without the planner's correction: the scheduling skill's Order section
computes it over the units' dependencies, and wavefront's `docs/graph.md` (Edges) gives a
macro the body edges its replacement list references, as this proposal asks. Validated:
in run 12's schedule of `deps/reftable/basics.c`, `REFTABLE_ALLOC_ARRAY` and
`REFTABLE_FREE_AND_NULL` follow `reftable_alloc_size`, `reftable_malloc` and `reftable_free`
(sublayer 3), `REFTABLE_CALLOC_ARRAY` `reftable_calloc` (4), `REFTABLE_ALLOC_GROW`
`reftable_alloc_grow` (5) and `REFTABLE_ALLOC_GROW_OR_NULL` both (6). The module's waves
had already run (169-175), so no wave of it is re-planned.

## Signal

Failed or rerun work in packing: a batch was handed macros whose bindings need wrappers
that another batch of the same wave writes at the same time.

## Evidence

`wave-169/batch-1` (`deps/reftable/basics.c`, foundational) holds the allocation macros of
`deps/reftable/basics.h`: `REFTABLE_ALLOC_ARRAY` (`reftable_alloc_size` then
`reftable_malloc`), `REFTABLE_CALLOC_ARRAY` (`reftable_calloc`), `REFTABLE_ALLOC_GROW`
(`reftable_alloc_grow`, which calls `reftable_realloc`), `REFTABLE_ALLOC_GROW_OR_NULL`
(`REFTABLE_ALLOC_GROW`, then `REFTABLE_FREE_AND_NULL`) and `REFTABLE_FREE_AND_NULL`
(`reftable_free`). `binding-symbols`' Macros section binds a macro that implements logic as
a function that "reaches C only for the symbols its expansion uses, through their safe
wrappers". Those wrappers are the policy and array owner over `reftable_malloc`,
`reftable_calloc`, `reftable_realloc`, `reftable_alloc_grow` and `reftable_free`, which the
foundational module binds as strategies (Lifecycle strategies) in `wave-169/batch-2`,
running beside batch 1 on the same file.

`schedule.json` put all twelve macros of the module in wave 169, the module's layer 0,
sublayer 0, and the functions they expand to in waves 170 to 174, after them. The macros'
records carry no dependency at all:

```text
$ wavefront c --config ../crustify/wavefront/configs/wavefront-config.json \
    query macros --name REFTABLE_ALLOC_GROW      # composed.depends_on: {syms: [], types: []}
$ wavefront c --config ... query dag --name REFTABLE_ALLOC_GROW --depth 1   # {}
```

That is wavefront's design: `docs/graph.md`'s Edges table gives a body edge to a `.c` macro
only, and "a macro a closure header defines ... stays C's", bound rather than translated.
But binding a header macro that implements logic reimplements its body in Rust, over the
wrappers of what it expands to, so its binding depends on them as a function's does on its
callees.

Batch 1 bound its other items first, `REFTABLE_FREE_AND_NULL` among them, which needs no
reftable type (any owner's drop frees through its own policy). It then waited for batch 2
to land, and bound the four array macros after its rebase, as a constructor and a growth
method of batch 2's array owner and as anchors on two of batch 2's methods. A batch running
beside the owner's batch cannot name it.

## Destination

`scheduling` (Waves and batches; Wrap batches), and wavefront's graph, whose goals the
second change must be judged against.

## Change

1. In `scheduling`, where a wave's items are packed: a macro that implements logic, which
   a binding reimplements, depends on the functions, globals and macros its body
   references, as a function does on its callees; the planner orders it after them, in the
   module's later sublayer, unless it is a constant, a type generator or a symbol alias.
2. In wavefront: give a header macro the body edges a `.c` macro has, to the symbols its
   definition references, in the default graph, or a flag that does, so that the schedule
   orders a logic macro after what it expands to without the planner's correction. A
   macro with no references keeps none, so constants are unaffected.

## Validation

Not validated: the change is to the plan. Rerunning the schedule of
`deps/reftable/basics.c` with the edges should put `REFTABLE_ALLOC_GROW` after
`reftable_alloc_grow`, and `REFTABLE_FREE_AND_NULL` after `reftable_free`.
