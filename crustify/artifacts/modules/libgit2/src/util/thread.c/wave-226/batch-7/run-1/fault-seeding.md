# Faults seeded in thread.c before the switch

What `wave-226/batch-7/run-1`, the module pass, seeded in `src/libgit2/src/util/thread.rs`
before the switch retires the module's equivalence tests, as `ubc_check.c`'s module pass did
(`module-pass-seeds-faults-before-the-switch`, on `crustify/devel`): one fault at a time, each
one site changed, the module's tests run over each, binary by binary.

## How

`fault-seeding/`, beside this record, holds the run:

- `mutants.py <thread.rs> <mutants.json>` writes the 64 faults, each an exact replacement of
  one site: a returned value (the one found where the one left is due, a flipped bit), a
  constant (`+ 1`, `^ 1`, the first id), an operator (add for subtract, saturating for
  wrapping), a comparison (a check dropped, `all` for `any`) or a statement dropped. Each site
  is checked unique in the file.
- `mutate.py <scratch tree> <mutants.json> <results.json> [ids…]` applies each fault to a
  scratch copy of the tree (`git archive HEAD`, `c/` and `crustify/builds` linked), builds
  with `ffi-export-src-util-thread-c` on, as the switch compiles the gateway, and runs
  `cargo-variant.sh plain test --features ffi-export-src-util-thread-c <binary> --
  util::thread` for `lib`, `unit_pub`, `ub_safe`, `ub_unsafe` and `equiv`, each under a
  240 s timeout; a failed test, or a binary that times out or ends without a result, catches
  the fault.
- `classify.py <results.json>` tallies them: the equivalence tests are the `equiv` binary's
  and the gateway's `equiv_priv`; every other test outlives the C.

`results-before.json` is the run over the wave's tip (`2f2ffb73d`); `results-after.json`
reruns, over this pass's tree, the three faults no test caught.

## What it found

| region | faults | outliving tests catch, at the tip | after this pass | equivalence tests catch |
|---|---:|---:|---:|---:|
| native atomics (`Atomic32Ref`, `Atomic64Ref`, `AtomicWord`) | 21 | 20 | 20 | 20 |
| the gateway (`mod ffi_export`) | 18 | 18 | 18 | 17 |
| thread-local storage (`tls`, `TlsKey`) | 19 | 17 | 19 | 9 |
| locks | 3 | 3 | 3 | 0 |
| threads (`ThreadRef`, `ThreadMut`, `JoinHandle`) | 3 | 3 | 3 | 0 |
| **all** | **64** | **61** | **63** | **46** |

- No fault is caught by the equivalence tests alone: what they check, the tests that stay
  check too, as `equiv-retirement.md` maps test by test.
- The one fault nothing catches, `load-cas`, makes `load` write back the value it read with a
  swap; it returns the same value and leaves the same value, so it is equivalent: no test,
  of either kind, can tell it apart.
- Two faults of the thread-local storage escaped every test at the tip, and this pass added a
  test for each, which catches it:
  - `t-prune-any`, a prune that stops when any entry of the thread's table is live, leaves a
    disposed key's entry beside a live key's, and the next store under the disposed key
    replaces it and succeeds. `unit_pub`'s
    `tlsdata::a_key_disposed_beside_a_live_one_names_no_key` stores under two keys, disposes
    of one and checks that a store under it fails and its values are dropped.
  - `t-first-store-any`, a thread's first store under an id that ignores the key's kind, lets
    C store a word under a Rust key's id. `ffi_import`'s
    `unit_priv::c_reaches_no_rust_key_from_a_thread_that_holds_none` makes the store from a
    thread that holds no value of the key; the existing test made it from one that did, where
    the entry's kind refused it.
- 16 of the 18 faults of the gateway are caught only by the gateway's own `unit_priv` tests,
  which compile only under the module's export feature and so run in no gate before the
  switch; from the switch on they run in every one. The other two (`x-set-null`,
  `x-get-null`) are also caught by `unit_pub`'s `sync::each_thread_sets_its_own_error`, whose
  error state is C's `errors.c` on the exports.

## Each fault

