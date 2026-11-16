/*
 * Differential test of work package l-commit, part 2: the writers, mutators
 * and walkers (every line printed must be the same with the C reference and
 * the Rust port).  Based on the review's drivers (reviews/l-commit-r1-repro).
 *
 * Covers: commit buffers over a grid of times, offsets and signs;
 * git_commit_create / _v / _from_ids / _ext (extra headers, a signing
 * callback using git_commitbuilder_add_header) / _with_signature / amend;
 * extract_signature and header_field; summary
 * and body; treebuilder insert/remove/filter/write; git_tree_walk pointer
 * identity; bypath; create_updated; and the revwalk's error codes over
 * malformed parents.
 *
 * It works in a scratch repository, /tmp/l_commit_create_diff.git, made
 * afresh by each run (object ids are deterministic).
 *
 * difftest-link: libgit2_tests
 */
#include "common.h"
#include "futils.h"
#include <git2.h>
#include <git2/sys/errors.h>
#include <git2/sys/commit.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

static git_repository *repo;
static void err(const char *w, int rc) {
	const git_error *e = git_error_last();
	printf("%s: rc=%d class=%d msg=%s\n", w, rc, e ? e->klass : -1, e ? e->message : "");
	git_error_clear();
}
static void wr(git_oid *out, const char *data, git_object_t t) {
	git_odb *odb; git_repository_odb(&odb, repo);
	int rc = git_odb_write(out, odb, data, strlen(data), t);
	if (rc) err("odb_write", rc);
	git_odb_free(odb);
}

static void revwalk_case(const char *label, const char *body) {
	git_oid id, out; git_revwalk *w; int rc;
	wr(&id, body, GIT_OBJECT_COMMIT);
	git_revwalk_new(&w, repo);
	rc = git_revwalk_push(w, &id);
	err(label, rc);
	if (rc == 0) { int n = 0; while ((rc = git_revwalk_next(&out, w)) == 0) n++; printf("  n=%d ", n); err("next", rc); }
	git_revwalk_free(w);
}

static const git_tree_entry *saved[64]; static int nsaved;
static int walk_cb(const char *root, const git_tree_entry *e, void *p) {
	if (!*root && nsaved < 64) saved[nsaved++] = e;
	return 0;
}

static int filt(const git_tree_entry *e, void *p) { return git_tree_entry_name(e)[0] == 'b'; }

static int sign_cb(git_commitbuilder *b, git_repository *r, const char *content, void *payload)
{
	int rc;
	(void)r;
	printf("sign: content len=%zu payload=%s\n", strlen(content), (const char *)payload);
	rc = git_commitbuilder_add_header(b, "gpgsig", "-----BEGIN\nmiddle\n-----END");
	printf("add_header rc=%d\n", rc);
	rc = git_commitbuilder_add_header(b, "x-two", "v");
	printf("add_header2 rc=%d\n", rc);
	return 0;
}

static int fail_sign(git_commitbuilder *b, git_repository *r, const char *content, void *payload)
{
	(void)b; (void)r; (void)content; (void)payload;
	git_error_set_str(GIT_ERROR_INVALID, "sign failed");
	return -42;
}

static void show_commit_id(const char *what, int rc, const git_oid *id)
{
	printf("%s rc=%d %s\n", what, rc, rc ? "" : git_oid_tostr_s(id));
	if (rc)
		err("  ", rc);
	else {
		git_odb *odb; git_odb_object *o;
		git_repository_odb(&odb, repo);
		if (git_odb_read(&o, odb, id) == 0) {
			printf("  bytes=[%.*s]\n", (int)git_odb_object_size(o), (const char *)git_odb_object_data(o));
			git_odb_object_free(o);
		}
		git_odb_free(odb);
	}
}

