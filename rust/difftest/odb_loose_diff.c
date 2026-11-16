/*
 * Loose and mempack backends through their slots (from review l-odb-r1,
 * loose/ld.c): corrupt objects, prefixes and foreach on SHA-1 and SHA-256
 * backends (with non-hex and upper-case names), slot out-parameters,
 * compression levels outside 0..9, modes and umask, fsync options,
 * freshen/mtimes, positive foreach returns, mempack dump bytes.
 *
 * difftest-link: libgit2_tests
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <unistd.h>
#include <utime.h>
#include <zlib.h>
#include <git2.h>
#include <git2/sys/odb_backend.h>
#include <git2/sys/mempack.h>
#include <git2/sys/errors.h>

static char root[256];
static size_t hash_limit = (size_t)-1;

static unsigned long fnv(const unsigned char *p, size_t n)
{
	unsigned long h = 1469598103934665603UL;
	while (n--) { h ^= *p++; h *= 1099511628211UL; }
	return h;
}

static void err(const char *what, int r)
{
	const git_error *e = git_error_last();
	printf("  %s = %d", what, r);
	if (r < 0) {
		/* temporary file names end in a random suffix: mask it */
		char m[1024], *q;
		snprintf(m, sizeof(m), "%s", e && e->message ? e->message : "(none)");
		if ((q = strstr(m, "_git2_")) != NULL)
			for (q += 6; *q && strchr("0123456789abcdef", *q); q++)
				*q = 'X';
		printf(" [%d] %s", e ? e->klass : -1, m);
	}
	printf("\n");
	git_error_clear();
}

static void put(const char *dir, const char *rel, const void *d, size_t n)
{
	char p[1024], q[1024];
	FILE *f;
	snprintf(p, sizeof(p), "%s/%s", dir, rel);
	strcpy(q, p);
	*strrchr(q, '/') = 0;
	mkdir(q, 0777);
	unlink(p);
	f = fopen(p, "wb");
	if (n) fwrite(d, 1, n, f);
	fclose(f);
}

static size_t defl(unsigned char *out, size_t cap, const void *in, size_t n, int level)
{
	uLongf len = cap;
	compress2(out, &len, in, n, level);
	return len;
}

static git_oid mkid(const char *hex, git_oid_t t)
{
	git_oid id;
	if (git_oid_from_prefix(&id, hex, strlen(hex), t) < 0) { printf("bad hex %s\n", hex); exit(1); }
	return id;
}

static void relpath(char *out, const char *hex)
{
	sprintf(out, "%.2s/%s", hex, hex + 2);
}

static void probe(git_odb_backend *be, const git_oid *id)
{
	void *buf = NULL; size_t len = 777; git_object_t t = 99; int r;
	git_odb_stream *s = NULL;
	r = be->read(&buf, &len, &t, be, id);
	err("read", r);
	/* A pack-like body shorter than its header leaves C's tail
	 * uninitialized (rust/DEFECTS.md): hash what was inflated only. */
	if (!r) { printf("   %zu %d fnv %016lx\n", len, t, fnv(buf, len < hash_limit ? len : hash_limit)); git_odb_backend_data_free(be, buf); }
	len = 777; t = 99;
	r = be->read_header(&len, &t, be, id);
	err("read_header", r);
	if (!r) printf("   %zu %d\n", len, t);
	len = 777; t = 99;
	r = be->readstream(&s, &len, &t, be, id);
	err("readstream", r);
	if (!r) {
		char b[33]; int n, calls = 0; size_t total = 0; unsigned long h = 1469598103934665603UL;
		while ((n = s->read(s, b, sizeof(b))) > 0) {
			int i; for (i = 0; i < n; i++) { h ^= (unsigned char)b[i]; h *= 1099511628211UL; }
			total += n; if (++calls > 100000) break;
		}
		printf("   stream %zu %d -> %zu in %d last %d fnv %016lx\n", len, t, total, calls, n, h);
		if (n < 0) err("sread", n);
		git_odb_stream_free(s);
	} else printf("   out %p %zu %d\n", (void *)s, len, t);
}

