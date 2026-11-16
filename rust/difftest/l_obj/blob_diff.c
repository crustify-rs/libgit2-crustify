/*
 * Review driver for l-obj blob.c port. difftest-link: libgit2_tests
 * usage: blob_diff <scratchdir>
 */
#include "common.h"
#include "git2.h"
#include "git2/sys/filter.h"
#include "object.h"
#include "blob.h"
#include "str.h"
#include "futils.h"
#include <dirent.h>
#include <sys/stat.h>
#include <unistd.h>

static void err(const char *what, int rc)
{
	const git_error *e = git_error_last();
	printf("%-40s rc=%d klass=%d msg=[%s]\n", what, rc, e ? e->klass : -99,
	       e && e->message ? e->message : "(null)");
	git_error_clear();
}

static char root[4096];
static git_repository *repo;
static git_oid id;

static void wfile(const char *rel, const void *data, size_t len)
{
	char p[4200];
	FILE *f;
	snprintf(p, sizeof(p), "%s/w/%s", root, rel);
	f = fopen(p, "wb");
	fwrite(data, 1, len, f);
	fclose(f);
}

static void list_streamed(const char *what)
{
	char p[4200];
	DIR *d;
	struct dirent *de;
	int n = 0;
	snprintf(p, sizeof(p), "%s/w/.git/objects", root);
	d = opendir(p);
	while ((de = readdir(d)) != NULL)
		if (strncmp(de->d_name, "streamed", 8) == 0)
			n++;
	closedir(d);
	printf("%-40s leftover streamed files=%d\n", what, n);
}

static void pid(const char *what, int rc)
{
	char s[GIT_OID_MAX_HEXSIZE + 1] = "";
	if (rc == 0)
		git_oid_tostr(s, sizeof(s), &id);
	printf("%-40s id=%s\n", what, s);
	err(what, rc);
}

static void bin_case(const char *what, const char *d, size_t len)
{
	int rc = git_blob_data_is_binary(d, len);
	int rc2 = -5, rc3 = -5;
	git_blob *b = NULL;
	git_object *o = NULL;
	if (git_blob_create_from_buffer(&id, repo, d, len) == 0 &&
	    git_blob_lookup(&b, repo, &id) == 0) {
		rc2 = git_blob_is_binary(b);
		git_blob_free(b);
	}
	if (git_object__from_raw(&o, d, len, GIT_OBJECT_BLOB, GIT_OID_SHA1) == 0) {
		rc3 = git_blob_is_binary((git_blob *)o);
		if (git_blob_rawsize((git_blob *)o) != len ||
		    git_blob_rawcontent((git_blob *)o) != (const void *)d)
			printf("raw mismatch\n");
		git_object_free(o);
	}
	printf("bin %-30s len=%zu data=%d blob=%d raw=%d\n", what, len, rc, rc2, rc3);
	git_error_clear();
}

static unsigned long seed = 99;
static unsigned rnd(void)
{
	seed = seed * 6364136223846793005UL + 1442695040888963407UL;
	return (unsigned)(seed >> 33);
}