static void more_creation(git_tree *tree)
{
	git_signature s1 = { "A U Thor", "a@b.c", { 1234567890, 120, '+' } };
	git_signature s2 = { "C O Mitter", "c@d.e", { 1234567999, -330, '-' } };
	git_oid c0, c1, c2, c3, c4, id;
	const git_oid *pids[2];
	git_commit *p0 = NULL, *p1 = NULL;
	git_commit_header hdrs[] = { { "x-a", "1" }, { "x-b", "multi\nline" } };
	git_commit_create_ext_options eo = GIT_COMMIT_CREATE_EXT_OPTIONS_INIT;
	int rc;

	rc = git_commit_create(&c0, repo, "HEAD", &s1, &s2, NULL, "root\n", tree, 0, NULL);
	show_commit_id("create root", rc, &c0);
	git_commit_lookup(&p0, repo, &c0);
	rc = git_commit_create_v(&c1, repo, "HEAD", &s1, &s2, "ISO-8859-1", "second\n\nbody\n", tree, 1, p0);
	show_commit_id("create_v", rc, &c1);
	rc = git_commit_create_v(&id, repo, NULL, &s1, &s2, NULL, "v null-terminated\n", tree, 5, p0, NULL);
	show_commit_id("create_v NULL", rc, &id);
	rc = git_commit_create_v(&id, repo, NULL, &s1, &s2, NULL, "x", NULL, 3);
	show_commit_id("create_v no tree", rc, &id);
	rc = git_commit_create_v(&id, repo, "HEAD", &s1, &s2, NULL, "not tip\n", tree, 1, p0);
	show_commit_id("create_v not tip", rc, &id);
	git_commit_lookup(&p1, repo, &c1);
	pids[0] = &c1; pids[1] = &c0;
	rc = git_commit_create_from_ids(&c2, repo, NULL, &s2, &s1, NULL, "merge\n", git_tree_id(tree), 2, pids);
	show_commit_id("from_ids", rc, &c2);
	pids[0] = git_tree_id(tree);
	rc = git_commit_create_from_ids(&id, repo, NULL, &s2, &s1, NULL, "bad parent\n", git_tree_id(tree), 1, pids);
	show_commit_id("from_ids bad", rc, &id);
	eo.extra_headers = hdrs; eo.extra_headers_len = 2; eo.message_encoding = "UTF-8";
	eo.sign = sign_cb; eo.payload = "pl";
	{
		git_commit *ps[2]; ps[0] = p1; ps[1] = p0;
		rc = git_commit_create_ext(&c3, repo, &s1, &s1, "ext\n", tree, 2, ps, &eo);
		show_commit_id("create_ext", rc, &c3);
		eo.sign = fail_sign;
		rc = git_commit_create_ext(&id, repo, &s1, &s1, "ext fail\n", tree, 1, ps, &eo);
		show_commit_id("create_ext fail", rc, &id);
		hdrs[1].value = NULL; eo.sign = NULL;
		rc = git_commit_create_ext(&id, repo, &s1, &s1, "null header\n", tree, 1, ps, &eo);
		show_commit_id("create_ext null header", rc, &id);
		rc = git_commit_create(&id, repo, NULL, &s1, &s1, NULL, NULL, tree, 1, (const git_commit **)ps);
		show_commit_id("create null message", rc, &id);
	}
	rc = git_commit_amend(&c4, p1, "HEAD", NULL, &s1, NULL, "amended\n", NULL);
	show_commit_id("amend", rc, &c4);
	rc = git_commit_amend(&id, p0, "HEAD", NULL, NULL, NULL, NULL, NULL);
	show_commit_id("amend not tip", rc, &id);
	{
		git_buf b = GIT_BUF_INIT, sig = GIT_BUF_INIT, data = GIT_BUF_INIT;
		rc = git_commit_create_buffer(&b, repo, &s1, &s2, NULL, "for signing\n", tree, 1, (const git_commit **)&p1);
		printf("buffer rc=%d\n", rc);
		rc = git_commit_create_with_signature(&id, repo, b.ptr, "-----BEGIN\nsig\n-----END", NULL);
		show_commit_id("with_signature", rc, &id);
		rc = git_commit_extract_signature(&sig, &data, repo, &id, NULL);
		printf("extract rc=%d sig=[%s] data=[%s]\n", rc, sig.ptr, data.ptr);
		git_buf_dispose(&sig); git_buf_dispose(&data);
		/* (An empty field hangs the C: DEFECTS.md; tested in Rust.) */
		rc = git_commit_create_with_signature(&id, repo, b.ptr, "s", "x-sig");
		show_commit_id("with_signature field", rc, &id);
		rc = git_commit_create_with_signature(&id, repo, "tree 1234\n", NULL, NULL);
		show_commit_id("with_signature bad", rc, &id);
		rc = git_commit_create_with_signature(&id, repo, "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nauthor a <b> 1 +0000\ncommitter a <b> 1 +0000\n", "s", NULL);
		show_commit_id("with_signature no body", rc, &id);
		git_buf_dispose(&b);
	}
	{
		git_commit *a = NULL;
		git_commit_lookup(&a, repo, &c4);
		rc = git_commit_nth_gen_ancestor(&p0, a, 1);
		printf("ancestor rc=%d %s\n", rc, rc ? "" : git_oid_tostr_s(git_commit_id(p0)));
		git_commit_free(p0);
		rc = git_commit_nth_gen_ancestor(&p0, a, 9);
		printf("ancestor 9 rc=%d\n", rc); if (rc) err("  ", rc);
		rc = git_commit_parent(&p0, a, 3);
		printf("parent 3 rc=%d\n", rc); if (rc) err("  ", rc);
		git_commit_free(a);
	}
	git_commit_free(p1);
}