static void corrupt(void)
{
	char dir[512], rel[128];
	git_odb_backend *be;
	unsigned char z[8192], m[8192];
	size_t n, i;
	int k;
	const char *hex = "0123456789012345678901234567890123456789";
	static const struct { const char *b; size_t n; } hdrs[] = {
		{ "blob +5\0hello", 13 }, { "blob 05\0hello", 13 }, { "blob 5 \0hello", 13 },
		{ "blob  5\0hello", 13 }, { "blob 5x\0hello", 13 }, { "blob \0", 6 },
		{ "OFS_DELTA 3\0abc", 15 }, { "REF_DELTA 3\0abc", 15 }, { " 3\0abc", 6 },
		{ "comm 3\0abc", 10 }, { "blob\0 5\0hello", 13 }, { "\0blob 5\0hello", 13 },
		{ "blob 9223372036854775807\0", 25 }, { "blob 9223372036854775806\0", 25 },
		{ "blob 18446744073709551615\0", 26 }, { "blob 9223372036854775808\0", 25 },
		{ "tag 0\0", 6 }, { "blob 3\0hello", 12 }, { "blob 5", 6 },
		{ "blob 1234567890123456789012345678901234567890123456789012345678901234567\0", 73 },
		{ "blob\t5\0hello", 12 }, { "blob 0x5\0hello", 14 }, { "blob -0\0", 8 },
	};
	printf("== corrupt\n");
	snprintf(dir, sizeof(dir), "%s/corrupt", root);
	mkdir(dir, 0777);
	git_odb_backend_loose(&be, dir, NULL);
	relpath(rel, hex);
	{ git_oid id = mkid(hex, GIT_OID_SHA1);
	for (i = 0; i < sizeof(hdrs)/sizeof(hdrs[0]); i++) {
		n = defl(z, sizeof(z), hdrs[i].b, hdrs[i].n, 6);
		printf("hdr %zu\n", i);
		put(dir, rel, z, n);
		probe(be, &id);
	}
	/* A valid object, every truncation and single byte corruption. */
	{
		char body[3000]; size_t bl;
		unsigned int x = 7;
		bl = sprintf(body, "blob %d", 2000) + 1;
		for (i = 0; i < 2000; i++) { x = x * 1103515245 + 12345; body[bl + i] = (i % 3) ? 'a' + (x >> 16) % 26 : (char)(x >> 8); }
		bl += 2000;
		for (k = 0; k <= 9; k += 9) {
			n = defl(z, sizeof(z), body, bl, k);
			for (i = 0; i < n; i += (n / 40 + 1)) {
				printf("trunc level %d at %zu/%zu\n", k, i, n);
				put(dir, rel, z, i);
				probe(be, &id);
			}
			for (i = 0; i < n; i += (n / 60 + 1)) {
				memcpy(m, z, n);
				m[i] ^= 0x55;
				printf("flip level %d at %zu/%zu\n", k, i, n);
				put(dir, rel, m, n);
				probe(be, &id);
			}
		}
	}
	/* Packlike. */
	{
		static const unsigned char heads[][12] = {
			{1, 0x35}, {1, 0x3f}, {2, 0xb5, 0x01}, {1, 0x95}, {1, 0x05}, {1, 0x45}, {1, 0x75},
			{10, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01},
			{11, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01},
			{9, 0xb0, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x10},
			{9, 0xbf, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x0f},
			{1, 0x30},
		};
		for (i = 0; i < sizeof(heads)/sizeof(heads[0]); i++) {
			for (k = 0; k < 4; k++) {
				size_t hl = heads[i][0];
				memcpy(m, heads[i] + 1, hl);
				n = hl;
				if (k == 1) n += defl(m + hl, sizeof(m) - hl, "hello", 5, 1);
				if (k == 2) { n += defl(m + hl, sizeof(m) - hl, "hello world, longer", 19, 1); }
				if (k == 3) { memcpy(m + hl, "garbage!", 8); n += 8; }
				printf("packlike %zu body %d\n", i, k);
				put(dir, rel, m, n);
				hash_limit = k == 1 ? 5 : k == 2 ? 19 : (size_t)-1;
				probe(be, &id);
				hash_limit = (size_t)-1;
			}
		}
	}
	/* Big header-only read: file > 1024 bytes, header at start. */
	{
		static char big[100000]; size_t bl = sprintf(big, "blob %d", 90000) + 1;
		memset(big + bl, 'q', 90000);
		n = defl(z, sizeof(z), big, bl + 90000, 0);
		printf("stored big %zu\n", n);
		{ unsigned char *zz = malloc(200000); uLongf zl = 200000; compress2(zz, &zl, (unsigned char *)big, bl + 90000, 0);
		  put(dir, rel, zz, zl); probe(be, &id);
		  /* now corrupt the tail only: header still readable */
		  zz[zl - 1] ^= 1; put(dir, rel, zz, zl); probe(be, &id); free(zz); }
	}
	/* empty file, 1-byte file, directory in place of file */
	put(dir, rel, "", 0); printf("empty\n"); probe(be, &id);
	put(dir, rel, "\x78", 1); printf("one\n"); probe(be, &id);
	{ char p[1024]; snprintf(p, sizeof(p), "%s/%s", dir, rel); unlink(p); mkdir(p, 0777); printf("dir\n"); probe(be, &id); rmdir(p); }
	printf("missing\n"); probe(be, &id);
	}
	be->free(be);
}