static void binary_tests(void)
{
	static char buf[20000];
	size_t sizes[] = { 0, 1, 127, 128, 129, 255, 256, 7999, 8000, 8001, 16000, 20000 };
	size_t i, j, k;
	bin_case("null0", NULL, 0);
	bin_case("text", "hello\nworld\r\n", 13);
	bin_case("nul", "a\0b", 3);
	bin_case("bom8", "\xEF\xBB\xBFhi", 5);
	bin_case("bom8only", "\xEF\xBB\xBF", 3);
	bin_case("bom8short", "\xEF\xBB", 2);
	bin_case("bom16le", "\xFF\xFEh\0i\0", 6);
	bin_case("bom16be", "\xFE\xFF\0h\0i", 6);
	bin_case("bom32le", "\xFF\xFE\0\0h\0\0\0", 8);
	bin_case("bom32be", "\0\0\xFE\xFF\0\0\0h", 8);
	bin_case("cr", "\r\r\r\r", 4);
	bin_case("ctrl1", "\x01", 1);
	bin_case("ctrl_vt", "\x0b\x0b", 2);
	bin_case("del", "\x7f", 1);
	bin_case("eof", "abc\x1a", 4);
	bin_case("high", "\xff\xfe\xfd", 3);
	for (i = 0; i < sizeof(sizes) / sizeof(sizes[0]); i++) {
		size_t n = sizes[i];
		/* all text, then a NUL at a few positions, then control ratio */
		memset(buf, 'a', n);
		bin_case("all-a", buf, n);
		for (k = 0; k < 4; k++) {
			size_t pos[] = { 0, 7999, 8000, n ? n - 1 : 0 };
			if (pos[k] >= n)
				continue;
			memset(buf, 'a', n);
			buf[pos[k]] = 0;
			bin_case("nul@", buf, n);
		}
		/* ratio of nonprintable: 1 in 128 vs 1 in 127 */
		for (k = 120; k < 135; k++) {
			memset(buf, 'a', n);
			for (j = 0; j < n; j += k)
				buf[j] = 1;
			bin_case("ctrl-ratio", buf, n);
		}
	}
	for (i = 0; i < 2000; i++) {
		size_t n = rnd() % 9000;
		unsigned mode = rnd() % 4;
		for (j = 0; j < n; j++) {
			unsigned r = rnd();
			if (mode == 0) buf[j] = (char)r;
			else if (mode == 1) buf[j] = (r % 200 == 0) ? (char)(r % 32) : 'x';
			else if (mode == 2) buf[j] = (r % 100 == 0) ? 0 : (char)(0x80 | r);
			else buf[j] = (r % 3 == 0) ? '\r' : (char)(0x20 + r % 90);
		}
		if (rnd() % 5 == 0 && n >= 3) memcpy(buf, "\xEF\xBB\xBF", 3);
		bin_case("rand", buf, n);
	}
}

static void create_tests(void)
{
	char p[4200];
	git_repository *bare = NULL;
	struct stat st;
	int rc;

	pid("buf NULL id", git_blob_create_from_buffer(NULL, repo, "x", 1));
	pid("buf NULL repo", git_blob_create_from_buffer(&id, NULL, "x", 1));
	pid("buf empty", git_blob_create_from_buffer(&id, repo, NULL, 0));

	wfile("a.txt", "line1\r\nline2\n", 13);
	wfile("b.bin", "a\0b\r\n", 5);
	pid("wd a.txt", git_blob_create_from_workdir(&id, repo, "a.txt"));
	pid("wd missing", git_blob_create_from_workdir(&id, repo, "nope.txt"));
	pid("wd dir", git_blob_create_from_workdir(&id, repo, "sub"));
	pid("wd dir/", git_blob_create_from_workdir(&id, repo, "sub/"));
	pid("wd NULL", git_blob_create_from_workdir(&id, repo, NULL));
	pid("wd empty", git_blob_create_from_workdir(&id, repo, ""));
	pid("wd ../", git_blob_create_from_workdir(&id, repo, "../outside.txt"));
	pid("wd link", git_blob_create_from_workdir(&id, repo, "link"));
	pid("wd dangling", git_blob_create_from_workdir(&id, repo, "dangling"));
	pid("wd dirlink", git_blob_create_from_workdir(&id, repo, "dirlink"));
	pid("wd .git/HEAD", git_blob_create_from_workdir(&id, repo, ".git/HEAD"));

	snprintf(p, sizeof(p), "%s/w/a.txt", root);
	pid("disk abs in wd", git_blob_create_from_disk(&id, repo, p));
	snprintf(p, sizeof(p), "%s/outside.txt", root);
	pid("disk abs outside", git_blob_create_from_disk(&id, repo, p));
	snprintf(p, sizeof(p), "%s/w/sub", root);
	pid("disk dir", git_blob_create_from_disk(&id, repo, p));
	snprintf(p, sizeof(p), "%s/w/missing", root);
	pid("disk missing", git_blob_create_from_disk(&id, repo, p));
	snprintf(p, sizeof(p), "%s/w/link", root);
	pid("disk link", git_blob_create_from_disk(&id, repo, p));
	pid("disk NULL", git_blob_create_from_disk(&id, repo, NULL));
	pid("disk rel", git_blob_create_from_disk(&id, repo, "outside.txt"));
	pid("disk empty", git_blob_create_from_disk(&id, repo, ""));

	/* internal create_from_paths */
	snprintf(p, sizeof(p), "%s/w/a.txt", root);
	memset(&st, 0, sizeof(st));
	rc = git_blob__create_from_paths(&id, &st, repo, p, NULL, S_IFLNK | 0777, false);
	pid("paths hint_mode=lnk on file", rc);
	memset(&st, 0, sizeof(st));
	rc = git_blob__create_from_paths(&id, &st, repo, p, "a.txt", 0, true);
	printf("st_size=%lld\n", (long long)st.st_size);
	pid("paths filters", rc);
	rc = git_blob__create_from_paths(&id, NULL, repo, p, NULL, 0, true);
	pid("paths assert", rc);
	rc = git_blob__create_from_paths(&id, NULL, repo, p, "b.bin", 0, true);
	pid("paths filters bin", rc);
	snprintf(p, sizeof(p), "%s/w/sub", root);
	memset(&st, 0, sizeof(st));
	rc = git_blob__create_from_paths(&id, &st, repo, p, NULL, 0, false);
	printf("dir st_size=%lld\n", (long long)st.st_size);
	pid("paths dir", rc);

	snprintf(p, sizeof(p), "%s/bare.git", root);
	git_repository_init(&bare, p, 1);
	pid("bare wd", git_blob_create_from_workdir(&id, bare, "a.txt"));
	snprintf(p, sizeof(p), "%s/outside.txt", root);
	pid("bare disk", git_blob_create_from_disk(&id, bare, p));
	{
		git_writestream *s = NULL;
		rc = git_blob_create_from_stream(&s, bare, "x.txt");
		err("bare stream", rc);
		if (s) {
			s->write(s, "abc\r\n", 5);
			pid("bare stream commit", git_blob_create_from_stream_commit(&id, s));
		}
	}
	git_repository_free(bare);
}

