/* difftest-link: libgit2_tests
 * grafts: parse corner cases (CRLF, blanks, junk, sha256, bad types),
 * map order under add/remove/replace, oids of empty, get/remove misses,
 * file refresh and checksum. usage: grafts2_diff <scratch dir> */
#include "common.h"
#include "git2.h"
#include "grafts.h"
#include "futils.h"

static unsigned long seed = 99;
static unsigned rnd(void) { seed = seed * 6364136223846793005UL + 1442695040888963407UL; return (unsigned)(seed >> 33); }
static void pq(const char *s, size_t n) { size_t i; for (i = 0; i < n; i++) { unsigned char c = s[i]; if (c >= 32 && c < 127 && c != '\\') putchar(c); else printf("\\x%02x", c); } }
static void dump(git_grafts *g)
{
	git_oid *oids = (git_oid *)0x1; size_t n = 77, i;
	int rc = git_grafts_oids(&oids, &n, g);
	printf(" oids rc=%d n=%zu ptr=%s size=%zu", rc, n, oids ? "set" : "NULL", git_grafts_size(g));
	for (i = 0; i < n; i++) {
		git_commit_graft *c = NULL; size_t p;
		git_grafts_get(&c, g, &oids[i]);
		printf(" [%.8s", git_oid_tostr_s(&oids[i]));
		for (p = 0; p < git_array_size(c->parents); p++) printf(" %.6s", git_oid_tostr_s(git_array_get(c->parents, p)));
		printf("]");
	}
	printf("\n");
	git__free(oids);
}
static void E(const char *w, int rc) { const git_error *e = git_error_last(); printf("%s rc=%d class=%d msg=%s", w, rc, e ? e->klass : -1, e ? e->message : "-"); git_error_clear(); }
static void pcase(git_oid_t ot, const char *buf, size_t len)
{
	git_grafts *g = NULL; int rc;
	rc = git_grafts_new(&g, ot);
	if (rc) { E("new", rc); printf("\n"); return; }
	rc = git_grafts_parse(g, buf, len);
	printf("ot=%d in=\"", ot); pq(buf, len); printf("\" "); E("parse", rc); dump(g);
	git_grafts_free(g);
}
static void randoid(git_oid *o, git_oid_t ot) { unsigned char raw[32]; size_t j; for (j = 0; j < 32; j++) raw[j] = rnd(); git_oid_from_raw(o, raw, ot); }

