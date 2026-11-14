# llhttp.c's equivalence tests call api.c functions that have no export, unlinking `equiv` at the switch

- **finding**: `misc/llhttp-equiv-tests-call-api-c-functions-without-exports`
- **batch**: `wave-194/batch-7/run-1`, `deps/llhttp/api.c`'s module pass
- **items**: the 13 api.c functions ported onto `llhttp.rs`'s accessors and maps and left
  unexported, since no C outside api.c calls them: `fn::llhttp_get_type`,
  `fn::llhttp_get_http_major`, `fn::llhttp_get_http_minor`, `fn::llhttp_get_method`,
  `fn::llhttp_get_status_code`, `fn::llhttp_get_upgrade`, `fn::llhttp_get_errno`,
  `fn::llhttp_errno_name`, `fn::llhttp_method_name`, `fn::llhttp_status_name`, with
  `fn::llhttp_settings_init`; and the tests of `tests/equiv/deps/llhttp/llhttp.rs`
  (`wave-194/batch-0`, `1ead111ee`, and `wave-194/batch-1`, `2c69341c3`) that call them
- **affected commit**: `9124cc5a6`
- **files**: `tests/equiv/deps/llhttp/llhttp.rs`, `tests/equiv/deps/llhttp/api.rs`

## Finding

At api.c's switch, `api.c.o` leaves the variants' archives, and the 31 exports of
`ffi-export-deps-llhttp-api-c` are all that define api.c's symbols. The batches before this
one checked that with archive copies lacking `api.c.o`, and left `equiv` out "by design": its
api.c comparisons call api.c's C and retire with it. But `equiv` is one binary, for every
module of the crate, and not only `tests/equiv/deps/llhttp/api.rs` calls api.c.
`tests/equiv/deps/llhttp/llhttp.rs`, the equivalence tests of `llhttp.c`, which stays C, reads
both its parsers through api.c's getters (`c_fields`), checks the setters through
`llhttp_get_type` and `llhttp_get_upgrade`, compares the maps with api.c's three name
functions, and builds a table with `llhttp_settings_init`. None of those has an export. So once
only `tests/equiv/deps/llhttp/api.rs` is retired, `equiv` does not link, and with it neither
`llhttp.c`'s surviving equivalence tests (`llhttp__internal_init` and
`llhttp__internal_execute`, which stay) nor any other module's run.

## Evidence

On `9124cc5a6`, with `crustify/tmp/linkcheck/plain` a copy of the `plain` variant's build
whose archive lacks `api.c.o` (each entry of `crustify/builds/plain` symlinked but
`crustify/libcrustify.a`, copied and then `ar d crustify/libcrustify.a api.c.o`, 211
objects), from the repository root:

```
CRUSTIFY_BUILD_DIR=$PWD/crustify/tmp/linkcheck/plain CARGO_TARGET_DIR=$PWD/target/linkcheck-plain \
crustify/scripts/cargo-variant.sh plain test --features ffi-export-deps-llhttp-api-c --test equiv --no-run
```

`rust-lld` stops at its limit of 20 undefined symbols (`link-affected.log`); 10 of the
references it lists are `tests/equiv/deps/llhttp/llhttp.rs`'s:

```text
rust-lld: error: undefined symbol: llhttp_get_type
          >>> referenced by llhttp.rs:78 (tests/equiv/deps/llhttp/llhttp.rs:78)
          >>> referenced by llhttp.rs:324 (tests/equiv/deps/llhttp/llhttp.rs:324)
rust-lld: error: undefined symbol: llhttp_settings_init
          >>> referenced by api.rs:202 (tests/equiv/deps/llhttp/api.rs:202)
          >>> referenced by llhttp.rs:250 (tests/equiv/deps/llhttp/llhttp.rs:250)
…
rust-lld: error: too many errors emitted, stopping now (use --error-limit=0 to see all errors)
```