static void stream_tests(void)
{
	git_writestream *s = NULL;
	static char big[5 * 1024 * 1024];
	int rc;

	memset(big, 'z', sizeof(big));
	rc = git_blob_create_from_stream(NULL, repo, NULL);
	err("stream NULL out", rc);
	rc = git_blob_create_from_stream(&s, NULL, NULL);
	err("stream NULL repo", rc);

	rc = git_blob_create_from_stream(&s, repo, NULL);
	err("stream open", rc);
	err("write1", s->write(s, "hello\r\n", 7));
	err("write0", s->write(s, NULL, 0));
	pid("commit plain", git_blob_create_from_stream_commit(&id, s));
	list_streamed("after commit plain");

	rc = git_blob_create_from_stream(&s, repo, "a.txt");
	err("write1", s->write(s, "hello\r\nworld\r\n", 14));
	pid("commit hint a.txt", git_blob_create_from_stream_commit(&id, s));

	rc = git_blob_create_from_stream(&s, repo, "sub");
	err("write1", s->write(s, "hello\r\n", 7));
	pid("commit hint dir", git_blob_create_from_stream_commit(&id, s));

	rc = git_blob_create_from_stream(&s, repo, "");
	err("write1", s->write(s, "hello\r\n", 7));
	pid("commit hint empty", git_blob_create_from_stream_commit(&id, s));

	rc = git_blob_create_from_stream(&s, repo, NULL);
	err("big write", s->write(s, big, sizeof(big)));
	err("big write2", s->write(s, big, 2 * 1024 * 1024));
	err("big write3", s->write(s, big, 2 * 1024 * 1024 - 1));
	pid("commit big", git_blob_create_from_stream_commit(&id, s));

	rc = git_blob_create_from_stream(&s, repo, NULL);
	err("write", s->write(s, "abc", 3));
	list_streamed("before free");
	s->free(s);
	list_streamed("after free w/o commit");

	rc = git_blob_create_from_stream(&s, repo, NULL);
	err("write", s->write(s, "abc", 3));
	err("close", s->close(s));
	list_streamed("after close");
	err("close again", s->close(s));
	s->free(s);
	list_streamed("after close+free");

	rc = git_blob_create_from_stream(&s, repo, NULL);
	err("close (empty)", s->close(s));
	if (getenv("WRITE_AFTER_CLOSE")) err("write0 after close", s->write(s, "", 0));
	s->free(s);

	rc = git_blob_create_from_stream(&s, repo, NULL);
	err("write", s->write(s, "abc", 3));
	err("close", s->close(s));
	/* commit after close: C behaviour? */
	if (getenv("COMMIT_AFTER_CLOSE"))
		pid("commit after close", git_blob_create_from_stream_commit(&id, s));
	else
		s->free(s);

	rc = git_blob_create_from_stream(&s, repo, NULL);
	if (getenv("WRITE_AFTER_CLOSE")) {
		err("close", s->close(s));
		err("write after close", s->write(s, "abc", 3));
	}
	s->free(s);
	list_streamed("end");
}