/* foreach's order is the file system's: collect, then print sorted */
static char fe_lines[4096][96];
static int fe_n;

static int fe_cb(const git_oid *id, void *p)
{
	char hex[GIT_OID_MAX_HEXSIZE + 1];
	git_oid_tostr(hex, sizeof(hex), id);
	if (fe_n < 4096)
		snprintf(fe_lines[fe_n++], sizeof(fe_lines[0]), "   cb %s t%d", hex, id->type);
	(*(int *)p)++;
	return *(int *)p == 1000 ? -42 : 0;
}

static int cmp_line(const void *a, const void *b) { return strcmp(a, b); }

static void fe_flush(void)
{
	int i;
	qsort(fe_lines, fe_n, sizeof(fe_lines[0]), cmp_line);
	for (i = 0; i < fe_n; i++)
		printf("%s\n", fe_lines[i]);
	fe_n = 0;
}

static void prefixes(git_oid_t t)
{
	char dir[512];
	git_odb_backend *be;
	git_odb_backend_loose_options o = GIT_ODB_BACKEND_LOOSE_OPTIONS_INIT;
	size_t hs = t == GIT_OID_SHA1 ? 40 : 64, len;
	char name[200];
	unsigned char z[256];
	size_t zn = defl(z, sizeof(z), "blob 2\0hi", 9, 1);
	int cnt, r;
	const char *names[] = {
		"aaaa", /* filled below */
	};
	(void)names;
	printf("== prefixes %d\n", t);
	snprintf(dir, sizeof(dir), "%s/prefix%d", root, t);
	mkdir(dir, 0777);
	o.oid_type = t;
	err("backend", git_odb_backend_loose(&be, dir, &o));
	/* objects */
#define PUTHEX(h) do { char rr[200]; relpath(rr, h); put(dir, rr, z, zn); } while (0)
	{
		char h[80]; size_t i;
		memset(h, 0, sizeof(h));
		for (i = 0; i < hs; i++) h[i] = "0123456789abcdef"[(i * 7) % 16];
		h[0] = 'a'; h[1] = 'b'; h[2] = 'c'; h[3] = 'd';
		PUTHEX(h);
		h[10] = 'f'; h[11] = 'f'; PUTHEX(h); /* shares abcd... up to 10 */
		/* non hex tail */
		h[0] = 'c'; h[1] = 'd'; h[hs - 1] = 'z'; PUTHEX(h);
		/* uppercase */
		h[0] = 'e'; h[1] = 'e'; h[hs - 1] = 'A'; h[5] = 'B'; PUTHEX(h);
		/* wrong length */
		snprintf(name, sizeof(name), "ab/cd12"); put(dir, name, z, zn);
		/* directory of right length */
		{ char p[600]; h[0]='a'; h[1]='b'; h[2]='e'; snprintf(p, sizeof(p), "%s/%.2s/%s", dir, h, h + 2); mkdir(p, 0777); }
		/* non-hex directory and stray files */
		snprintf(name, sizeof(name), "zz/%.*s", (int)(hs - 2), h + 2); put(dir, name, z, zn);
		put(dir, "stray", "x", 1);
		snprintf(name, sizeof(name), "info/%.*s", (int)(hs - 2), h + 2); put(dir, name, z, zn);
		snprintf(name, sizeof(name), "a/%.*s", (int)(hs - 1), h + 1); put(dir, name, z, zn);
	}
	{
		static const char *keys[] = { "abcd", "abcd0", "abcd07", "abcd07e", "abcdffffffff", "cd", "cd07", "cd07e5c3a18f", "eeee", "EEcd", "ab", "abe", "abe0", "zzzz", "0000" };
		size_t i;
		for (i = 0; i < sizeof(keys)/sizeof(keys[0]); i++) {
			for (len = 3; len <= hs + 2; len++) {
				git_oid key, out; char hx[100]; void *buf = NULL; size_t l; git_object_t ty;
				char kk[100];
				if (len != 3 && len != 4 && len != 5 && len != 7 && len != 12 && len != hs - 1 && len != hs && len != hs + 1) continue;
				memset(kk, '0', sizeof(kk));
				memcpy(kk, keys[i], strlen(keys[i]));
				kk[hs] = 0;
				memset(&key, 0, sizeof(key));
				if (git_oid_from_prefix(&key, kk, hs, t) < 0) { err("key", -1); continue; }
				memset(&out, 0, sizeof(out));
				if (len <= hs || 1) {
					r = be->exists_prefix(&out, be, &key, len);
					printf("%s %zu exists_prefix %d\n", keys[i], len, r);
					err("e", r);
					if (!r) { git_oid_tostr(hx, sizeof(hx), &out); printf("   %s\n", hx); }
					printf("   out type %d\n", out.type);
				}
				if (len >= 4 && len <= hs) {
					memset(&out, 0, sizeof(out));
					r = be->read_prefix(&out, &buf, &l, &ty, be, &key, len);
					printf("%s %zu read_prefix %d\n", keys[i], len, r);
					err("r", r);
					printf("   out type %d\n", out.type);
					if (!r) git_odb_backend_data_free(be, buf);
				}
			}
		}
	}
	cnt = 0;
	r = be->foreach(be, fe_cb, &cnt);
	fe_flush();
	printf("foreach %d %d\n", r, cnt); err("fe", r);
	be->free(be);
	/* foreach on a missing dir */
	snprintf(dir, sizeof(dir), "%s/nonexistent%d", root, t);
	git_odb_backend_loose(&be, dir, &o);
	cnt = 0; r = be->foreach(be, fe_cb, &cnt); fe_flush(); err("fe-missing", r);
	{ git_oid k = mkid("abcdef", t), out; r = be->exists_prefix(&out, be, &k, 6); err("ep-missing", r); }
	be->free(be);
}

