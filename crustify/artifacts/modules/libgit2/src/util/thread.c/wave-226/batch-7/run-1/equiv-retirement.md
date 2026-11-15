# thread.c's equivalence tests at the switch

What `wave-226/batch-7/run-1`, the module pass, found of each equivalence test of
`src/util/thread.c` before the switch retires it. At the switch the raw bindings of the
twelve header functions and of the four `git_tlsdata_*` reach the module's exports, so each
of these tests would compare the Rust with itself: each retires, and none needs no C, so
none is carried over as it stands. Each row names the tests that outlive the C and keep what
it checks: the `unit_pub`, `ub_safe` and `ub_unsafe` tests, and the `unit_priv` tests of the
module's regions, the gateway's among them, which compile unconditionally from the switch on.

Whether they keep it was checked by seeding faults (`fault-seeding.md`, beside this file):
of 64 faults, one site each, the tests that
outlive the C caught 61 on the wave's tip and catch 63 with the two tests this pass added;
the equivalence tests catch 46, none that the others miss. The one fault nothing catches is
equivalent (a `load` that writes back what it read). So every row below is kept, and the
equivalence tests can retire without losing a check.

## `tests/equiv/src/util/thread/atomics.rs`

| equivalence test | what it compares | kept by |
|---|---|---|
| `atomic32_sequences_match_the_c` | `Atomic32Ref`'s `get`, `set`, `inc`, `dec` and `add` against `git_atomic32_*`, step by step, from `0`, `1`, `-1`, `i32::MAX` and `i32::MIN`: returns and values | `unit_pub` `atomics::atomic32_operations_return_what_they_leave_and_wrap` and the five `atomics::atomic32_*` carried over from `threads/atomic.c`; `thread::arithmetic_wraps_at_the_bounds` |
| `atomic64_sequences_match_the_c` | the same for `Atomic64Ref`'s `get`, `set` and `add` against `git_atomic64_*` | `unit_pub` `atomics::atomic64_operations_return_what_they_leave_and_wrap`, `atomics::atomic_ssize_adds_and_reads_back` and the three `atomics::atomic64_*` |
| `pointer_slot_sequences_match_the_c` | `atomic_compare_and_swap`, `atomic_swap` and `atomic_load` over an `AtomicPtr` against `git_atomic__*` over a `void *` | `unit_pub` `atomics::cas_pointer`, `atomics::swap`, `atomics::load_ptr` and `atomics::a_compare_and_swap_to_the_same_value_succeeds` |
| `intptr_slot_sequences_match_the_c` | the same over an `intptr_t` slot, across the sign | `unit_pub` `atomics::cas_intptr`, `atomics::load_intptr` and `atomics::unsigned_slots_compare_swap_and_load` |
| `c_and_rust_share_one_count` | C's builtins and the port on one count from many threads: no update lost, each side reading the other's | `ub_safe` `atomics::a_refcount_rises_and_falls_from_many_threads` and `thread::shared_handles_add_from_many_threads`; the gateway's `unit_priv::exports_lose_no_update_across_threads`, which mixes the exports and the native API on one count. Sharing one object with C's builtins ends at the switch: no C builtin is left to share it with |
| `c_and_rust_race_to_install_one_value` | C's and the port's compare-and-swap racing to fill one slot | `ub_safe` `atomics::racing_initialisers_install_one_value` and `thread::compare_and_swap_from_many_threads` |

## `tests/equiv/src/util/thread/tlsdata.rs`

| equivalence test | what it compares | kept by |
|---|---|---|
| `set_get_and_dispose_match_c` | `test_threads_tlsdata__can_set_and_get`'s calls and more, on a POSIX key and a `TlsKey` | `unit_pub` `tlsdata::can_set_and_get` and `tlsdata::a_value_is_absent_until_stored_and_after_none`; the gateway's `unit_priv::the_exports_keep_cs_contract` for C's keys |
| `threads_match_c` | `test_threads_tlsdata__threads`'s ten threads | `unit_pub` `tlsdata::threads` and `tlsdata::a_static_key_serves_every_thread` |
| `destructors_run_on_the_values_c_destroys` | the values each destructor runs on at thread exit, after storing, replacing and removing, and none for a disposed key or a key without one | `unit_pub` `tlsdata::the_destructor_runs_at_thread_exit_on_the_values_held`, `tlsdata::set_returns_the_value_it_replaces_without_destroying_it`, `tlsdata::a_key_without_a_destructor_drops_its_values_at_thread_exit` and `tlsdata::a_disposed_key_names_no_key_and_destroys_nothing`; `ffi_import`'s `unit_priv::c_keys_store_words_and_destroy_them_at_thread_exit` for C's words and destructor |
| `refusals_of_a_key_naming_none_match_c` | the refusals of a key that names none | `unit_pub` `tlsdata::a_disposed_key_names_no_key_and_destroys_nothing`, `tlsdata::a_key_disposed_beside_a_live_one_names_no_key` (this pass) and `tlsdata::ids_are_never_reused`; the gateway's `unit_priv::key_zero_names_no_key` `ffi_import`'s `unit_priv::c_reaches_no_rust_key_and_no_unknown_id`, and its `unit_priv::c_reaches_no_rust_key_from_a_thread_that_holds_none` (this pass) |

## The gateway's `equiv_priv` (`src/libgit2/src/util/thread.rs`, `mod ffi_export`)

These compare the exports with the header's `static inline` bodies, which the raw bindings
reach through `static_fns.c`'s `<name>__extern` wrappers until the switch removes the bodies
and the wrappers: the module goes, and with it the second implementation.

| equivalence test | kept by |
|---|---|
| `atomic32_exports_match_the_header` | `unit_priv::atomic32_exports_return_what_they_leave` |
| `atomic64_exports_match_the_header` | `unit_priv::atomic64_exports_return_what_they_leave` |
| `slot_exports_match_the_header` | `unit_priv::slot_exports_return_what_the_slot_held` and `unit_priv::slot_exports_carry_an_integer` |
| `exports_and_header_share_one_count` | `unit_priv::exports_lose_no_update_across_threads` |

After the switch the C that remains checks the exports too: the C suites `threads::atomic`,
`threads::tlsdata`, `threads::basic`, `threads::diff` and every suite that takes a reference
count, which the switch rehearsal of this run passed on `plain` (batch 6), ASan+UBSan and TSan.