static void filter_tests(void)
{
	git_blob *b = NULL, *bb = NULL, *d = NULL;
	git_buf out = GIT_BUF_INIT;
	git_blob_filter_options o;
	git_oid head;
	int rc, f;

	git_blob_create_from_buffer(&id, repo, "x\ny\n", 4);
	git_blob_lookup(&b, repo, &id);
	git_blob_create_from_buffer(&id, repo, "x\0y\n", 4);
	git_blob_lookup(&bb, repo, &id);

	err("init v0", git_blob_filter_options_init(&o, 0));
	err("init v2", git_blob_filter_options_init(&o, 2));
	memset(&o, 0x5a, sizeof(o));
	err("init v1", git_blob_filter_options_init(&o, 1));
	printf("init: version=%d flags=%u reserved=%p\n", o.version, o.flags, (void *)o.commit_id);

	err("filter NULL blob", git_blob_filter(&out, NULL, "a.txt", NULL));
	err("filter NULL path", git_blob_filter(&out, b, NULL, NULL));
	err("filter NULL out", git_blob_filter(NULL, b, "a.txt", NULL));
	o.version = 0;
	err("filter v0", git_blob_filter(&out, b, "a.txt", &o));
	o.version = 2;
	err("filter v2", git_blob_filter(&out, b, "a.txt", &o));

	for (f = 0; f < 16; f++) {
		git_blob_filter_options_init(&o, 1);
		o.flags = f;
		if (f & GIT_BLOB_FILTER_ATTRIBUTES_FROM_COMMIT)
			git_oid_from_string(&o.attr_commit_id, "0123456789012345678901234567890123456789", GIT_OID_SHA1);
		rc = git_blob_filter(&out, b, "a.txt", &o);
		printf("filter f=%d out=[%.*s] size=%zu\n", f, (int)out.size, out.ptr ? out.ptr : "", out.size);
		err("filter text", rc);
		git_buf_dispose(&out);
		rc = git_blob_filter(&out, bb, "a.txt", &o);
		printf("filter bin f=%d size=%zu ptr=%d\n", f, out.size, out.ptr != NULL);
		err("filter bin", rc);
		git_buf_dispose(&out);
		rc = git_blob_filter(&out, b, "b.bin", &o);
		printf("filter b.bin f=%d size=%zu\n", f, out.size);
		err("filter b.bin", rc);
		git_buf_dispose(&out);
	}
	rc = git_blob_filter(&out, b, "a.txt", NULL);
	printf("filter NULL opts out=[%.*s]\n", (int)out.size, out.ptr ? out.ptr : "");
	err("filter NULL opts", rc);
	git_buf_dispose(&out);

	/* reuse a non-empty out buffer */
	git_buf_dispose(&out);
	rc = git_blob_filter(&out, b, "a.txt", NULL);
	rc = git_blob_filter(&out, b, "a.txt", NULL);
	printf("filter twice out=[%.*s]\n", (int)out.size, out.ptr ? out.ptr : "");
	err("filter twice", rc);
	git_buf_dispose(&out);

	err("dup", git_blob_dup(&d, b));
	printf("dup same=%d owner=%d rawsize=%llu\n", d == b, git_blob_owner(d) == repo,
	       (unsigned long long)git_blob_rawsize(d));
	git_blob_free(d);
	printf("rawsize NULL=%llu\n", (unsigned long long)git_blob_rawsize(NULL));
	err("rawsize NULL", 0);
	printf("rawcontent NULL=%p\n", git_blob_rawcontent(NULL));
	err("rawcontent NULL", 0);
	err("is_binary NULL", git_blob_is_binary(NULL));
	{
		git_blob *p = NULL;
		size_t l;
		for (l = 0; l <= 41; l++) {
			rc = git_blob_lookup_prefix(&p, repo, git_blob_id(b), l);
			printf("prefix %zu ok=%d\n", l, rc == 0 && p != NULL);
			err("lookup_prefix", rc);
			git_blob_free(p);
			p = NULL;
		}
		git_oid_from_string(&head, "0000000000000000000000000000000000000001", GIT_OID_SHA1);
		err("lookup missing", git_blob_lookup(&p, repo, &head));
	}
	git_blob_free(b);
	git_blob_free(bb);
}


