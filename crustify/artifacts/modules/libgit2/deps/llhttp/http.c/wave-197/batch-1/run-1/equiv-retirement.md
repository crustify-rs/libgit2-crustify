# What `deps/llhttp/http.c`'s equivalence tests established, and which unit tests keep it

At the module's switch, `tests/equiv/deps/llhttp/http.rs` and its `mod http;` in
`tests/equiv/deps/llhttp.rs` retire with http.c's C: its six tests call that C. This module
pass (`wave-197/batch-1/run-1`) maps each to the tests that keep what it established, in
`tests/unit_pub/deps/llhttp/http.rs` unless named otherwise. Tests marked (new) are this
pass's. `the_headers_of_*`, `a_skipped_body_is_empty_at_the_messages_end`,
`after_message_complete_clears_the_flags_for_the_next_message` and the `http_*` version tests
parse real messages through `llhttp.c`'s generated parser. From the switch on, that parser
calls the four exports itself, so these tests also check the exports through the C state
machine that relies on their return codes. The gateway's `unit_priv` tests in
`src/libgit2/deps/llhttp/http.rs` call each export as C calls it.

| retiring `equiv` test | what it compared | kept by |
|---|---|---|
| `message_needs_eof_is_llhttp_message_needs_eof` | every flag word (the unused bits `0x100` and `0x8000` included), four types (an unnamed one included) and 16 status codes | `a_request_never_needs_eof` (every flag word of a request); `a_response_needs_eof_without_a_length_or_a_final_chunked` (a response's and an unsettled parser's flags, the unused bits (new)); `the_headers_of_responses_decide_their_bodies` (`needs_eof` at 100 to 103, 199 (new), 101 with and without upgrade headers (new), 200 with each framing, 204, 304, and 304 with a length (new)) |
| `should_keep_alive_is_llhttp_should_keep_alive` | every flag word, four types, six statuses and seven versions: 0.9, 1.0, 1.1, 0.0, 0.1, 2.0, 2.1 | `http_0_0_keeps_alive_only_with_keep_alive` (0.0); `http_0_9_keeps_alive_only_with_keep_alive` (new: a minor above zero with a major of zero); `http_1_0_keeps_alive_only_with_keep_alive` (new: `close` ignored before 1.1); `http_1_1_keeps_alive_unless_it_closes` (new: `close` wins over `keep-alive`); `http_2_0_is_read_as_an_earlier_version_for_keep_alive` (new: the kept upstream departure, named); the `keep_alive` column of `the_headers_of_responses_decide_their_bodies` and `the_headers_of_requests_decide_their_bodies` (a body that ends with the input ends the connection) |
| `before_headers_complete_is_llhttps_hook` | 16 flag words, four types, 16 statuses, five methods, upgrade bytes 0, 1 and 2 | `before_headers_complete_on_a_zeroed_parser` (both headers, either alone, each type, the upgrade before); the `upgrade` column of `the_headers_of_*` (`CONNECT` with and without a body (new), an upgrading request, a 101 with upgrade headers, a 200 announcing them, a 101 without them (new)); `Seen::of`'s check that deciding again leaves the upgrade as it was |
| `after_headers_complete_is_llhttps_hook` | 32 flag words, four types, 16 statuses, three methods, three lengths (0, 1, `u64::MAX`), eight lenient words, three upgrade bytes | `after_headers_complete_on_a_zeroed_parser` (16 states, each code, the refusal and both leniencies); `after_headers_complete_on_a_zeroed_parser_s_rarer_states` (new: a skipped chunked body upgrading at once, an unsettled parser never refused); `the_headers_of_responses_decide_their_bodies` and `the_headers_of_requests_decide_their_bodies` (every code from real headers, the refusal and its leniency); `an_upgrade_pauses_the_parse_where_after_headers_complete_says_so`; `a_skipped_body_is_empty_at_the_messages_end`; `unit_priv::headers_code_is_the_code_the_parser_switches_on` (the six codes) |
| `after_message_complete_is_llhttps_hook` | every flag word, four types, four statuses, three versions, three finish bytes, the whole parser after the call | `after_message_complete_settles_keep_alive_then_clears_the_message` (the return, flags cleared, `HTTP_FINISH_SAFE`, upgrade, type and lenient flags kept); `after_message_complete_clears_the_flags_for_the_next_message` (a pipelined message read afresh; on a parsed message, the status, version, length, type and error kept (new)); the `http_*` version tests (its return for each version (new)); the gateway's `unit_priv::after_message_complete_returns_keep_alive_and_clears_the_message` (the export's `int`) |
| `the_hooks_in_sequence_are_llhttps` | seven messages through the three hooks and both predicates, the whole parsers compared after each step | the real-message tests above, in which the parser runs the hooks in sequence and the callbacks ask the predicates between them; `ub_safe`'s `the_hooks_run_inside_their_parsers_parse` |

What no unit test keeps, and none needs once the C is gone:

- the differential itself: each call left C's parser and the Rust's byte for byte alike,
  their private fields included (`_index`, `_current`, `header_state`, `error_pos` and the
  span state). The port writes only `upgrade`, `finish` and `flags`, through their setters;
- the bytes no safe path stores and `llhttp.c` never stores: an unnamed type (7), an unnamed
  method (`0xFF`) and an upgrade byte of 2. The setters take `ParserType` and `bool`;
- status codes and lengths that fall in a class a kept case already represents: 1, 99, 104,
  203, 205, 404 and 999, and a `Content-Length` of `u64::MAX`.
