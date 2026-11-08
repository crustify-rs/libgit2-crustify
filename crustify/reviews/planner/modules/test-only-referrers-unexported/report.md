# Exports that only C tests reach, left without a port/unexport

## What the earlier plan decided

Planner run 6 planned a ported function's or global's `port/unexport` only when the
closure's C references it. It counted the C tests as referrers only on top of that
(`crustify/artifacts/planner/run-6/compose/stages.py`, `stage`):

```python
    if objective == 'port/unexport' and (freeing or keepers):
        # a function or global that remaining C never references has no export:
        # the tests count only for one that the closure's C references
        wait, untied = test_wait(k, own)
```

A function that only C tests reference therefore got the verdict `none`. The scheduling
spec reads that verdict as "its change is part of the item's `port`", so the module's
port writes no export for it.

## The evidence against it

The exports serve the remaining C, "its translation units, tests and executables alike"
(`specs/workflows/porting.md`, Rust consumers; the conventions' Exports section). The
scheduling spec plans no `port/unexport` only for "a function or global that remaining C
never references". A C test of another module's subject outlives the integration of the
module whose function it calls. That test is remaining C, and it links against the
export.

One case is `git_annotated_commit_from_revspec`. Its only callers are C tests:
`tests/libgit2/rebase/abort.c`, `refs/reflog/messages.c`, and `reset/hard.c`, `mixed.c`
and `soft.c`. Those tests are tied to `rebase.c`, `reflog.c` and `refs.c`, and `reset.c`,
so they stay after `annotated_commit.c` is integrated. They need its export until those
modules are integrated too.

```sh
cd /target/libgit2-bindings
wavefront /target/libgit2-bindings/c --config ../crustify/wavefront/configs/wavefront-config.json \
  query symbols --name git_annotated_commit_from_revspec --callers     # its callers: tests only
grep -rn git_annotated_commit_from_revspec c/tests | head
python3 -I crustify/artifacts/planner/run-7/compose/stages7.py
python3 -I crustify/artifacts/planner/run-7/compose/test_referrers7.py   # 0 violations
```

Under run 6's verdicts, 176 functions and globals would have had no export at their
module's integration, although a remaining test calls them. The remaining tests would
then fail to link.

## The edit and what it moves

`stages7.py` counts the C tests as referrers of every function and global, whatever the
closure's C does. A test case leaves with the first module it is tied to to be integrated.
A test file's helpers leave with its last test case, and a helper file leaves with the
last test that reaches it. The 176 items below are `port/unexport` items now, each in a
stage wave of its module. The wave waits for the last translation wave of one module of
each test that reaches the item (the wave's index is in parentheses):

- `src/libgit2/annotated_commit.c`: git_annotated_commit_from_revspec (2141)
- `src/libgit2/apply.c`: git_apply__patch (2255)
- `src/libgit2/attrcache.c`: git_attr_cache__is_cached (1948), git_attr_cache_flush (1948)
- `src/libgit2/blob.c`: git_blob_create_from_workdir (2150), git_blob_data_is_binary (1158)
- `src/libgit2/branch.c`: git_branch_create_from_annotated (1470), git_branch_iterator_free (1472), git_branch_iterator_new (1472), git_branch_move (1473), git_branch_next (1472), git_branch_remote_name (1471), git_branch_upstream_name (1471)
- `src/libgit2/checkout.c`: git_checkout_head (2154)
- `src/libgit2/clone.c`: git_clone_options_init (1356)
- `src/libgit2/commit.c`: git_commit__parse (1206), git_commit_create_buffer (2328), git_commit_create_from_ids (1204), git_commit_create_with_signature (2328), git_commit_raw_header (1203)
- `src/libgit2/config.c`: git_config_delete_multivar (594)
- `src/libgit2/config_cache.c`: git_repository__configmap_lookup_cache_clear (707)
- `src/libgit2/diff_parse.c`: git_diff_from_buffer (1236)
- `src/libgit2/diff_print.c`: git_diff_print (1233), git_diff_print_callback__to_buf (937), git_diff_print_callback__to_file_handle (1233), git_diff_status_char (1233), git_diff_to_buf (1042), git_patch_to_buf (974)
- `src/libgit2/fetch.c`: git_fetch_options_init (1143)
- `src/libgit2/filter.c`: git_filter_register (1076), git_filter_unregister (1076)
- `src/libgit2/ignore.c`: git_ignore_add_rule (2213)
- `src/libgit2/index.c`: git_index__checksum (985), git_index_add_all (985), git_index_clear (985), git_index_conflict_get (986), git_index_entry_stage (2149), git_index_open (1946), git_index_path (2092), git_index_remove_bypath (2205), git_index_reuc_entrycount (2149), git_index_reuc_get_byindex (2149), git_index_set_caps (1235)
- `src/libgit2/iterator.c`: git_iterator_foreach (1238)
- `src/libgit2/merge.c`: git_merge (2153), git_merge_base_many (1999), git_merge_base_octopus (1999), git_merge_commits (2153)
- `src/libgit2/merge_driver.c`: git_merge_driver_register (1302), git_merge_driver_unregister (1302)
- `src/libgit2/object_api.c`: git_blob_lookup_prefix (845), git_commit_lookup_prefix (844), git_tag_id (844)
- `src/libgit2/odb.c`: git_odb_add_backend (1819), git_odb_foreach (748), git_odb_new (1819), git_odb_open (747), git_odb_write_multi_pack_index (749)
- `src/libgit2/odb_loose.c`: git_odb__backend_loose (955), git_odb_backend_loose (955)
- `src/libgit2/odb_mempack.c`: git_mempack_new (1823)
- `src/libgit2/oid.c`: git_oid_ncmp (746), git_oid_strcmp (472), git_oid_streq (2186)
- `src/libgit2/pack-objects.c`: git_packbuilder_insert_tree (1741), git_packbuilder_name (1741)
- `src/libgit2/patch.c`: git_patch_get_hunk (1232), git_patch_get_line_in_hunk (1232), git_patch_num_hunks (1232), git_patch_num_lines_in_hunk (792), git_patch_size (792)
- `src/libgit2/patch_generate.c`: git_patch_from_buffers (1640)
- `src/libgit2/patch_parse.c`: git_patch_from_buffer (975)
- `src/libgit2/pathspec.c`: git_pathspec_free (1342), git_pathspec_match_diff (1342), git_pathspec_match_list_diff_entry (1342), git_pathspec_match_list_entry (1342), git_pathspec_match_list_entrycount (1342), git_pathspec_match_list_free (1342), git_pathspec_match_tree (1342), git_pathspec_new (1342)
- `src/libgit2/push.c`: git_push_options_init (1611)
- `src/libgit2/rebase.c`: git_rebase_commit (2344), git_rebase_finish (2344), git_rebase_free (2344), git_rebase_init (2344), git_rebase_next (2344), git_rebase_open (2343), git_rebase_operation_byindex (2343), git_rebase_operation_current (2343), git_rebase_operation_entrycount (2343)
- `src/libgit2/refdb.c`: git_refdb_compress (720), git_refdb_delete (2137), git_refdb_new (720)
- `src/libgit2/reflog.c`: git_reflog_append (784), git_reflog_entry_id_old (1300)
- `src/libgit2/refs.c`: git_reference__cmp_cb (532), git_reference_cmp (663), git_reference_has_log (781), git_reference_set_target (532), git_reference_shorthand (533)
- `src/libgit2/refspec.c`: git_refspec_transform (573)
- `src/libgit2/remote.c`: git_remote_add_fetch (1102), git_remote_add_push (1103), git_remote_connect (1355), git_remote_create_anonymous (1104), git_remote_default_branch (1355), git_remote_delete (1102), git_remote_disconnect (1105), git_remote_get_refspec (1106), git_remote_push (1105), git_remote_pushurl (1106), git_remote_rename (1102), git_remote_set_instance_pushurl (1106), git_remote_set_instance_url (1106), git_remote_set_url (1107), git_remote_upload (1105)
- `src/libgit2/repository.c`: git_repository_detach_head (2194), git_repository_get_namespace (669), git_repository_head_commit (668), git_repository_head_detached (2136), git_repository_is_shallow (1810), git_repository_new (1501), git_repository_open_from_worktree (1326), git_repository_set_bare (1502), git_repository_set_config (667), git_repository_set_head_detached_from_annotated (782), git_repository_set_ident (1099), git_repository_set_index (1945), git_repository_set_namespace (669), git_repository_set_workdir (1945), git_repository_submodule_cache_all (1700), git_repository_submodule_cache_clear (1700), git_repository_wrap_odb (1817)
- `src/libgit2/revwalk.c`: git_revwalk_push_head (1545), git_revwalk_push_ref (1546), git_revwalk_reset (1544)
- `src/libgit2/signature.c`: git_signature__equal (825), git_signature_from_buffer (825)
- `src/libgit2/status.c`: git_status_foreach (1950), git_status_should_ignore (1951)
- `src/libgit2/streams/openssl.c`: git_openssl__reset_context (1055)
- `src/libgit2/submodule.c`: git_submodule_add_to_index (1716), git_submodule_foreach (2217), git_submodule_name (1713), git_submodule_open (2217), git_submodule_path (2217), git_submodule_repo_init (1714), git_submodule_set_ignore (1713), git_submodule_set_url (1715), git_submodule_update (1712)
- `src/libgit2/tag.c`: git_tag_annotation_create (922), git_tag_create (921), git_tag_create_lightweight (920), git_tag_delete (919)
- `src/libgit2/transports/credential.c`: git_credential_ssh_key_memory_new (1347), git_credential_ssh_key_new (1347)
- `src/libgit2/transports/local.c`: git_transport_local (1803)
- `src/libgit2/transports/smart.c`: git_transport_remote_connect_options (930)
- `src/libgit2/transports/ssh.c`: git_transport_ssh_with_paths (1133)
- `src/libgit2/tree.c`: git_tree_create_updated (1082), git_tree_entry_byname (1083)
- `src/libgit2/worktree.c`: git_worktree_add (1328)
- `src/util/fs_path.c`: git__percent_decode (476), git_fs_path__set_owner (477), git_fs_path_basename_r (476), git_fs_path_does_decompose_unicode (476), git_fs_path_islink (476)
- `src/util/futils.c`: git_futils_cp (588), git_futils_creat_withpath (982), git_futils_mkdir_r (1944)
- `src/util/posix.c`: p_fsync__cnt (1377)
- `src/util/str.c`: git_str_join_n (475)
- `src/util/thread.c`: git_atomic32_add (340)
- `src/util/vector.c`: git_vector_verify_sorted (980)

The stage waves are re-planned with the rest: 1010 stage waves in place of 780, holding
1547 `port/unexport` items in place of 1183.

## Questions for the user

None.
