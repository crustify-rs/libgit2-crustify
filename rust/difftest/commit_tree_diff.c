/*
 * Differential test of work package l-commit: every line printed must be the
 * same when linked against the C reference and against the Rust port.
 *
 * Parses commits and trees with git_object__from_raw (and the internal
 * git_commit__parse_raw / git_tree__parse_raw, quick parses included) over:
 * the fuzzers' object corpora, every object of the test resources'
 * repositories, hand-written edge cases, every truncation of a few seeds and
 * many random mutations of them.  For each it prints the status, the error,
 * and every field (ids, signatures, encoding, raw header and message,
 * summary, body, header fields, tree entries and lookups by name).
 *
 * difftest-link: libgit2_tests
 *
 * Build and compare:
 *   RUST_BUILD=/work/builds/l-commit rust/difftest/run.sh rust/difftest/commit_tree_diff.c
 */

#include "common.h"
#include "git2.h"
#include "commit.h"
#include "tree.h"
#include "object.h"
#include "odb.h"
#include "futils.h"

#include <dirent.h>

static git_repository *g_repo;
static unsigned long seed = 777;
static unsigned rnd(void)
{
	seed = seed * 6364136223846793005UL + 1442695040888963407UL;
	return (unsigned)(seed >> 33);
}

static unsigned long fnv(const void *p, size_t n)
{
	const unsigned char *s = p;
	unsigned long h = 1469598103934665603UL;
	while (n--)
		h = (h ^ *s++) * 1099511628211UL;
	return h;
}

static void show_err(int rc)
{
	const git_error *e = git_error_last();
	printf("  rc=%d class=%d msg=%s\n", rc, e ? e->klass : -1, e ? e->message : "");
	git_error_clear();
}

static void show_str(const char *what, const char *s)
{
	if (!s) {
		printf("  %s=NULL\n", what);
		return;
	}
	printf("  %s len=%zu h=%lx\n", what, strlen(s), fnv(s, strlen(s)));
	if (strlen(s) < 80 && !strchr(s, '\n'))
		printf("  %s='%s'\n", what, s);
}

static void show_sig(const char *what, const git_signature *s)
{
	if (!s) {
		printf("  %s=NULL\n", what);
		return;
	}
	printf("  %s name=%s email=%s time=%lld offset=%d sign=%d\n", what,
		s->name ? s->name : "(null)", s->email ? s->email : "(null)",
		(long long)s->when.time, s->when.offset, s->when.sign);
}

static void show_commit(git_commit *c)
{
	char hex[GIT_OID_MAX_HEXSIZE + 1];
	static const char *fields[] = { "tree", "parent", "author", "committer", "encoding", "gpgsig", "mergetag", "x", "", "author ", NULL };
	git_buf b = GIT_BUF_INIT;
	unsigned i;
	int rc;

	printf("  tree=%s parents=%u\n", git_oid_tostr(hex, sizeof(hex), git_commit_tree_id(c)), git_commit_parentcount(c));
	for (i = 0; i < git_commit_parentcount(c); i++)
		printf("  parent %s\n", git_oid_tostr(hex, sizeof(hex), git_commit_parent_id(c, i)));
	show_sig("author", git_commit_author(c));
	show_sig("committer", git_commit_committer(c));
	printf("  time=%lld offset=%d\n", (long long)git_commit_time(c), git_commit_time_offset(c));
	show_str("encoding", git_commit_message_encoding(c));
	show_str("raw_header", git_commit_raw_header(c));
	show_str("raw_message", git_commit_message_raw(c));
	show_str("message", git_commit_message(c));
	show_str("summary", git_commit_summary(c));
	show_str("summary2", git_commit_summary(c));
	show_str("body", git_commit_body(c));
	for (i = 0; fields[i]; i++) {
		rc = git_commit_header_field(&b, c, fields[i]);
		printf("  field %s rc=%d\n", fields[i], rc);
		if (rc == 0)
			show_str("value", b.ptr);
		else
			show_err(rc);
		git_buf_dispose(&b);
	}
}

