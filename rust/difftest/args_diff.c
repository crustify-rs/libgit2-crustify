/*
 * Differential test of argument handling at the C ABI (unsafe-r1 D-1, T-1):
 * every line printed must be the same when linked against the C reference
 * and against the Rust port.
 *
 * - NULL arguments that the C checks (or returns before reading): the
 *   return code and the error message, in the C's order of checks.
 * - String arguments that lie in the call's own output buffer: the C reads
 *   them where it uses them (after it sanitized or cleared the buffer).
 *
 * Writes only to a scratch copy of testrepo.git (./args_diff.tmp, removed
 * at the end); the shared fixture is never opened.
 *
 * difftest-link: libgit2_tests
 * Build and compare: RUST_BUILD=/work/builds/unsafe rust/difftest/run.sh args_diff.c
 */

#include "common.h"
#include "git2.h"
#include "git2/sys/odb_backend.h"
#include "git2/sys/errors.h"
#include "futils.h"
#include "str.h"
#include "vector.h"
#include "refs.h"
#include "commit.h"
#include "index.h"
#include "repository.h"
#include <stdio.h>
#include <string.h>

static void show(const char *what, int rc)
{
	const git_error *e = git_error_last();
	printf("%-52s rc=%d err=%s\n", what, rc, e && e->message ? e->message : "(none)");
	git_error_clear();
}

static void show_buf(const char *what, int rc, const char *ptr, size_t size)
{
	size_t i;
	printf("%-52s rc=%d size=%zu [", what, rc, size);
	for (i = 0; i < size; i++)
		putchar(ptr[i] ? ptr[i] : '.');
	printf("]\n");
	git_error_clear();
}

static void nulls(git_repository *repo)
{
	git_odb *odb = NULL;
	git_odb_object *o = NULL;
	git_odb_stream *st = NULL;
	size_t len;
	git_object_t t;
	git_treebuilder *bld = NULL;
	git_oid tid;
	git_tree *tree = NULL;
	git_vector v = GIT_VECTOR_INIT;
	git_index *index = NULL;

	git_odb_new(&odb);
	show("read_prefix(&o, odb, NULL, 3)", git_odb_read_prefix(&o, odb, NULL, 3));
	show("read_prefix(NULL, odb, NULL, 40)", git_odb_read_prefix(NULL, odb, NULL, 40));
	show("read_prefix(&o, NULL, NULL, 40)", git_odb_read_prefix(&o, NULL, NULL, 40));
	show("open_rstream(empty odb, NULL)", git_odb_open_rstream(&st, &len, &t, odb, NULL));
	show("open_rstream(NULL stream, empty odb, NULL)", git_odb_open_rstream(NULL, &len, &t, odb, NULL));
	git_odb_free(odb);

	git_treebuilder_new(&bld, repo, NULL);
	git_treebuilder_write(&tid, bld);
	git_tree_lookup(&tree, repo, &tid);
	git_error_clear();
	show(git_tree_entry_byid(tree, NULL) ? "byid(empty tree, NULL) -> entry" : "byid(empty tree, NULL) -> NULL", 0);
	show("byid(NULL, NULL)", git_tree_entry_byid(NULL, NULL) ? 1 : 0);
	git_tree_free(tree);
	git_treebuilder_free(bld);

	show("vector_insert_null(NULL, 0, 0)", git_vector_insert_null(NULL, 0, 0));
	show("vector_insert_null(&v, 1, 1)", git_vector_insert_null(&v, 1, 1));
	git_vector_dispose(&v);

	git_index_new(&index);
	show("index__fill(NULL, NULL)", git_index__fill(NULL, NULL));
	git_index_free(index);

	show("shallow_roots_write(NULL, NULL)", git_repository__shallow_roots_write(NULL, NULL));
}

