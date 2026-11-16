/*
 * Review l-obj r1: object.c lookup/peel/short_id/bypath/id_from_* against
 * the C.  difftest-link: libgit2_tests
 */
#include "common.h"
#include "git2.h"
#include "git2/sys/repository.h"
#include "object.h"
#include <stdio.h>
#include <unistd.h>
#include <fcntl.h>

#ifndef LOBJ_REPOS
# define LOBJ_REPOS "/tmp/lobjrepos"
#endif

static void err(const char *what, int rc)
{
	const git_error *e = git_error_last();
	printf("%s rc=%d class=%d msg=%s\n", what, rc, e ? e->klass : -1,
	       e && e->message ? e->message : "(null)");
	git_error_clear();
}

#define MAXO 4096
static git_oid oids[MAXO]; static int noids;
static int collect(const git_oid *id, void *p) { (void)p; if (noids < MAXO) oids[noids++] = *id; return 0; }
static int cmpoid(const void *a, const void *b) { return git_oid_cmp(a, b); }

static void do_repo(const char *path)
{
	git_repository *repo; git_odb *odb; int i, rc;
	git_object_t types[] = { GIT_OBJECT_ANY, GIT_OBJECT_INVALID, 0, GIT_OBJECT_COMMIT, GIT_OBJECT_TREE, GIT_OBJECT_BLOB, GIT_OBJECT_TAG, 5, 6, 7, 42 };
	size_t ti, len;
	printf("== %s\n", path);
	if ((rc = git_repository_open(&repo, path)) < 0) { err("open", rc); return; }
	git_repository_odb(&odb, repo);
	noids = 0;
	if (strstr(path, "_256") && getenv("IDLIST")) {
		FILE *f = fopen(getenv("IDLIST"), "r"); char line[200];
		while (f && fgets(line, sizeof line, f) && noids < MAXO) { line[64] = 0; git_oid_from_string(&oids[noids++], line, GIT_OID_SHA256); }
		if (f) fclose(f);
	} else
	git_odb_foreach(odb, collect, NULL);
	qsort(oids, noids, sizeof(git_oid), cmpoid);
	printf("objects %d\n", noids);
	for (i = 0; i < noids; i++) {
		char hex[GIT_OID_MAX_HEXSIZE + 1];
		git_object *o = NULL;
		git_oid_tostr(hex, sizeof(hex), &oids[i]);
		/* lookup with every type */
		for (ti = 0; ti < sizeof(types)/sizeof(types[0]); ti++) {
			rc = git_object_lookup(&o, repo, &oids[i], types[ti]);
			printf("%s lookup t=%d rc=%d", hex, types[ti], rc);
			if (rc == 0) { printf(" type=%d\n", git_object_type(o)); git_object_free(o); o = NULL; }
			else { printf("\n"); err("  e", rc); }
		}
		/* lookup_prefix at a few lengths */
		for (len = 0; len <= 66; len += (len < 12 ? 1 : 7)) {
			rc = git_object_lookup_prefix(&o, repo, &oids[i], len, GIT_OBJECT_ANY);
			printf("%s prefix len=%zu rc=%d", hex, len, rc);
			if (rc == 0) { printf(" id=%s\n", git_oid_tostr_s(git_object_id(o))); git_object_free(o); o = NULL; }
			else { printf("\n"); err("  e", rc); }
		}
		if ((rc = git_object_lookup(&o, repo, &oids[i], GIT_OBJECT_ANY)) == 0) {
			git_buf b = GIT_BUF_INIT;
			git_object_t pt[] = { GIT_OBJECT_ANY, GIT_OBJECT_COMMIT, GIT_OBJECT_TREE, GIT_OBJECT_BLOB, GIT_OBJECT_TAG };
			size_t k;
			for (k = 0; k < 5; k++) {
				git_object *p = NULL;
				rc = git_object_peel(&p, o, pt[k]);
				printf("%s peel %d rc=%d", hex, pt[k], rc);
				if (rc == 0) { printf(" -> %s %d\n", git_oid_tostr_s(git_object_id(p)), git_object_type(p)); git_object_free(p); }
				else { printf("\n"); err("  e", rc); }
			}
			rc = git_object_short_id(&b, o);
			printf("%s short rc=%d %s\n", hex, rc, rc == 0 ? b.ptr : "");
			git_buf_dispose(&b);
			{
				const char *paths[] = { "README", "new.txt", "subdir", "subdir/README", "a/b/c", "", "/", "subdir/", "branch_file.txt", "link_to_new.txt" };
				for (k = 0; k < sizeof(paths)/sizeof(paths[0]); k++) {
					git_object_t bt[] = { GIT_OBJECT_ANY, GIT_OBJECT_BLOB, GIT_OBJECT_TREE, GIT_OBJECT_COMMIT };
					size_t m;
					for (m = 0; m < 4; m++) {
						git_object *p = NULL;
						rc = git_object_lookup_bypath(&p, o, paths[k], bt[m]);
						if (rc == 0) { printf("%s bypath '%s' %d -> %s\n", hex, paths[k], bt[m], git_oid_tostr_s(git_object_id(p))); git_object_free(p); }
						else { printf("%s bypath '%s' %d rc=%d\n", hex, paths[k], bt[m], rc); err("  e", rc); }
					}
				}
			}
			git_object_free(o);
		} else err("lookup", rc);
		/* is_valid */
		for (ti = 0; ti < sizeof(types)/sizeof(types[0]); ti++) {
			bool v = git_object__is_valid(repo, &oids[i], types[ti]);
			printf("%s valid t=%d %d\n", hex, types[ti], v);
			err("  e", 0);
		}
	}
	/* bogus ids */
	{
		git_oid z; git_object *o = NULL;
		memset(&z, 0, sizeof(z)); z.type = git_repository_oid_type(repo);
		for (len = 0; len <= 66; len += 5) {
			rc = git_object_lookup_prefix(&o, repo, &z, len, GIT_OBJECT_ANY);
			printf("zero prefix %zu rc=%d\n", len, rc); err("  e", rc);
		}
		if (noids) {
			git_oid a = oids[0];
			for (len = 4; len <= 10; len++) {
				rc = git_object_lookup_prefix(&o, repo, &a, len, GIT_OBJECT_BLOB);
				printf("first prefix %zu blob rc=%d\n", len, rc); err("  e", rc);
				if (rc == 0) git_object_free(o);
			}
		}
		/* abbrev settings */
		{
			git_config *cfg; const char *ab[] = { "4", "7", "12", "40", "64", "3", "41", "auto", "no", "false", "x" };
			size_t k;
			git_repository_config(&cfg, repo);
			for (k = 0; k < sizeof(ab)/sizeof(ab[0]) && noids; k++) {
				git_buf b = GIT_BUF_INIT; git_object *o2 = NULL;
				git_config_set_string(cfg, "core.abbrev", ab[k]);
				git_object_lookup(&o2, repo, &oids[noids/2], GIT_OBJECT_ANY);
				rc = git_object_short_id(&b, o2);
				printf("abbrev %s rc=%d %s\n", ab[k], rc, rc == 0 ? b.ptr : ""); err("  e", rc);
				git_buf_dispose(&b); git_object_free(o2);
			}
			git_config_delete_entry(cfg, "core.abbrev");
			git_config_free(cfg);
		}
	}
	git_odb_free(odb);
	git_repository_free(repo);
}