int main(int argc, char **argv) {
	(void)argc; (void)argv;
	git_libgit2_init();
	git_futils_rmdir_r("/tmp/l_commit_create_diff.git", NULL, GIT_RMDIR_REMOVE_FILES);
	git_error_clear();
	if (git_repository_init(&repo, "/tmp/l_commit_create_diff.git", 1)) { err("init", -1); return 1; }
	const char *t = "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n";
	char buf[4096];
	/* revwalk on malformed commits */
	revwalk_case("bad tree", "tree xyz\nauthor a <b> 1 +0000\ncommitter a <b> 1 +0000\n\nm\n");
	snprintf(buf, sizeof buf, "%sauthor a <b> 1 +0000\n\nm\n", "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n"); revwalk_case("no committer", buf);
	snprintf(buf, sizeof buf, "%scommitter a b> 1 +0000\n\nm\n", "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n"); revwalk_case("bad email", buf);
	revwalk_case("short", "tree 12\n");
	snprintf(buf, sizeof buf, "%scommitter a <b> 1 +0000", "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n"); revwalk_case("no newline", buf);

	/* commit buffers */
	git_tree *tree; git_oid tid;
	git_oid_from_string(&tid, "4b825dc642cb6eb9a060e54bf8d69288fbee4904", GIT_OID_SHA1);
	{ git_treebuilder *b; git_treebuilder_new(&b, repo, NULL); git_treebuilder_write(&tid, b); git_treebuilder_free(b); }
	git_tree_lookup(&tree, repo, &tid);
	long long times[] = { 0, 1, 4294967296LL + 5, -1, 9223372036854775807LL };
	int offs[] = { 0, -1, 90, -2147483647 - 1, 2147483647, -754, 14*60+59 };
	char signs[] = { 0, '-', '+', 'x' };
	for (unsigned i = 0; i < 5; i++) for (unsigned j = 0; j < 7; j++) for (unsigned k = 0; k < 4; k++) {
		git_signature s = { "Na me", "e@x", { times[i], offs[j], signs[k] } };
		git_buf out = GIT_BUF_INIT;
		int rc = git_commit_create_buffer(&out, repo, &s, &s, k ? "UTF-8\nx" : NULL, "msg\n", tree, 0, NULL);
		if (rc) err("cb", rc); else { printf("cb %u %u %u: %s", i, j, k, strstr(out.ptr, "author")); }
		git_buf_dispose(&out);
	}
	/* extract signature */
	const char *sigs[] = {
		"%sauthor a <b> 1 +0000\ncommitter a <b> 1 +0000\ngpgsig -----BEGIN\n line2\n \n line4\nx y\n\nmsg\n",
		"%sauthor a <b> 1 +0000\ncommitter a <b> 1 +0000\ngpgsigx foo\ngpgsig\n\nmsg\n",
		"%sauthor a <b> 1 +0000\ncommitter a <b> 1 +0000\ngpgsig foo\n\nmsg gpgsig in body\n",
		"%sgpgsig a\n",
		"%sauthor a <b> 1 +0000\ncommitter a <b> 1 +0000\ngpgsig foo\n bar",
		"%sauthor a <b> 1 +0000\ncommitter a <b> 1 +0000\n\ngpgsig foo\n",
		NULL };
	for (unsigned i = 0; sigs[i]; i++) {
		git_oid id; git_buf s = GIT_BUF_INIT, d = GIT_BUF_INIT;
		snprintf(buf, sizeof buf, sigs[i], t); wr(&id, buf, GIT_OBJECT_COMMIT);
		const char *fields[] = { NULL, "gpgsig", "gpg", "author", "x", "msg" };
		for (unsigned f = 0; f < 6; f++) {
			int rc = git_commit_extract_signature(&s, &d, repo, &id, fields[f]);
			printf("es %u %u rc=%d sig=[%s] data=[%s]\n", i, f, rc, s.ptr ? s.ptr : "", d.ptr ? d.ptr : "");
			if (rc) err("  es", rc);
		}
		git_commit *c; if (!git_commit_lookup(&c, repo, &id)) {
			const char *hf[] = { "gpgsig", "committer", "x", "gpgsigx", "author" };
			for (unsigned f = 0; f < 5; f++) { git_buf h = GIT_BUF_INIT; int rc = git_commit_header_field(&h, c, hf[f]); printf("hf %u %s rc=%d [%s]\n", i, hf[f], rc, h.ptr ? h.ptr : ""); if (rc) err("  hf", rc); git_buf_dispose(&h);}
			printf("summary=[%s] body=[%s]\n", git_commit_summary(c), git_commit_body(c) ? git_commit_body(c) : "(null)");
			git_commit_free(c);
		} else err("lookup", -1);
		git_buf_dispose(&s); git_buf_dispose(&d);
	}
	/* treebuilder + walk + create_updated */
	{
		git_oid blob, sub, top; git_treebuilder *b; git_tree *tt;
		wr(&blob, "hello\n", GIT_OBJECT_BLOB);
		git_treebuilder_new(&b, repo, NULL);
		const char *names[] = { "b", "a", "a.b", "a-b", "ba", "b.c", NULL };
		for (int i = 0; names[i]; i++) { int rc = git_treebuilder_insert(NULL, b, names[i], &blob, GIT_FILEMODE_BLOB); if (rc) err("ins", rc); }
		int rc = git_treebuilder_insert(NULL, b, "a/b", &blob, GIT_FILEMODE_BLOB); err("ins slash", rc);
		rc = git_treebuilder_insert(NULL, b, ".git", &blob, GIT_FILEMODE_BLOB); err("ins .git", rc);
		rc = git_treebuilder_insert(NULL, b, "x", &blob, 0100664); err("ins 664", rc);
		rc = git_treebuilder_remove(b, "nope"); err("rm", rc);
		git_treebuilder_write(&sub, b);
		git_treebuilder_insert(NULL, b, "dir", &sub, GIT_FILEMODE_TREE);
		git_treebuilder_filter(b, filt, NULL);
		printf("count after filter=%zu\n", git_treebuilder_entrycount(b));
		git_treebuilder_insert(NULL, b, "zdir", &sub, GIT_FILEMODE_TREE);
		git_treebuilder_write(&top, b); printf("top=%s\n", git_oid_tostr_s(&top));
		git_treebuilder_free(b);
		git_tree_lookup(&tt, repo, &top);
		nsaved = 0;
		git_tree_walk(tt, GIT_TREEWALK_PRE, walk_cb, NULL);
		for (int i = 0; i < nsaved; i++)
			printf("walk saved %d: same_ptr=%d name=%s\n", i, saved[i] == git_tree_entry_byindex(tt, i), git_tree_entry_name(saved[i]));
		const char *paths[] = { "dir/a", "dir/", "dir", "dir/zz", "", "/dir", "dir//a", "a.b/x", NULL };
		for (int i = 0; paths[i]; i++) { git_tree_entry *e; rc = git_tree_entry_bypath(&e, tt, paths[i]); printf("bypath %s rc=%d\n", paths[i], rc); if (rc) err("  ", rc); else git_tree_entry_free(e); }
		struct { const char *p; int act; unsigned mode; } ups[][3] = {
			{{"dir/new", 0, GIT_FILEMODE_BLOB}, {"dir/a", 1, 0}, {"q/r/s", 0, GIT_FILEMODE_BLOB}},
			{{"dir", 0, GIT_FILEMODE_BLOB}, {NULL}},
			{{"a.b/x", 0, GIT_FILEMODE_BLOB}, {NULL}},
			{{"dir/a", 0, GIT_FILEMODE_BLOB}, {"dir/a", 1, 0}},
			{{"nope/x", 1, 0}, {NULL}},
			{{"dir//x", 0, GIT_FILEMODE_BLOB}, {NULL}},
			{{"dir/x/", 0, GIT_FILEMODE_BLOB}, {NULL}},
			{{"dir/a", 7, 0}, {NULL}},
			{{"dir/a", 1, 0}, {"dir/a.b", 1, 0}, {"dir/a-b", 1, 0}},
			{{"zdir/a", 1, 0}, {"zdir/a-b",1,0}, {"zdir/a.b",1,0}},
		};
		for (unsigned i = 0; i < sizeof ups / sizeof ups[0]; i++) {
			git_tree_update u[3]; size_t n = 0; git_oid out;
			for (int j = 0; j < 3 && ups[i][j].p; j++) { u[n].action = ups[i][j].act; u[n].id = blob; u[n].filemode = ups[i][j].mode; u[n].path = ups[i][j].p; n++; }
			rc = git_tree_create_updated(&out, repo, tt, n, u);
			printf("upd %u rc=%d %s\n", i, rc, rc ? "" : git_oid_tostr_s(&out)); if (rc) err("  ", rc);
		}
		git_tree_free(tt);
	}
	/* revwalk through a valid commit to a malformed parent */
	{
	const char *bad[] = { "tree xyz\nauthor a <b> 1 +0000\ncommitter a <b> 1 +0000\n\nm\n",
		"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nauthor a <b> 1 +0000\n\nm\n",
		"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\ncommitter a b> 1 +0000\n\nm\n",
		"tree 12\n", "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\ncommitter a <b> x +0000\n", NULL };
	for (int i = 0; bad[i]; i++) {
		git_oid p, c, out; char buf[512]; git_revwalk *w; int rc, n = 0;
		wr(&p, bad[i], GIT_OBJECT_COMMIT);
		snprintf(buf, sizeof buf, "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nparent %s\nauthor a <b> 1 +0000\ncommitter a <b> 1 +0000\n\nm\n", git_oid_tostr_s(&p));
		wr(&c, buf, GIT_OBJECT_COMMIT);
		{ git_commit *cc; rc = git_commit_lookup(&cc, repo, &c); if (rc) err("lookup", rc); else git_commit_free(cc); }
		git_revwalk_new(&w, repo); rc = git_revwalk_push(w, &c); if (rc) err("push", rc);
		while ((rc = git_revwalk_next(&out, w)) == 0) n++;
		printf("case %d n=%d ", i, n); err("next", rc);
		git_revwalk_free(w);
	}
	}
	more_creation(tree);
	git_tree_free(tree);
	git_repository_free(repo);
	return 0;
}
