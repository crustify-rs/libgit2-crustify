# A wrap batch ahead of its consumer lists the fields its consumers access

Status: `open`.

## Signal

Skills that contradict each other (`evolution` §2: a defect in the spec). `binding-types`'
Surface section and `scheduling`'s Wrap batches section give a wrap batch that precedes its
consumers the fields *those consumers* access; the `wavefront` tool skill (Scheduling ›
Waves and batches) fills an item's `field_users` from wavefront's `used_fields`, which in a
default schedule are "the fields the closure's own functions access", the type's own
module's functions included (wavefront `docs/batching.md`, Used fields). For a type wrapped
ahead of its consumers, the worklist therefore names fields only the type's own C touches.

## Evidence

`wave-194/batch-0/run-1` (`deps/llhttp/api.c`, wrapping `deps/llhttp/llhttp.c`'s
`llhttp__internal_s` ahead of api.c's ports). Its `task.json` lists 21 `field_users`, every
field of the struct. `wavefront query structs --name llhttp__internal_s --field-touchers`
shows six of them, `_index`, `_span_pos0`, `_span_cb0`, `_current`, `header_state` and
`initial_message_completed`, touched by `deps/llhttp/llhttp.c`'s functions alone: no
function of api.c, http.c, `transports/httpclient.c` or `transports/httpparser.c` reaches
them. Three of them are unsafe to write from outside the generated parser: `_current` and
`_span_cb0` are the state and span callback it jumps through, `_index` an index into its
match tables. The batch bound the other 15, documented why the six have no accessor, and
filled the dashboard's `fields` cell with the 21 assigned.

## Destination

The spec: the `wavefront` tool skill, Scheduling › Wrap batches, which says how a wrap
batch's items are drawn from the schedule.

## Change

In Scheduling › Wrap batches: a wrap batch ahead of its consumer takes as an item's
`field_users` the fields its consumers access, the item's `used_fields` less those that
only functions of the item's own module touch, as `query structs --field-touchers` lists
them; a field the remaining C of another module touches stays. `binding-types`' Surface
section is unchanged: it already states the rule the worklist then matches.

## Validation

None run: a planning change, validated by the next wrap batch ahead whose type has fields
its own module alone touches, whose `field_users` would hold the 15 consumer fields here.