int main(int argc, char **argv)
{
	const char *A = "d7224d49d6d5aff6ade596ed74f4bcd4f77b29e2", *B = "8cda5e8e1d9d2e10e1e10c3d9a2a3a4d5e6f7081";
	const char *S = "d7224d49d6d5aff6ade596ed74f4bcd4f77b29e2d7224d49d6d5aff6ade596ed";
	const char *forms[] = { "%s\r\n", "%s %s\r\n", "\n%s\n", "%s\n\n%s\n", "%s\n \n", "%s #c\n", "%s %s#\n", "%s %s %s %s %s %s\n",
		"%s\n%s %s\n", "%s\x00\n", "%s %.39s\n", "%.39s\n", "%s0\n", "%s\n%s", "\r\n", "%s\t\n", "%s %s\n%s\n", NULL };
	char buf[2048], path[1024];
	size_t i, k;
	int rc;
	git_libgit2_init();
	for (i = 0; forms[i]; i++) {
		int n = snprintf(buf, sizeof(buf), forms[i], A, B, A, B, A, B);
		if (i == 9) n = 42;
		pcase(GIT_OID_SHA1, buf, n);
		n = snprintf(buf, sizeof(buf), forms[i], S, S, S, S, S, S);
		pcase(GIT_OID_SHA256, buf, n);
		n = snprintf(buf, sizeof(buf), forms[i], A, B, A, B, A, B);
		pcase(GIT_OID_SHA256, buf, n);
	}
	pcase(3, A, 40); pcase(3, "", 0); pcase(0, A, 40);
	{ git_grafts *g = NULL; rc = git_grafts_new(&g, 3); E("new type 3", rc); printf("\n"); git_grafts_free(g); }

	/* order under add/remove/replace */
	{
		git_grafts *g; git_oid ids[600]; git_array_oid_t par = GIT_ARRAY_INIT; git_oid *x;
		git_grafts_new(&g, GIT_OID_SHA1);
		printf("empty:"); dump(g);
		for (i = 0; i < 600; i++) randoid(&ids[i], GIT_OID_SHA1);
		x = git_array_alloc(par); *x = ids[0];
		for (k = 0; k < 5000; k++) {
			size_t j = rnd() % 600;
			switch (rnd() % 3) {
			case 0: case 1: rc = git_grafts_add(g, &ids[j], par); break;
			case 2: rc = git_grafts_remove(g, &ids[j]); break;
			}
			if (rc && rc != GIT_ENOTFOUND) { E("op", rc); printf("\n"); }
			if (k % 500 == 499) { printf("step %zu:", k); dump(g); }
		}
		{ git_commit_graft *c = (git_commit_graft *)0x5; git_oid z; randoid(&z, GIT_OID_SHA1); rc = git_grafts_get(&c, g, &z); printf("get miss out=%p ", (void *)c); E("", rc); printf("\n");
		  rc = git_grafts_remove(g, &z); E("remove miss", rc); printf("\n"); }
		git_grafts_clear(g); printf("cleared:"); dump(g);
		/* deterministic replace order */
		for (i = 0; i < 20; i++) git_grafts_add(g, &ids[i], par);
		git_grafts_add(g, &ids[3], par); git_grafts_remove(g, &ids[5]); git_grafts_add(g, &ids[5], par); git_grafts_add(g, &ids[100], par);
		printf("replace:"); dump(g);
		git_array_clear(par);
		git_grafts_free(g);
	}
	/* file refresh */
	{
		git_grafts *g = NULL;
		snprintf(path, sizeof(path), "%s/grafts", argv[1]);
		p_unlink(path);
		rc = git_grafts_open(&g, path, GIT_OID_SHA1); E("open missing", rc); dump(g);
		snprintf(buf, sizeof(buf), "%s %s\n", A, B); git_futils_writebuffer(&(git_str){ buf, 0, strlen(buf) }, path, 0, 0644);
		rc = git_grafts_refresh(g); E("refresh new", rc); dump(g);
		rc = git_grafts_refresh(g); E("refresh same", rc); dump(g);
		snprintf(buf, sizeof(buf), "%s\nzz\n", B); git_futils_writebuffer(&(git_str){ buf, 0, strlen(buf) }, path, 0, 0644);
		rc = git_grafts_refresh(g); E("refresh bad", rc); dump(g);
		rc = git_grafts_refresh(g); E("refresh bad again", rc); dump(g);
		p_unlink(path);
		rc = git_grafts_refresh(g); E("refresh gone", rc); dump(g);
		snprintf(buf, sizeof(buf), "%s\nzz\n", B); git_futils_writebuffer(&(git_str){ buf, 0, strlen(buf) }, path, 0, 0644);
		rc = git_grafts_refresh(g); E("refresh recreated same", rc); dump(g);
		rc = git_grafts_open_or_refresh(&g, path, GIT_OID_SHA1); E("open_or_refresh existing", rc); dump(g);
		git_grafts_free(g); g = NULL;
		rc = git_grafts_open_or_refresh(&g, path, GIT_OID_SHA1); E("open_or_refresh new (bad file)", rc); printf(" g=%s\n", g ? "set" : "NULL");
		git_grafts_free(g); g = NULL;
		p_mkdir(path, 0755); p_unlink(path);
		{ char d[1100]; snprintf(d, sizeof(d), "%s/dir", argv[1]); p_mkdir(d, 0755); rc = git_grafts_open(&g, d, GIT_OID_SHA1); E("open dir", rc); printf("\n"); git_grafts_free(g); g = NULL; }
		/* in-memory refresh */
		git_grafts_new(&g, GIT_OID_SHA1); rc = git_grafts_refresh(g); E("refresh mem", rc); printf("\n"); git_grafts_free(g);
	}
	git_libgit2_shutdown();
	return 0;
}