static void show_tree(git_tree *t)
{
	char hex[GIT_OID_MAX_HEXSIZE + 1];
	size_t i, n = git_tree_entrycount(t);
	const git_tree_entry *e;

	printf("  entries=%zu\n", n);
	for (i = 0; i < n; i++) {
		e = git_tree_entry_byindex(t, i);
		printf("  %06o %06o %d %s len=%u %s\n", git_tree_entry_filemode_raw(e),
			git_tree_entry_filemode(e), git_tree_entry_type(e),
			git_oid_tostr(hex, sizeof(hex), git_tree_entry_id(e)),
			e->filename_len, git_tree_entry_name(e));
		/* lookups by name of each entry, and of near misses */
		printf("   byname=%d", git_tree_entry_byname(t, git_tree_entry_name(e)) ? (int)(git_tree_entry_byname(t, git_tree_entry_name(e)) - git_tree_entry_byindex(t, 0)) : -1);
		printf(" byid=%d", git_tree_entry_byid(t, git_tree_entry_id(e)) ? (int)(git_tree_entry_byid(t, git_tree_entry_id(e)) - git_tree_entry_byindex(t, 0)) : -1);
		if (i > 0)
			printf(" cmp=%d", git_tree_entry_cmp(git_tree_entry_byindex(t, i - 1), e) > 0 ? 1 : (git_tree_entry_cmp(git_tree_entry_byindex(t, i - 1), e) < 0 ? -1 : 0));
		printf("\n");
	}
	{
		static const char *names[] = { "", "a", "README", "src", "src/", "zzzz", "new.txt", "a/b", NULL };
		for (i = 0; names[i]; i++) {
			e = git_tree_entry_byname(t, names[i]);
			printf("  name %s -> %d\n", names[i], e ? (int)(e - git_tree_entry_byindex(t, 0)) : -1);
		}
	}
}

static void parse_one(const char *label, const char *data, size_t len)
{
	git_object *obj = NULL;
	git_commit *c;
	int rc;

	printf("%s len=%zu h=%lx\n", label, len, fnv(data, len));

	rc = git_object__from_raw(&obj, data, len, GIT_OBJECT_COMMIT, GIT_OID_SHA1);
	printf(" commit rc=%d\n", rc);
	if (rc < 0)
		show_err(rc);
	else
		show_commit((git_commit *)obj);
	git_object_free(obj);
	obj = NULL;

	/* quick parse */
	c = git__calloc(1, sizeof(git_commit));
	c->object.repo = g_repo;
	{
		git_odb_object fake;
		git_commit__parse_options opts = { GIT_OID_SHA1, GIT_COMMIT_PARSE_QUICK };
		char *copy = git__malloc(len + 1);
		memcpy(copy, data, len);
		copy[len] = '\0';
		memset(&fake, 0, sizeof(fake));
		fake.cached.size = len;
		fake.cached.type = GIT_OBJECT_COMMIT;
		fake.buffer = copy;
		rc = git_commit__parse_ext(c, &fake, &opts);
		printf(" quick rc=%d parents=%u", rc, (unsigned)c->parent_ids.size);
		if (rc == 0)
			printf(" time=%lld", (long long)c->committer->when.time);
		printf(" header=%d\n", c->raw_header != NULL);
		if (rc < 0)
			show_err(rc);
		git__free(copy);
	}
	git_commit__free(c);

	rc = git_object__from_raw(&obj, data, len, GIT_OBJECT_TREE, GIT_OID_SHA1);
	printf(" tree rc=%d\n", rc);
	if (rc < 0)
		show_err(rc);
	else
		show_tree((git_tree *)obj);
	git_object_free(obj);

	/* SHA-256 */
	obj = NULL;
	rc = git_object__from_raw(&obj, data, len, GIT_OBJECT_COMMIT, GIT_OID_SHA256);
	printf(" commit256 rc=%d", rc);
	if (rc == 0)
		printf(" parents=%u", git_commit_parentcount((git_commit *)obj));
	printf("\n");
	git_error_clear();
	git_object_free(obj);
	obj = NULL;
	rc = git_object__from_raw(&obj, data, len, GIT_OBJECT_TREE, GIT_OID_SHA256);
	printf(" tree256 rc=%d", rc);
	if (rc == 0)
		printf(" entries=%zu", git_tree_entrycount((git_tree *)obj));
	printf("\n");
	git_error_clear();
	git_object_free(obj);
}