static void extra_tests(void)
{
	git_object *o = NULL;
	git_buf out = GIT_BUF_INIT;
	git_str s = GIT_STR_INIT;
	char p[4200];
	int rc;
	rc = git_object__from_raw(&o, "a\r\nb\n", 5, GIT_OBJECT_BLOB, GIT_OID_SHA1);
	err("raw", rc);
	printf("raw owner=%p\n", (void *)git_blob_owner((git_blob *)o));
	rc = git_blob__getbuf(&s, (git_blob *)o);
	printf("getbuf size=%zu\n", s.size);
	err("getbuf", rc);
	git_str_dispose(&s);
	if (!getenv("NO_RAW_FILTER")) {
		rc = git_blob_filter(&out, (git_blob *)o, "a.txt", NULL);
		printf("raw filter size=%zu\n", out.size);
		err("raw filter", rc);
		git_buf_dispose(&out);
	}
	git_object_free(o);
	wfile("empty", "", 0);
	snprintf(p, sizeof(p), "%s/w/empty", root);
	rc = git_blob__create_from_paths(&id, NULL, repo, p, NULL, S_IFLNK, false);
	pid("paths lnk empty file", rc);
	rc = git_blob__create_from_paths(&id, NULL, repo, p, NULL, 0, false);
	pid("paths empty file", rc);
	snprintf(p, sizeof(p), "%s/w/link", root);
	rc = git_blob__create_from_paths(&id, NULL, repo, p, NULL, S_IFREG|0644, false);
	pid("paths regmode on link", rc);
	snprintf(p, sizeof(p), "%s/w/a.txt", root);
	chmod(p, 0);
	pid("wd unreadable", git_blob_create_from_workdir(&id, repo, "a.txt"));
	pid("disk unreadable", git_blob_create_from_disk(&id, repo, p));
	chmod(p, 0644);
	snprintf(p, sizeof(p), "%s/outside.txt", root);
	chmod(p, 0);
	pid("disk unreadable outside", git_blob_create_from_disk(&id, repo, p));
	chmod(p, 0644);
}

int main(int argc, char **argv)
{
	char p[4200];
	git_config *cfg;
	snprintf(root, sizeof(root), "%s", argv[1]);
	setvbuf(stdout, NULL, _IONBF, 0);
	git_libgit2_init();
	snprintf(p, sizeof(p), "%s/w", root);
	git_repository_init(&repo, p, 0);
	git_repository_config(&cfg, repo);
	git_config_set_bool(cfg, "core.autocrlf", 1);
	git_config_free(cfg);
	snprintf(p, sizeof(p), "%s/w/sub", root);
	mkdir(p, 0777);
	snprintf(p, sizeof(p), "%s/w/link", root);
	symlink("a.txt", p);
	snprintf(p, sizeof(p), "%s/w/dangling", root);
	symlink("nowhere/at/all", p);
	snprintf(p, sizeof(p), "%s/w/dirlink", root);
	symlink("sub", p);
	snprintf(p, sizeof(p), "%s/outside.txt", root);
	{ FILE *f = fopen(p, "w"); fputs("out\r\nside\n", f); fclose(f); }
	wfile(".gitattributes", "*.bin binary\n*.txt text eol=crlf\n", 33);
	if (chdir(root) != 0) return 1;

	if (!getenv("ONLY") || !strcmp(getenv("ONLY"), "bin")) binary_tests();
	if (!getenv("ONLY") || !strcmp(getenv("ONLY"), "create")) create_tests();
	if (!getenv("ONLY") || !strcmp(getenv("ONLY"), "stream")) stream_tests();
	if (!getenv("ONLY") || !strcmp(getenv("ONLY"), "filter")) filter_tests();
	if (!getenv("ONLY") || !strcmp(getenv("ONLY"), "extra")) extra_tests();
	git_repository_free(repo);
	git_libgit2_shutdown();
	return 0;
}