The three name functions, which the limit hides, are the other three of the file's 23
unexported symbols. Every other suite already links against the exports (batch 6's check).

## Fix

In this batch's second commit, the nine tests that compare `llhttp.rs`'s accessors and maps
with api.c's getters and name functions move, unchanged, from
`tests/equiv/deps/llhttp/llhttp.rs` to an inline module `getters` of
`tests/equiv/deps/llhttp/api.rs`: they test api.c's ported items, whose anchors are
`Replaces fn::llhttp_get_*`, so they are api.c's and retire with its C. The file
`llhttp.rs` keeps the tests that compare `llhttp.c`'s Rust with `llhttp.c`'s C, and calls no
api.c function. Retiring `tests/equiv/deps/llhttp/api.rs` alone, and its `mod api;`, is then
api.c's whole equivalence retirement. In a scratch tree with that file retired, the workspace
links against the exports and passes on `plain` (`equiv` 532), and on ASan+UBSan and TSan but
for the known nightly failures.

## What the retiring tests established, and which unit test keeps it

Retiring `tests/equiv/deps/llhttp/api.rs` removes 21 `equiv` tests. `unit_pub` keeps, in
`tests/unit_pub/deps/llhttp/api.rs` unless named otherwise:

| retiring `equiv` test | kept by |
|---|---|
| `new_leaves_the_table_as_llhttp_settings_init_does` | `new_and_default_are_empty_tables`, `a_zeroed_table_is_empty` |
| `the_setters_fill_the_table_as_c_fills_its_fields` | `each_setter_sets_its_own_slot_and_none_clears_it`, `a_setter_replaces_what_its_slot_held_and_leaves_the_others`, `a_slot_read_back_calls_the_handler_it_was_set_to` |
| `set_settings_leaves_the_parser_as_llhttp_init_does` | `set_settings_stores_the_tables_address_and_none_clears_it`, `init_starts_a_used_parser_afresh_with_its_type_and_table`, `reset_keeps_the_type_table_and_lenient_flags_and_clears_the_rest` |
| `a_table_given_by_set_settings_runs_as_one_given_by_llhttp_init` | `a_response_runs_its_slots_in_order`, `a_chunked_request_and_the_next_run_their_slots_in_order` |
| `pause_and_the_resumptions_move_the_parser_as_c_does` | `pause_pauses_a_parser_without_an_error`, `pause_keeps_an_error_and_its_reason`, `resume_clears_only_a_pause_and_keeps_the_reason`, `resume_after_upgrade_clears_only_an_upgrades_pause` |
| `execute_reads_runs_between_pauses_as_llhttp_execute_does` | `execute_reads_a_message_in_runs_as_parse_does`, `a_paused_parser_reads_nothing_until_resumed`, `an_upgrade_resumes_after_resume_after_upgrade`, `pause_keeps_an_error_and_its_reason` |
| `the_lenient_setters_set_the_flags_as_c_does` | `each_lenient_setter_sets_and_clears_its_own_bit`, `the_lenient_setters_together_set_every_flag`, `unit_priv`'s `each_lenient_flag_fits_lenient_flags` |
| `set_reason_stores_the_string_as_llhttp_set_error_reason_does` | `unit_pub/deps/llhttp/llhttp.rs`'s `each_setter_writes_what_its_getter_reads` |
| `init_leaves_the_parser_as_llhttp_init_does` | `init_starts_a_used_parser_afresh_with_its_type_and_table`; `reinit`'s whole state, `equiv/deps/llhttp/llhttp.rs`'s `new_and_reinit_leave_the_parser_as_llhttp_internal_init_does`, which stays |
| `reset_leaves_the_parser_as_llhttp_reset_does` | `reset_keeps_the_type_table_and_lenient_flags_and_clears_the_rest`, `reset_keeps_the_type_a_parser_of_both_kinds_settled_on` |
| `finish_finishes_the_parser_as_llhttp_finish_does` | the seven `finish_*` tests, and the gateway's `llhttp_finish_returns_cs_codes` |
| `the_dispatchers_call_the_parsers_table_as_api_cs_do` | `each_dispatcher_calls_its_own_slot_with_its_run`, `a_dispatcher_returns_0_without_a_table_or_a_slot`, `a_span_dispatchers_minus_one_is_hpe_user_with_its_slots_reason`, `a_dispatcher_hands_back_any_other_value_unchanged`; the exports' names, the gateway's new `each_dispatcher_export_calls_its_own_slot` |
| `getters::a_response_read_in_pieces_reads_alike`, `getters::a_status_code_llhttp_status_does_not_name_reads_alike` | new: `the_getters_read_what_a_response_read_in_pieces_leaves` |
| `getters::requests_read_their_method_and_upgrade_alike` | new: `the_getters_read_a_requests_method_and_upgrade` |
| `getters::an_invalid_message_reports_its_error_alike_and_keeps_it` | new: `an_error_is_located_in_the_slices_it_is_counted_from` |
| `getters::a_callbacks_own_error_reads_as_unnamed_alike` | new: `a_callbacks_own_error_is_kept_and_read_as_unnamed` |
| `getters::setting_the_error_pauses_and_resumes_as_c_does` | `unit_pub/deps/llhttp/llhttp.rs`'s `a_paused_parser_reads_nothing_until_resumed` |
| `getters::the_lenient_flags_set_through_rust_relax_the_c_parser` | `a_lenient_setter_relaxes_its_check` |
| `getters::the_setters_write_what_api_c_reads` | new: `the_type_and_upgrade_set_are_what_the_getters_read` |
| `getters::the_maps_give_what_api_cs_name_functions_return` | `unit_pub/deps/llhttp/llhttp.rs`'s `errno_map_lists_every_errno_as_hpe_constants` and the three `*_names_*` tests; the maps' text, `equiv/deps/llhttp/llhttp.rs`'s `the_maps_are_llhttp_hs_x_macros_entry_for_entry`, which stays |

Not kept, and none needed once api.c's C is gone: that each operation leaves the parser's
private state (`_index`, `_span_pos0`, `_current`, `header_state`) byte for byte as C's
`llhttp_init`, `llhttp_reset`, `llhttp_finish` and the pause calls did, from each of six
states; and that the lenient setters read any nonzero `int` as true, which their `bool`
replaced and no export takes.