static int stop_cb(const git_oid *id, void *p) { (void)id; return ++*(int *)p == 2 ? 5 : 0; }
static int stopneg_cb(const git_oid *id, void *p) { (void)id; if (++*(int *)p == 2) { git_error_set_str(GIT_ERROR_INVALID, "mine"); return -7; } return 0; }

static void writes(void)
{
	char dir[512];
	git_odb_backend *be;
	git_odb_backend_loose_options o = GIT_ODB_BACKEND_LOOSE_OPTIONS_INIT;
	git_odb *odb;
	git_oid id;
	int r, cnt;
	struct stat st;
	char p[1024], hex[41];
	static const int lvls[] = { -5, -1, 0, 1, 5, 9, 10, 100 };
	size_t i;
	printf("== writes\n");
	for (i = 0; i < sizeof(lvls)/sizeof(lvls[0]); i++) {
		mode_t um;
		for (um = 0; um <= 077; um += 077) {
			umask(um);
			snprintf(dir, sizeof(dir), "%s/w%zu_%o", root, i, um);
			mkdir(dir, 0777);
			o.compression_level = lvls[i];
			o.file_mode = i == 3 ? 0666 : 0;
			o.dir_mode = i == 3 ? 0775 : 0;
			r = git_odb_backend_loose(&be, dir, &o);
			err("be", r);
			git_odb_new(&odb);
			git_odb_add_backend(odb, be, 1);
			r = git_odb_write(&id, odb, "hello world\n", 12, GIT_OBJECT_BLOB);
			err("write", r);
			git_oid_tostr(hex, sizeof(hex), &id);
			snprintf(p, sizeof(p), "%s/%.2s/%s", dir, hex, hex + 2);
			if (stat(p, &st) == 0) {
				FILE *f = fopen(p, "rb"); unsigned char b[4096]; size_t n = fread(b, 1, sizeof(b), f); fclose(f);
				printf(" lvl %d um %o: mode %o size %zu fnv %016lx\n", lvls[i], um, st.st_mode & 07777, n, fnv(b, n));
				snprintf(p, sizeof(p), "%s/%.2s", dir, hex); stat(p, &st);
				printf("   dir %o\n", st.st_mode & 07777);
			} else printf(" lvl %d no file\n", lvls[i]);
			snprintf(p, sizeof(p), "%s/tmp_object", dir);
			printf("   tmp %d\n", access(p, F_OK));
			/* freshen: make it old, rewrite */
			snprintf(p, sizeof(p), "%s/%.2s/%s", dir, hex, hex + 2);
			{ struct utimbuf ut = { 1000, 1000 }; utime(p, &ut); }
			r = git_odb_write(&id, odb, "hello world\n", 12, GIT_OBJECT_BLOB);
			stat(p, &st);
			printf("   rewrite %d fresh %d\n", r, st.st_mtime > 100000);
			/* write streams */
			{
				git_odb_stream *ws;
				r = git_odb_open_wstream(&ws, odb, 5, GIT_OBJECT_BLOB); err("ws", r); if (r) goto skipws;
				err("w", git_odb_stream_write(ws, "abc", 3));
				err("fin-short", git_odb_stream_finalize_write(&id, ws));
				git_odb_stream_free(ws);
				r = git_odb_open_wstream(&ws, odb, 5, GIT_OBJECT_BLOB); err("ws", r); if (r) goto skipws;
				err("w", git_odb_stream_write(ws, "abcdef", 6));
				err("fin-long", git_odb_stream_finalize_write(&id, ws));
				git_odb_stream_free(ws);
				r = git_odb_open_wstream(&ws, odb, 5, GIT_OBJECT_BLOB); err("ws", r); if (r) goto skipws;
				err("w", git_odb_stream_write(ws, "abcde", 5));
				err("w-over", git_odb_stream_write(ws, "f", 1));
				err("fin", git_odb_stream_finalize_write(&id, ws));
				git_oid_tostr(hex, sizeof(hex), &id);
				printf("   %s\n", hex);
				git_odb_stream_free(ws);
				skipws:
				err("ws-bad-type", git_odb_open_wstream(&ws, odb, 5, (git_object_t)6));
				err("ws-any", git_odb_open_wstream(&ws, odb, 5, GIT_OBJECT_ANY));
				/* direct backend slot write stream without odb */
				err("bws", be->writestream(&ws, be, 3, GIT_OBJECT_BLOB));
				if (ws) {
					git_oid fake = mkid("1111111111111111111111111111111111111111", GIT_OID_SHA1);
					err("bw", ws->write(ws, "xyzxyz", 6));
					err("bfin", ws->finalize_write(ws, &fake));
					ws->free(ws);
					snprintf(p, sizeof(p), "%s/11/11111111111111111111111111111111111111", dir);
					if (stat(p, &st) == 0) { FILE *f = fopen(p, "rb"); unsigned char b[4096]; size_t n = fread(b, 1, sizeof(b), f); fclose(f); printf("   bws file %zu %016lx mode %o\n", n, fnv(b, n), st.st_mode & 07777); }
				}
			}
			/* write with a bad type */
			r = be->write(be, &id, "abc", 3, (git_object_t)6); err("bad-type-write", r);
			r = be->write(be, &id, "abc", 3, (git_object_t)42); err("bad-type-write42", r);
			r = be->freshen(be, &id); err("freshen", r);
			{ git_oid nf = mkid("2222222222222222222222222222222222222222", GIT_OID_SHA1); r = be->freshen(be, &nf); err("freshen-missing", r); }
			cnt = 0; r = git_odb_foreach(odb, stop_cb, &cnt); err("fe-stop", r);
			cnt = 0; r = git_odb_foreach(odb, stopneg_cb, &cnt); err("fe-stopneg", r);
			git_odb_free(odb);
		}
	}
	umask(022);
	/* objects dir that does not exist / is a file */
	snprintf(dir, sizeof(dir), "%s/w_file", root);
	put(root, "w_file", "x", 1);
	git_odb_backend_loose(&be, dir, NULL);
	r = be->write(be, &id, "abc", 3, GIT_OBJECT_BLOB); err("write-into-file", r);
	{ git_odb_stream *ws = (void *)1; r = be->writestream(&ws, be, 3, GIT_OBJECT_BLOB); err("ws-into-file", r); printf("   ws %p\n", (void *)ws); }
	be->free(be);
	/* fsync */
	snprintf(dir, sizeof(dir), "%s/fsync", root); mkdir(dir, 0777);
	o.compression_level = -1; o.file_mode = 0; o.dir_mode = 0;
	o.flags = GIT_ODB_BACKEND_LOOSE_FSYNC;
	git_odb_backend_loose(&be, dir, &o);
	printf("fsync-flag\n");
	r = be->write(be, &id, "abc", 3, GIT_OBJECT_BLOB); err("w", r);
	be->free(be);
	o.flags = 0;
	git_libgit2_opts(GIT_OPT_ENABLE_FSYNC_GITDIR, 1);
	git_odb_backend_loose(&be, dir, &o);
	printf("fsync-opt\n");
	id = mkid("3333333333333333333333333333333333333333", GIT_OID_SHA1);
	r = be->write(be, &id, "abcd", 4, GIT_OBJECT_BLOB); err("w", r);
	git_libgit2_opts(GIT_OPT_ENABLE_FSYNC_GITDIR, 0);
	printf("fsync-none\n");
	id = mkid("4444444444444444444444444444444444444444", GIT_OID_SHA1);
	r = be->write(be, &id, "abcde", 5, GIT_OBJECT_BLOB); err("w", r);
	be->free(be);
}