static void ids(void)
{
	git_object_id_options o = GIT_OBJECT_ID_OPTIONS_INIT; git_oid id; int rc, t, v;
	char path[] = "/tmp/lobj_r1_idXXXXXX"; int fd;
	const char *data = "hello\r\nworld\n";
	for (v = -1; v < 4; v++) {
		git_object_id_options x; memset(&x, 0xaa, sizeof(x));
		rc = git_object_id_options_init(&x, (unsigned)v);
		printf("init v=%d rc=%d ver=%u ot=%d oid=%d f=%p\n", v, rc, x.version, x.object_type, x.oid_type, rc == 0 ? (void*)x.filters : NULL); err("  e", rc);
	}
	for (t = -3; t < 9; t++) {
		int ot;
		for (ot = -1; ot < 4; ot++) {
			o.object_type = t; o.oid_type = ot;
			rc = git_object_id_from_buffer(&id, data, strlen(data), &o);
			printf("buf t=%d ot=%d rc=%d %s\n", t, ot, rc, rc == 0 ? git_oid_tostr_s(&id) : ""); err("  e", rc);
		}
	}
	rc = git_object_id_from_buffer(&id, NULL, 0, NULL); printf("null0 rc=%d %s\n", rc, rc ? "" : git_oid_tostr_s(&id)); err("  e", rc);
	rc = git_object_id_from_buffer(&id, NULL, 3, NULL); printf("null3 rc=%d\n", rc); err("  e", rc);
	fd = mkstemp(path); write(fd, data, strlen(data)); close(fd);
	o.object_type = 0; o.oid_type = 0;
	rc = git_object_id_from_file(&id, path, &o); printf("file rc=%d %s\n", rc, rc ? "" : git_oid_tostr_s(&id)); err("  e", rc);
	rc = git_object_id_from_file(&id, "/nonexistent/x", &o); printf("file-missing rc=%d\n", rc); err("  e", rc);
	rc = git_object_id_from_file(&id, "/tmp", &o); printf("file-dir rc=%d\n", rc); err("  e", rc);
	fd = open(path, O_RDONLY);
	rc = git_object_id_from_fd(&id, fd, strlen(data), &o); printf("fd rc=%d %s\n", rc, rc ? "" : git_oid_tostr_s(&id)); err("  e", rc); close(fd);
	fd = open(path, O_RDONLY);
	rc = git_object_id_from_fd(&id, fd, strlen(data) + 5, &o); printf("fd-short rc=%d\n", rc); err("  e", rc); close(fd);
	fd = open(path, O_RDONLY);
	rc = git_object_id_from_fd(&id, fd, 3, &o); printf("fd-long rc=%d\n", rc); err("  e", rc); close(fd);
	rc = git_object_id_from_fd(&id, -1, 3, &o); printf("fd-bad rc=%d\n", rc); err("  e", rc);
	rc = git_object_id_from_fd(&id, -1, 0, &o); printf("fd-bad0 rc=%d %s\n", rc, rc ? "" : git_oid_tostr_s(&id)); err("  e", rc);
	{
		char lpath[64]; snprintf(lpath, sizeof lpath, "%s.lnk", path); unlink(lpath);
		symlink("some/target/path", lpath);
		rc = git_object_id_from_symlink(&id, lpath, &o); printf("symlink rc=%d %s\n", rc, rc ? "" : git_oid_tostr_s(&id)); err("  e", rc);
		rc = git_object_id_from_symlink(&id, path, &o); printf("symlink-file rc=%d %s\n", rc, rc ? "" : git_oid_tostr_s(&id)); err("  e", rc);
		rc = git_object_id_from_symlink(&id, "/nonexistent/q", &o); printf("symlink-missing rc=%d\n", rc); err("  e", rc);
		o.object_type = GIT_OBJECT_TREE;
		rc = git_object_id_from_symlink(&id, lpath, &o); printf("symlink-tree rc=%d\n", rc); err("  e", rc);
		o.object_type = 0;
		unlink(lpath);
	}
	unlink(path);
	{
		const char *s[] = { "commit", "tree", "blob", "tag", "", "comm", "commitx", "TAG", "ofs-delta", "ref-delta", "any", "invalid" };
		size_t k;
		for (k = 0; k < sizeof(s)/sizeof(s[0]); k++) printf("s2t %s %d n3=%d\n", s[k], git_object_string2type(s[k]), git_object_stringn2type(s[k], 3));
		printf("s2t NULL %d\n", git_object_string2type(NULL));
		for (t = -3; t < 9; t++) printf("t2s %d '%s' valid=%d\n", t, git_object_type2string(t), git_object_type_is_valid(t));
	}
}

int main(void)
{
	git_libgit2_init();
	ids();
	do_repo(LOBJ_REPOS "/testrepo.git");
	do_repo(LOBJ_REPOS "/duplicate.git");
	do_repo(LOBJ_REPOS "/peeled.git");
	do_repo(LOBJ_REPOS "/bad_tag.git");
	do_repo(LOBJ_REPOS "/short_tag.git");
	do_repo(LOBJ_REPOS "/testrepo_256.git");
	git_libgit2_opts(GIT_OPT_ENABLE_STRICT_OBJECT_CREATION, 0);
	{
		git_repository *r; git_oid z; memset(&z, 0x11, sizeof z); z.type = 1;
		git_repository_open(&r, LOBJ_REPOS "/testrepo.git");
		printf("nonstrict valid %d\n", git_object__is_valid(r, &z, GIT_OBJECT_TAG));
		git_repository_free(r);
	}
	git_libgit2_shutdown();
	return 0;
}