| region | fault | site | outliving tests that fail | equivalence tests that fail |
|---|---|---|---|---|
| native atomics | `a32-inc-old` | inc returns the value it found | lib: `atomic32_exports_return_what_they_leave`, lib: `exports_lose_no_update_across_threads`, unit_pub: `a_count_rises_and_falls_through_every_handle`, unit_pub: `atomic32_inc`, unit_pub: `atomic32_operations_return_what_they_leave_and_wrap`, ub_safe: `a_refcount_rises_and_falls_from_many_threads` | 4 |
| native atomics | `a32-inc-by2` | inc adds 2 | lib: `atomic32_exports_return_what_they_leave`, lib: `exports_lose_no_update_across_threads`, unit_pub: `a_count_rises_and_falls_through_every_handle`, unit_pub: `atomic32_inc`, unit_pub: `atomic32_operations_return_what_they_leave_and_wrap`, ub_safe: `a_refcount_rises_and_falls_from_many_threads` | 4 |
| native atomics | `a32-inc-sat` | inc's result saturates | lib: `atomic32_exports_return_what_they_leave`, unit_pub: `atomic32_operations_return_what_they_leave_and_wrap` | 2 |
| native atomics | `a32-dec-old` | dec returns the value it found | lib: `atomic32_exports_return_what_they_leave`, unit_pub: `a_count_rises_and_falls_through_every_handle`, unit_pub: `atomic32_dec`, unit_pub: `atomic32_operations_return_what_they_leave_and_wrap` | 2 |
| native atomics | `a32-dec-add` | dec adds | lib: `atomic32_exports_return_what_they_leave`, lib: `exports_lose_no_update_across_threads`, unit_pub: `a_count_rises_and_falls_through_every_handle`, unit_pub: `atomic32_dec`, unit_pub: `atomic32_operations_return_what_they_leave_and_wrap`, ub_safe: `a_refcount_rises_and_falls_from_many_threads` | 4 |
| native atomics | `a32-dec-sat` | dec's result saturates | lib: `atomic32_exports_return_what_they_leave`, unit_pub: `atomic32_operations_return_what_they_leave_and_wrap` | 2 |
| native atomics | `a32-add-old` | add32 returns the value it found | lib: `atomic32_exports_return_what_they_leave`, unit_pub: `atomic32_add`, unit_pub: `atomic32_operations_return_what_they_leave_and_wrap` | 3 |
| native atomics | `a32-add-sub` | add32 subtracts | lib: `atomic32_exports_return_what_they_leave`, unit_pub: `atomic32_add`, unit_pub: `atomic32_operations_return_what_they_leave_and_wrap` | 3 |
| native atomics | `a32-add-sat` | add32's result saturates | lib: `atomic32_exports_return_what_they_leave`, unit_pub: `atomic32_operations_return_what_they_leave_and_wrap` | 1 |
| native atomics | `a32-get-bit` | get32 flips the low bit | lib: `atomic32_exports_return_what_they_leave`, lib: `exports_lose_no_update_across_threads`, unit_pub: `a_count_rises_and_falls_through_every_handle`, unit_pub: `a_memory_barrier_keeps_every_value`, unit_pub: `atomic32_get`, unit_pub: `atomic32_operations_return_what_they_leave_and_wrap`, ub_safe: `a_fence_orders_a_value_before_its_flag`, ub_safe: `a_refcount_rises_and_falls_from_many_threads` | 4 |
| native atomics | `a32-set-plus1` | set32 stores val + 1 | lib: `atomic32_exports_return_what_they_leave`, unit_pub: `a_memory_barrier_keeps_every_value`, unit_pub: `atomic32_operations_return_what_they_leave_and_wrap`, unit_pub: `atomic32_set`, ub_safe: `a_fence_orders_a_value_before_its_flag` | 2 |
| native atomics | `a64-add-old` | add64 returns the value it found | lib: `atomic64_exports_return_what_they_leave`, unit_pub: `atomic64_add`, unit_pub: `atomic64_operations_return_what_they_leave_and_wrap`, unit_pub: `atomic_ssize_adds_and_reads_back` | 2 |
| native atomics | `a64-add-sat` | add64's result saturates | lib: `atomic64_exports_return_what_they_leave`, unit_pub: `atomic64_operations_return_what_they_leave_and_wrap` | 1 |
| native atomics | `a64-get-bit` | get64 flips the low bit | lib: `atomic64_exports_return_what_they_leave`, unit_pub: `atomic64_get`, unit_pub: `atomic64_operations_return_what_they_leave_and_wrap`, unit_pub: `atomic_ssize_adds_and_reads_back`, ub_safe: `a_static_total_adds_from_many_threads` | 2 |
| native atomics | `a64-set-plus1` | set64 stores val + 1 | lib: `atomic64_exports_return_what_they_leave`, unit_pub: `atomic64_operations_return_what_they_leave_and_wrap`, unit_pub: `atomic64_set`, unit_pub: `atomic_ssize_adds_and_reads_back` | 2 |
| native atomics | `cas-new` | a successful compare-and-swap returns the new value | lib: `slot_exports_carry_an_integer`, lib: `slot_exports_return_what_the_slot_held`, unit_pub: `cas_intptr`, unit_pub: `cas_pointer`, unit_pub: `unsigned_slots_compare_swap_and_load`, ub_safe: `racing_initialisers_install_one_value` | 3 |
| native atomics | `cas-old` | a compare-and-swap returns the expected value either way | lib: `slot_exports_return_what_the_slot_held`, unit_pub: `a_compare_and_swap_to_the_same_value_succeeds`, unit_pub: `cas_intptr`, unit_pub: `cas_pointer`, unit_pub: `unsigned_slots_compare_swap_and_load`, ub_safe: `racing_initialisers_install_one_value` | 4 |
| native atomics | `cas-unconditional` | the compare-and-swap stores whatever the slot held | lib: `slot_exports_return_what_the_slot_held`, unit_pub: `a_compare_and_swap_to_the_same_value_succeeds`, unit_pub: `cas_intptr`, unit_pub: `cas_pointer`, unit_pub: `unsigned_slots_compare_swap_and_load` | 3 |
| native atomics | `swap-new` | swap returns the new value | lib: `exports_lose_no_update_across_threads`, lib: `slot_exports_return_what_the_slot_held`, unit_pub: `swap`, unit_pub: `unsigned_slots_compare_swap_and_load`, ub_safe: `swaps_from_many_threads_lose_no_value` | 3 |
| native atomics | `swap-nostore` | swap does not store | lib: `exports_lose_no_update_across_threads`, lib: `slot_exports_return_what_the_slot_held`, unit_pub: `swap`, unit_pub: `unsigned_slots_compare_swap_and_load`, ub_safe: `swaps_from_many_threads_lose_no_value` | 3 |
| native atomics | `load-cas` | load stores the slot's value back by compare-and-swap of a stale guess | none | 0 |
| gateway | `x-get32` | git_atomic32_get flips the low bit | lib: `atomic32_exports_return_what_they_leave`, lib: `exports_lose_no_update_across_threads` | 1 |
| gateway | `x-set32` | git_atomic32_set stores val ^ 1 | lib: `atomic32_exports_return_what_they_leave` | 1 |
| gateway | `x-inc32` | git_atomic32_inc adds 2 | lib: `atomic32_exports_return_what_they_leave`, lib: `exports_lose_no_update_across_threads` | 2 |
| gateway | `x-dec32` | git_atomic32_dec increments | lib: `atomic32_exports_return_what_they_leave`, lib: `exports_lose_no_update_across_threads` | 2 |
| gateway | `x-add32` | git_atomic32_add subtracts | lib: `atomic32_exports_return_what_they_leave` | 1 |
| gateway | `x-get64` | git_atomic64_get flips the low bit | lib: `atomic64_exports_return_what_they_leave` | 1 |
| gateway | `x-set64` | git_atomic64_set stores val ^ 1 | lib: `atomic64_exports_return_what_they_leave` | 1 |
| gateway | `x-add64` | git_atomic64_add subtracts | lib: `atomic64_exports_return_what_they_leave` | 1 |
| gateway | `x-cas` | git_atomic__compare_and_swap swaps unconditionally | lib: `slot_exports_return_what_the_slot_held` | 1 |
| gateway | `x-swap` | git_atomic__swap only loads | lib: `exports_lose_no_update_across_threads`, lib: `slot_exports_return_what_the_slot_held` | 1 |
| gateway | `x-load` | git_atomic__load empties the slot | lib: `slot_exports_carry_an_integer`, lib: `slot_exports_return_what_the_slot_held` | 1 |
| gateway | `x-global-init` | git_threads_global_init returns 1 | lib: `global_init_reports_success` | 0 |
| gateway | `x-init-id` | git_tlsdata_init writes the next id | lib: `key_zero_names_no_key`, lib: `the_exports_keep_cs_contract` | 3 |
| gateway | `x-init-nodtor` | git_tlsdata_init drops the destructor | lib: `the_exports_keep_cs_contract` | 1 |
| gateway | `x-set-ok` | git_tlsdata_set reports success on failure | lib: `key_zero_names_no_key`, lib: `the_exports_keep_cs_contract` | 1 |
| gateway | `x-set-null` | git_tlsdata_set stores NULL | lib: `the_exports_keep_cs_contract`, unit_pub: `each_thread_sets_its_own_error` | 3 |
| gateway | `x-get-null` | git_tlsdata_get returns NULL | lib: `the_exports_keep_cs_contract`, unit_pub: `each_thread_sets_its_own_error` | 3 |
| gateway | `x-dispose-ok` | git_tlsdata_dispose reports success on failure | lib: `key_zero_names_no_key`, lib: `the_exports_keep_cs_contract` | 1 |
| thread-local storage | `t-drop-no-c` | a thread's exit runs no C destructor | lib: `the_exports_keep_cs_contract`, lib: `a_thread_that_calls_exit_destroys_its_values`, lib: `c_keys_store_words_and_destroy_them_at_thread_exit` | 1 |
| thread-local storage | `t-drop-no-rust` | a thread's exit runs no Rust destructor | unit_pub: `a_destructor_cannot_store_while_its_thread_exits`, unit_pub: `clones_are_copies`, unit_pub: `the_destructor_runs_at_thread_exit_on_the_values_held`, ub_safe: `destructors_that_reach_the_storage_at_exit` | 1 |
| thread-local storage | `t-drop-dead` | a thread's exit destroys the values of disposed keys | ub_safe: `<101>` | 1 |
| thread-local storage | `t-replace-none` | replacing a value returns nothing | unit_pub: `set_returns_the_value_it_replaces_without_destroying_it`, ub_safe: `drops_that_reach_the_storage` | 0 |
| thread-local storage | `t-remove-keeps` | storing NULL keeps the value | lib: `the_exports_keep_cs_contract`, lib: `c_keys_store_words_and_destroy_them_at_thread_exit`, unit_pub: `a_value_is_absent_until_stored_and_after_none`, unit_pub: `set_returns_the_value_it_replaces_without_destroying_it`, unit_pub: `the_destructor_runs_at_thread_exit_on_the_values_held`, unit_pub: `with_lends_the_value_where_get_would_clone_it` | 2 |
| thread-local storage | `t-set-kind` | a store ignores the key's kind | lib: `c_reaches_no_rust_key_and_no_unknown_id` | 0 |
| thread-local storage | `t-get-holds` | a read ignores the key's state | ub_safe: `<101>` | 0 |
| thread-local storage | `t-first-id-0` | the first id is 0 | lib: `key_zero_names_no_key`, lib: `the_exports_keep_cs_contract`, lib: `a_c_key_without_a_destructor_destroys_nothing`, lib: `a_forked_child_exits_while_other_threads_make_keys`, lib: `a_thread_that_calls_exit_destroys_its_values`, lib: `c_keys_store_words_and_destroy_them_at_thread_exit`, lib: `c_reaches_no_rust_key_and_no_unknown_id`, lib: `a_key_holds_one_kind`, lib: `ids_start_at_one_and_run_out_after_the_last`, unit_pub: `concurrent_init_succeeds`, unit_pub: `each_thread_sets_its_own_error`, unit_pub: `a_clone_run_by_get_reads_but_cannot_store`, unit_pub: `a_destructor_cannot_store_while_its_thread_exits`, unit_pub: `a_disposed_key_names_no_key_and_destroys_nothing`, unit_pub: `a_key_without_a_destructor_drops_its_values_at_thread_exit`, unit_pub: `a_static_key_serves_every_thread`, unit_pub: `a_value_is_absent_until_stored_and_after_none`, unit_pub: `can_set_and_get`, unit_pub: `clones_are_copies`, unit_pub: `global_init_sets_nothing_up`, unit_pub: `ids_are_never_reused`, unit_pub: `keys_and_destructors_print_and_copy`, unit_pub: `keys_of_different_types_are_independent`, unit_pub: `set_returns_the_value_it_replaces_without_destroying_it`, unit_pub: `the_destructor_runs_at_thread_exit_on_the_values_held`, unit_pub: `threads`, unit_pub: `with_lends_the_value_where_get_would_clone_it`, ub_safe: `a_clone_that_disposes_of_its_key`, ub_safe: `destructors_that_reach_the_storage_at_exit`, ub_safe: `drops_that_reach_the_storage`, ub_safe: `keys_made_and_disposed_while_threads_share_one`, ub_safe: `values_of_a_key_disposed_elsewhere_are_dropped_once` | 4 |
| thread-local storage | `t-reuse-id` | a disposed key's id is handed out again | lib: `a_forked_child_exits_while_other_threads_make_keys`, unit_pub: `a_destructor_cannot_store_while_its_thread_exits`, unit_pub: `a_disposed_key_names_no_key_and_destroys_nothing`, unit_pub: `ids_are_never_reused`, ub_safe: `keys_made_and_disposed_while_threads_share_one` | 1 |
| thread-local storage | `t-dispose-live` | disposing leaves the key live | lib: `c_keys_store_words_and_destroy_them_at_thread_exit`, unit_pub: `a_disposed_key_names_no_key_and_destroys_nothing`, unit_pub: `with_lends_the_value_where_get_would_clone_it`, ub_safe: `<101>` | 1 |
| thread-local storage | `t-holds-kind` | a key holds any kind | lib: `c_reaches_no_rust_key_and_no_unknown_id`, lib: `a_key_holds_one_kind` | 0 |
| thread-local storage | `t-holds-live` | a disposed key still holds | ub_safe: `<101>` | 0 |
| thread-local storage | `t-prune-keeps` | pruning keeps every entry | lib: `c_keys_store_words_and_destroy_them_at_thread_exit`, unit_pub: `a_disposed_key_names_no_key_and_destroys_nothing`, ub_safe: `a_clone_that_disposes_of_its_key`, ub_safe: `drops_that_reach_the_storage`, ub_safe: `values_of_a_key_disposed_elsewhere_are_dropped_once` | 0 |
| thread-local storage | `t-prune-any` | pruning stops when any entry is live | unit_pub: `a_key_disposed_beside_a_live_one_names_no_key` (none before this pass) | 0 |
| thread-local storage | `t-first-store-any` | a first store takes any registered id, of any kind | lib: `c_reaches_no_rust_key_from_a_thread_that_holds_none` (none before this pass) | 0 |
| thread-local storage | `t-dispose-twice` | disposing of a key twice succeeds | lib: `key_zero_names_no_key`, lib: `the_exports_keep_cs_contract`, lib: `c_keys_store_words_and_destroy_them_at_thread_exit`, lib: `c_reaches_no_rust_key_and_no_unknown_id`, unit_pub: `a_disposed_key_names_no_key_and_destroys_nothing`, ub_safe: `destructors_that_reach_the_storage_at_exit` | 1 |
| thread-local storage | `k-set-prev` | TlsKey::set returns no previous value | unit_pub: `a_value_is_absent_until_stored_and_after_none`, unit_pub: `set_returns_the_value_it_replaces_without_destroying_it`, unit_pub: `with_lends_the_value_where_get_would_clone_it` | 0 |
| thread-local storage | `k-dtor-skip` | TlsKey's destructor is never called | unit_pub: `a_destructor_cannot_store_while_its_thread_exits`, unit_pub: `clones_are_copies`, unit_pub: `the_destructor_runs_at_thread_exit_on_the_values_held`, ub_safe: `destructors_that_reach_the_storage_at_exit` | 1 |
| thread-local storage | `k-with-none` | TlsKey::with lends None | unit_pub: `with_lends_the_value_where_get_would_clone_it` | 0 |
| locks | `l-notify` | notify_one wakes nobody | unit_pub: `<timeout>`, ub_safe: `<timeout>` | 0 |
| locks | `l-mutex-unlock` | MutexGuard::unlock leaks the guard | unit_pub: `<timeout>`, ub_safe: `<timeout>` | 0 |
| locks | `l-poisoned` | Mutex::is_poisoned is false | unit_pub: `a_panic_under_a_mutex_poisons_it` | 0 |
| threads | `h-set-thread` | set_thread stores the id ^ 1 | ub_unsafe: `a_later_id_replaces_an_earlier_one`, ub_unsafe: `each_object_of_an_array_holds_its_own_id`, ub_unsafe: `stores_ids_that_name_no_thread`, ub_unsafe: `stores_the_id_of_a_live_thread` | 0 |
| threads | `h-thread` | ThreadRef::thread reads the id ^ 1 | unit_pub: `a_thread_prints_its_id`, unit_pub: `a_thread_value_moves_with_its_id`, unit_pub: `a_zeroed_thread_stores_the_id_zero`, ub_unsafe: `a_later_id_replaces_an_earlier_one`, ub_unsafe: `each_object_of_an_array_holds_its_own_id`, ub_unsafe: `stores_ids_that_name_no_thread`, ub_unsafe: `stores_the_id_of_a_live_thread` | 0 |
| threads | `h-finished` | is_finished is inverted | unit_pub: `<timeout>` | 0 |