static void mempack(void)
{
	git_odb_backend *mp;
	git_repository *repo;
	git_odb *odb;
	git_oid ids[20], tree, commit, parent;
	git_signature *sig;
	git_buf pack = GIT_BUF_INIT;
	size_t n, i;
	int r;
	char dir[512];
	printf("== mempack\n");
	snprintf(dir, sizeof(dir), "%s/repo", root);
	err("init", git_repository_init(&repo, dir, 0));
	err("mp", git_mempack_new(&mp));
	git_repository_odb(&odb, repo);
	err("add", git_odb_add_backend(odb, mp, 999));
	for (i = 0; i < 20; i++) {
		char d[32]; snprintf(d, sizeof(d), "blob %zu", i);
		git_odb_write(&ids[i], odb, d, strlen(d), GIT_OBJECT_BLOB);
	}
	git_mempack_object_count(&n, mp); printf(" count %zu\n", n);
	{
		git_treebuilder *tb;
		git_treebuilder_new(&tb, repo, NULL);
		for (i = 0; i < 5; i++) { char nm[8]; sprintf(nm, "f%zu", i); git_treebuilder_insert(NULL, tb, nm, &ids[i], GIT_FILEMODE_BLOB); }
		err("tree", git_treebuilder_write(&tree, tb));
		git_treebuilder_free(tb);
		git_signature_new(&sig, "a", "a@b", 1000, 0);
		{
			git_tree *t; git_tree_lookup(&t, repo, &tree);
			err("c1", git_commit_create(&parent, repo, NULL, sig, sig, NULL, "one", t, 0, NULL));
			{ git_commit *pc; git_commit_lookup(&pc, repo, &parent);
			  err("c2", git_commit_create(&commit, repo, NULL, sig, sig, NULL, "two", t, 1, (const git_commit **)&pc));
			  git_commit_free(pc); }
			git_tree_free(t);
		}
		git_signature_free(sig);
	}
	git_mempack_object_count(&n, mp); printf(" count %zu\n", n);
	{
		void *b; size_t l; git_object_t t;
		r = mp->read(&b, &l, &t, mp, &ids[3]); err("read", r); if (!r) { printf("  %zu %d %.*s\n", l, t, (int)l, (char *)b); git_odb_backend_data_free(mp, b); }
		{ git_oid none = mkid("5555555555555555555555555555555555555555", GIT_OID_SHA1);
		  r = mp->read(&b, &l, &t, mp, &none); err("read-missing", r);
		  r = mp->read_header(&l, &t, mp, &none); err("rh-missing", r);
		  printf("  exists %d %d\n", mp->exists(mp, &none), mp->exists(mp, &ids[0])); }
		/* empty object */
		{ git_oid e; err("w0", git_odb_write(&e, odb, "", 0, GIT_OBJECT_BLOB));
		  r = mp->read(&b, &l, &t, mp, &e); err("read0", r); if (!r) { printf("  %zu %d %d\n", l, t, b != NULL); git_odb_backend_data_free(mp, b); } }
		if (getenv("MPFE")) { r = git_odb_foreach(odb, fe_cb, &r); fe_flush(); err("odb-foreach", r); }
	}
	r = git_mempack_dump(&pack, repo, mp); err("dump", r);
	printf("  pack %zu fnv %016lx\n", pack.size, fnv((unsigned char *)pack.ptr, pack.size));
	git_buf_dispose(&pack);
	{
		git_packbuilder *pb; git_buf b2 = GIT_BUF_INIT;
		git_packbuilder_new(&pb, repo);
		r = git_mempack_write_thin_pack(mp, pb); err("thin", r);
		printf("  thin objects %zu\n", git_packbuilder_object_count(pb));
		r = git_packbuilder_write_buf(&b2, pb); err("thin-buf", r);
		printf("  thin pack %zu fnv %016lx\n", b2.size, fnv((unsigned char *)b2.ptr, b2.size));
		git_buf_dispose(&b2);
		git_packbuilder_free(pb);
	}
	r = git_mempack_reset(mp); err("reset", r);
	git_mempack_object_count(&n, mp); printf(" count %zu\n", n);
	r = git_mempack_dump(&pack, repo, mp); err("dump-empty", r);
	printf("  pack %zu fnv %016lx\n", pack.size, fnv((unsigned char *)pack.ptr, pack.size));
	git_buf_dispose(&pack);
	r = git_mempack_object_count(&n, NULL); err("count-null", r);
	r = git_mempack_new(NULL); err("new-null", r);
	git_odb_free(odb);
	git_repository_free(repo);
	/* A mempack not added to anything */
	{
		git_odb_backend *m2; git_mempack_new(&m2);
		git_oid x = mkid("6666666666666666666666666666666666666666", GIT_OID_SHA1);
		r = m2->write(m2, &x, "zz", 2, GIT_OBJECT_COMMIT); err("w", r);
		r = m2->write(m2, &x, "yy", 2, GIT_OBJECT_COMMIT); err("w-dup", r);
		git_mempack_object_count(&n, m2); printf(" count %zu\n", n);
		m2->free(m2);
	}
}

int main(int argc, char **argv)
{
	char cmd[300];
	git_libgit2_init();
	setvbuf(stdout, NULL, _IOLBF, 0);
	umask(022);
	snprintf(root, sizeof(root), "/tmp/odb_loose_diff.%d", (int)getuid()); { char c[300]; snprintf(c, sizeof(c), "rm -rf %s", root); if (system(c)) return 1; }
	mkdir(root, 0777);
	if (argc < 2 || strchr(argv[1], 'c')) corrupt();
	if (argc < 2 || strchr(argv[1], 'p')) { prefixes(GIT_OID_SHA1); prefixes(GIT_OID_SHA256); }
	if (argc < 2 || strchr(argv[1], 'w')) writes();
	if (argc < 2 || strchr(argv[1], 'm')) mempack();
	snprintf(cmd, sizeof(cmd), "rm -rf %s", root);
	if (system(cmd)) return 1;
	git_libgit2_shutdown();
	return 0;
}