static void mutate_all(const char *label, const char *data, size_t len, int rounds)
{
	char name[128];
	char *buf = git__malloc(len + 64);
	size_t i;
	int r;

	/* every truncation */
	for (i = 0; i < len; i++) {
		snprintf(name, sizeof(name), "%s trunc %zu", label, i);
		parse_one(name, data, i);
	}
	/* random mutations: byte flips, insertions of interesting bytes */
	for (r = 0; r < rounds; r++) {
		static const char interesting[] = { '\n', ' ', '\0', '<', '>', '-', '+', '0', '9', 'a', '/', '\t' };
		size_t n = len, k, nmut = 1 + rnd() % 4;
		memcpy(buf, data, len);
		for (k = 0; k < nmut && n > 0; k++) {
			size_t at = rnd() % n;
			switch (rnd() % 4) {
			case 0: buf[at] = interesting[rnd() % sizeof(interesting)]; break;
			case 1: buf[at] = (char)rnd(); break;
			case 2: /* delete */
				memmove(buf + at, buf + at + 1, n - at - 1); n--; break;
			case 3: /* insert */
				if (n < len + 60) {
					memmove(buf + at + 1, buf + at, n - at);
					buf[at] = interesting[rnd() % sizeof(interesting)];
					n++;
				}
				break;
			}
		}
		snprintf(name, sizeof(name), "%s mut %d", label, r);
		parse_one(name, buf, n);
	}
	git__free(buf);
}

static const char *cases[] = {
	"tree 1810dff58d8a660512d4832e740f692884338ccd\nauthor A <a@b> 1 +0000\ncommitter C <c@d> 2 -0130\n\nmsg\n",
	"tree 1810dff58d8a660512d4832e740f692884338ccd\nparent 1810dff58d8a660512d4832e740f692884338ccd\nparent 0000000000000000000000000000000000000000\nauthor A <a@b> 1 +0000\nauthor B <b@b> 1 +0000\ncommitter C <c@d> 2 -0130\nencoding ISO-8859-1\nencoding UTF-8\ngpgsig -----BEGIN\n line2\n \n end\nx y\n\n\n\n  Summary   line\n  continues here\n\n\n  body  \n\n more\n\n",
	"tree 1810dff58d8a660512d4832e740f692884338ccd\nauthor <> 1\ncommitter  , <> \n",
	"tree 1810dff58d8a660512d4832e740f692884338ccd\nauthor A <a@b> 99999999999999999999 +0000\ncommitter C <c@d> 2 +9999\n\n",
	"tree 1810dff58d8a660512d4832e740f692884338ccd\nauthor A <a@b> 12 +1460\ncommitter C <c@d> -5 x0100\n\nsummary\twith\ttabs\r\nand cr\n \n\t\nbody\n",
	"tree 1810dff58d8a660512d4832e740f692884338ccd\nauthor A <a@b> 12 +0100\ncommitter C <c@d> 13 -0000\n\n\n\n",
	"tree 1810dff58d8a660512d4832e740f692884338ccd\nauthor A <a@b> 12 +0100\ncommitter C <c@d> 13 -0000\n",
	"tree 1810dff58d8a660512d4832e740f692884338ccd\nauthor A <a@b> 12 +0100\ncommitter C <c@d> 13 -0000\nencoding ",
	"tree 1810dff58d8a660512d4832e740f692884338ccd\nparent zz10dff58d8a660512d4832e740f692884338ccd\nauthor A <a@b> 12 +0100\ncommitter C <c@d> 13\n\nm",
	"tree 1810dff58d8a660512d4832e740f692884338ccd\nauthor A >a@b< 12 +0100\ncommitter C <c@d> 13\n\nm",
	"tree 1810dff58d8a660512d4832e740f692884338ccd\nauthor A <a@b>\ncommitter C <c@d> 13  +0100\n\none line summary that is quite a bit longer than the others",
	"tree 1810dff58d8a660512d4832e740f692884338ccd\nauthor A <a@b> 1 +0000\ncommitter C <c@d> 2 +0000\nmergetag object 1\n type commit\n\nmsg",
	NULL
};

static void read_file_and_parse(const char *path, int rounds)
{
	git_str buf = GIT_STR_INIT;
	if (git_futils_readbuffer(&buf, path) < 0) {
		git_error_clear();
		return;
	}
	parse_one(path, buf.ptr, buf.size);
	if (rounds)
		mutate_all(path, buf.ptr, buf.size, rounds);
	git_str_dispose(&buf);
}