static void in_output(git_repository *repo)
{
	git_buf b = GIT_BUF_INIT;
	git_str s = GIT_STR_INIT;
	git_object *obj = NULL;
	git_config *cfg = NULL;
	git_strarray a;
	char *strs[2];
	int rc;

	/* git_config_parse_path: sanitized first, then `value` read. */
	b.ptr = git__strdup("abcde"); b.reserved = 6; b.size = 5;
	rc = git_config_parse_path(&b, b.ptr);
	show_buf("parse_path(&b, b.ptr)", rc, b.ptr, b.size);
	git_buf_dispose(&b);
	b.ptr = git__strdup("xabcde"); b.reserved = 7; b.size = 6;
	rc = git_config_parse_path(&b, b.ptr + 1);
	show_buf("parse_path(&b, b.ptr + 1)", rc, b.ptr, b.size);
	git_buf_dispose(&b);

	/* git_commit_header_field: cleared first, then `field` read. */
	if (git_revparse_single(&obj, repo, "HEAD") == 0) {
		b.ptr = git__strdup("xauthor"); b.reserved = 8; b.size = 7;
		rc = git_commit_header_field(&b, (git_commit *)obj, b.ptr + 1);
		show_buf("header_field(&b, c, b.ptr + 1)", rc, b.ptr, b.size);
		git_buf_dispose(&b);
		b.ptr = git__strdup("author"); b.reserved = 7; b.size = 6;
		rc = git_commit_header_field(&b, (git_commit *)obj, b.ptr);
		show_buf("header_field(&b, c, b.ptr)", rc, b.ptr, b.size);
		git_buf_dispose(&b);
		git_object_free(obj);
	}

	/* git_config_get_string_buf / get_path: name read after the sanitize. */
	if (git_repository_config_snapshot(&cfg, repo) == 0) {
		b.ptr = git__strdup("xcore.bare"); b.reserved = 11; b.size = 10;
		rc = git_config_get_string_buf(&b, cfg, b.ptr + 1);
		show_buf("get_string_buf(&b, cfg, b.ptr + 1)", rc, b.ptr, b.size);
		git_buf_dispose(&b);
		b.ptr = git__strdup("core.bare"); b.reserved = 10; b.size = 9;
		rc = git_config_get_string_buf(&b, cfg, b.ptr);
		show_buf("get_string_buf(&b, cfg, b.ptr)", rc, b.ptr, b.size);
		git_buf_dispose(&b);
		git_config_free(cfg);
	}

	/* git_reference__normalize_name: '/' checked, cleared, then read.
	 * (A name further into the buffer is read by the C while it writes
	 * the result over it: rust/DEFECTS.md; not compared.) */
	git_str_sets(&s, "q//w");
	rc = git_reference__normalize_name(&s, s.ptr, 0);
	show_buf("normalize_name(&s, s.ptr)", rc, s.ptr, s.size);
	git_str_sets(&s, "/a");
	rc = git_reference__normalize_name(&s, s.ptr, 0);
	show_buf("normalize_name(&s, s.ptr) leading slash", rc, s.ptr, s.size);
	git_str_dispose(&s);

	/* git_blob_filter: `path` read only while the filters load. */
	{
		git_oid id;
		git_blob *blob = NULL;
		git_oid_from_string(&id, "a8233120f6ad708f843d861ce2b7228ec4e3dec6", GIT_OID_SHA1);
		if (git_blob_lookup(&blob, repo, &id) == 0) {
			b.ptr = git__strdup("xREADME"); b.reserved = 8; b.size = 7;
			rc = git_blob_filter(&b, blob, b.ptr + 1, NULL);
			show_buf("blob_filter(&b, blob, b.ptr + 1)", rc, b.ptr, b.size > 20 ? 20 : b.size);
			git_buf_dispose(&b);
			git_blob_free(blob);
		}
	}

	/* git_strarray_copy(a, a): C empties it. */
	strs[0] = "x"; strs[1] = "y"; a.strings = strs; a.count = 2;
	rc = git_strarray_copy(&a, &a);
	printf("%-52s rc=%d count=%zu strings=%s\n", "strarray_copy(&a, &a)", rc, a.count, a.strings ? "set" : "NULL");
}

int main(void)
{
	git_repository *repo = NULL;

	git_libgit2_init();
	git_futils_rmdir_r("args_diff.tmp", NULL, GIT_RMDIR_REMOVE_FILES);
	if (git_futils_cp_r("tests/resources/testrepo.git", "args_diff.tmp", GIT_CPDIR_CREATE_EMPTY_DIRS | GIT_CPDIR_COPY_DOTFILES, 0777) < 0 ||
	    git_repository_open(&repo, "args_diff.tmp") < 0) {
		printf("setup failed\n");
		return 1;
	}
	nulls(repo);
	in_output(repo);
	git_repository_free(repo);
	git_futils_rmdir_r("args_diff.tmp", NULL, GIT_RMDIR_REMOVE_FILES);
	git_libgit2_shutdown();
	return 0;
}