static int each_object(const git_oid *id, void *payload)
{
	git_odb *odb = payload;
	git_odb_object *obj;
	char hex[GIT_OID_MAX_HEXSIZE + 1];
	if (git_odb_read(&obj, odb, id) < 0) {
		git_error_clear();
		return 0;
	}
	if (git_odb_object_type(obj) == GIT_OBJECT_COMMIT || git_odb_object_type(obj) == GIT_OBJECT_TREE)
		parse_one(git_oid_tostr(hex, sizeof(hex), id), git_odb_object_data(obj), git_odb_object_size(obj));
	git_odb_object_free(obj);
	return 0;
}

static void repo_objects(const char *path)
{
	git_repository *repo;
	git_odb *odb;
	if (git_repository_open_bare(&repo, path) < 0) {
		printf("cannot open %s\n", path);
		git_error_clear();
		return;
	}
	git_repository_odb(&odb, repo);
	printf("repo %s\n", path);
	git_odb_foreach(odb, each_object, odb);
	git_odb_free(odb);
	git_repository_free(repo);
}

static void tree_cases(void)
{
	static const char t1[] = "100644 a\0aaaaaaaaaaaaaaaaaaaa" "40000 a.b\0bbbbbbbbbbbbbbbbbbbb" "40000 a\0cccccccccccccccccccc" "160000 sub\0dddddddddddddddddddd" "120000 z\0eeeeeeeeeeeeeeeeeeee";
	static const char t2[] = "100664 legacy\0aaaaaaaaaaaaaaaaaaaa" "100755 x\0bbbbbbbbbbbbbbbbbbbb" "644 lowbits\0cccccccccccccccccccc" "0100644 zero\0dddddddddddddddddddd" "+100644 plus\0eeeeeeeeeeeeeeeeeeee";
	static const char t3[] = " 100644 sp\0aaaaaaaaaaaaaaaaaaaa";
	static const char t4[] = "1777777 big\0aaaaaaaaaaaaaaaaaaaa";
	static const char t5[] = "100644  twospace\0aaaaaaaaaaaaaaaaaaaa";
	static const char t6[] = "100644 \0aaaaaaaaaaaaaaaaaaaa";
	static const char t7[] = "100644 short\0aaaaaaaaaaaaaaaaaaa";
	static const char t8[] = "100644 noname";
	static const char t9[] = "10064x name\0aaaaaaaaaaaaaaaaaaaa";
	static const char t10[] = "177777 m\0aaaaaaaaaaaaaaaaaaaa" "00000000000000100644 m2\0aaaaaaaaaaaaaaaaaaaa";
	parse_one("t1", t1, sizeof(t1) - 1);
	parse_one("t2", t2, sizeof(t2) - 1);
	parse_one("t3", t3, sizeof(t3) - 1);
	parse_one("t4", t4, sizeof(t4) - 1);
	parse_one("t5", t5, sizeof(t5) - 1);
	parse_one("t6", t6, sizeof(t6) - 1);
	parse_one("t7", t7, sizeof(t7) - 1);
	parse_one("t8", t8, sizeof(t8) - 1);
	parse_one("t9", t9, sizeof(t9) - 1);
	parse_one("t10", t10, sizeof(t10) - 1);
	mutate_all("t1", t1, sizeof(t1) - 1, 3000);
	mutate_all("t2", t2, sizeof(t2) - 1, 1000);
}

int main(void)
{
	int i;
	char name[32];

	git_libgit2_init();
	if (git_repository_open_bare(&g_repo, "tests/resources/testrepo.git") < 0)
		return 1;

	for (i = 0; cases[i]; i++) {
		snprintf(name, sizeof(name), "case %d", i);
		parse_one(name, cases[i], strlen(cases[i]));
	}
	mutate_all("case 1", cases[1], strlen(cases[1]), 3000);
	mutate_all("case 4", cases[4], strlen(cases[4]), 1000);
	tree_cases();

	read_file_and_parse("fuzzers/corpora/objects/commit", 2000);
	read_file_and_parse("fuzzers/corpora/objects/tree", 2000);
	read_file_and_parse("fuzzers/corpora/objects/tag", 0);
	read_file_and_parse("fuzzers/corpora/objects/blob", 0);

	repo_objects("tests/resources/testrepo.git");
	repo_objects("tests/resources/merge-resolve/.gitted");
	repo_objects("tests/resources/attr/.gitted");
	repo_objects("tests/resources/shallow.git");

	git_repository_free(g_repo);
	git_libgit2_shutdown();
	return 0;
}
