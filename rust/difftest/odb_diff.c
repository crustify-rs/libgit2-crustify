/*
 * Differential test of the object database (WP l-odb): the loose
 * backend's files (bytes, modes) for every compression level, object
 * type and a range of sizes; reading them back (whole, header, stream,
 * prefix); corrupt and pack-like loose objects and their errors; the
 * object header formatter; the cache's eviction order.
 *
 * difftest-link: libgit2_tests
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
#include <zlib.h>

#include "git2.h"
#include "git2/sys/odb_backend.h"
#include "git2/sys/mempack.h"
#include "git2/sys/errors.h"

extern int git_odb__format_object_header(size_t *out_len, char *hdr, size_t hdr_size, git_object_size_t obj_len, git_object_t obj_type);

static char root[256];

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
	if (r < 0)
		printf(" [%d] %s", e ? e->klass : -1, e && e->message ? e->message : "(none)");
	printf("\n");
	git_error_clear();
}

static unsigned char *data_of(size_t n, int kind)
{
	unsigned char *d = malloc(n + 1);
	size_t i;
	unsigned int x = 12345;
	for (i = 0; i < n; i++) {
		x = x * 1103515245 + 12345;
		d[i] = kind == 0 ? (unsigned char)('a' + i % 26) :
		       kind == 1 ? (unsigned char)(x >> 16) :
		       (unsigned char)(i % 7 == 0 ? '\n' : 'x');
	}
	return d;
}

static void file_info(const char *objdir, const git_oid *id)
{
	char path[512], hex[GIT_OID_MAX_HEXSIZE + 1];
	struct stat st;
	FILE *f;
	unsigned char *buf;
	long n;

	git_oid_tostr(hex, sizeof(hex), id);
	snprintf(path, sizeof(path), "%s/%.2s/%s", objdir, hex, hex + 2);
	if (stat(path, &st) < 0) { printf("  no file\n"); return; }
	f = fopen(path, "rb");
	fseek(f, 0, SEEK_END); n = ftell(f); fseek(f, 0, SEEK_SET);
	buf = malloc(n ? n : 1);
	n = (long)fread(buf, 1, n, f);
	fclose(f);
	printf("  file %ld bytes fnv %016lx mode %o\n", n, fnv(buf, n), st.st_mode & 07777);
	free(buf);
	snprintf(path, sizeof(path), "%s/%.2s", objdir, hex);
	stat(path, &st);
	printf("  dir mode %o\n", st.st_mode & 07777);
}

static void read_all(git_odb *odb, const git_oid *id)
{
	git_odb_object *obj = NULL;
	size_t len = 0; git_object_t t = 0;
	int r = git_odb_read(&obj, odb, id);
	err("read", r);
	if (!r) {
		printf("  read %zu type %d fnv %016lx\n", git_odb_object_size(obj), git_odb_object_type(obj),
			fnv(git_odb_object_data(obj), git_odb_object_size(obj)));
		git_odb_object_free(obj);
	}
	r = git_odb_read_header(&len, &t, odb, id);
	err("read_header", r);
	if (!r) printf("  header %zu %d\n", len, t);
	{
		git_odb_stream *s = NULL;
		r = git_odb_open_rstream(&s, &len, &t, odb, id);
		err("rstream", r);
		if (!r) {
			char buf[777]; unsigned long h = 1469598103934665603UL; size_t total = 0; int n, calls = 0;
			while ((n = git_odb_stream_read(s, buf, sizeof(buf))) > 0) {
				size_t i; for (i = 0; i < (size_t)n; i++) { h ^= (unsigned char)buf[i]; h *= 1099511628211UL; }
				total += n; calls++;
			}
			printf("  stream %zu %d -> %zu bytes in %d reads, last %d, fnv %016lx\n", len, t, total, calls, n, h);
			if (n < 0) err("stream_read", n);
			git_odb_stream_free(s);
		}
	}
}

static void headers(void)
{
	static const git_object_size_t lens[] = { 0, 1, 9, 10, 12345, 4294967296ULL, 9223372036854775807ULL, 18446744073709551615ULL };
	static const int types[] = { -2, -1, 0, 1, 2, 3, 4, 5, 6, 7, 8 };
	static const size_t sizes[] = { 0, 1, 2, 5, 6, 7, 8, 12, 64 };
	size_t a, b, c;
	printf("== headers\n");
	for (a = 0; a < sizeof(lens)/sizeof(lens[0]); a++)
	for (b = 0; b < sizeof(types)/sizeof(types[0]); b++)
	for (c = 0; c < sizeof(sizes)/sizeof(sizes[0]); c++) {
		char hdr[64]; size_t n = 999; int r;
		memset(hdr, 'Z', sizeof(hdr));
		r = git_odb__format_object_header(&n, hdr, sizes[c], lens[a], types[b]);
		printf("%llu %d %zu: %d %zu '%.*s'\n", (unsigned long long)lens[a], types[b], sizes[c], r,
			r ? 0 : n, (int)(sizes[c] ? strnlen(hdr, sizes[c]) : 0), hdr);
		git_error_clear();
	}
}

static void write_loose(void)
{
	static const int levels[] = { -1, 0, 1, 6, 9 };
	static const size_t sizes[] = { 0, 1, 100, 4095, 70000 };
	static const git_object_t types[] = { GIT_OBJECT_BLOB, GIT_OBJECT_COMMIT, GIT_OBJECT_TREE, GIT_OBJECT_TAG };
	size_t l, s, t, k;
	printf("== write loose\n");
	for (l = 0; l < sizeof(levels)/sizeof(levels[0]); l++) {
		char dir[512];
		git_odb *odb; git_odb_backend *be;
		git_odb_backend_loose_options opts = GIT_ODB_BACKEND_LOOSE_OPTIONS_INIT;
		opts.compression_level = levels[l];
		if (l == 2) { opts.dir_mode = 0750; opts.file_mode = 0640; }
		if (l == 3) opts.flags = GIT_ODB_BACKEND_LOOSE_FSYNC;
		snprintf(dir, sizeof(dir), "%s/level%zu", root, l);
		mkdir(dir, 0777);
		git_odb_new(&odb);
		err("backend", git_odb_backend_loose(&be, dir, &opts));
		git_odb_add_backend(odb, be, 1);
		for (s = 0; s < sizeof(sizes)/sizeof(sizes[0]); s++)
		for (t = 0; t < 4; t++)
		for (k = 0; k < 3; k++) {
			unsigned char *d = data_of(sizes[s], k);
			git_oid id; char hex[41]; int r;
			printf("level %d size %zu type %d kind %zu\n", levels[l], sizes[s], types[t], k);
			r = git_odb_write(&id, odb, d, sizes[s], types[t]);
			err("write", r);
			git_oid_tostr(hex, sizeof(hex), &id);
			printf("  id %s\n", hex);
			file_info(dir, &id);
			read_all(odb, &id);
			free(d);
		}
		/* A streamed write of the same data gives the same file. */
		{
			unsigned char *d = data_of(70000, 1);
			git_odb_stream *ws; git_oid id; size_t off; char hex[41];
			err("wstream", git_odb_open_wstream(&ws, odb, 70000, GIT_OBJECT_BLOB));
			for (off = 0; off < 70000; off += 3333)
				err("swrite", git_odb_stream_write(ws, (char *)d + off, off + 3333 > 70000 ? 70000 - off : 3333));
			err("finalize", git_odb_stream_finalize_write(&id, ws));
			git_oid_tostr(hex, sizeof(hex), &id);
			printf("  streamed %s\n", hex);
			git_odb_stream_free(ws);
			err("wstream2", git_odb_open_wstream(&ws, odb, 10, GIT_OBJECT_BLOB));
			err("swrite-over", git_odb_stream_write(ws, (char *)d, 11));
			err("finalize-short", git_odb_stream_finalize_write(&id, ws));
			git_odb_stream_free(ws);
			free(d);
		}
		git_odb_free(odb);
	}
}

static void put_file(const char *dir, const char *hex, const unsigned char *data, size_t n)
{
	char path[512];
	FILE *f;
	snprintf(path, sizeof(path), "%s/%.2s", dir, hex);
	mkdir(path, 0777);
	snprintf(path, sizeof(path), "%s/%.2s/%s", dir, hex, hex + 2);
	unlink(path);
	f = fopen(path, "wb");
	fwrite(data, 1, n, f);
	fclose(f);
}

static size_t deflated(unsigned char *out, size_t cap, const unsigned char *in, size_t n, int level)
{
	uLongf len = cap;
	compress2(out, &len, in, n, level);
	return len;
}

static void corrupt(void)
{
	static const struct { const char *body; size_t len; } bodies[] = {
		{ "blob 5\0hello", 12 },
		{ "blob 5\0hel", 10 },
		{ "blob 2\0hello", 12 },
		{ "blob\0hello", 10 },
		{ "blob 5", 6 },
		{ "bogus 5\0hello", 13 },
		{ "commitx 3\0abc", 13 },
		{ "tree -1\0", 8 },
		{ "blob  7\0abcdefg", 15 },
		{ "blob 99999999999999999999\0", 26 },
		{ "blob 0\0", 7 },
		{ "", 0 },
		{ "ofs-delta 3\0abc", 15 },
		{ "blob 64\0" "0123456789012345678901234567890123456789012345678901234567890123", 72 },
		{ "blob 70\0" "0123456789012345678901234567890123456789012345678901234567890123456789", 78 },
	};
	char dir[512];
	git_odb *odb; git_odb_backend *be;
	size_t i, lvl;
	unsigned char buf[4096];
	char hex[41];
	unsigned int serial = 0;
	git_oid id;
#define NEXT_ID() (snprintf(hex, sizeof(hex), "%08x90123456789012345678901234567890", ++serial), \
	git_oid_from_string(&id, hex, GIT_OID_SHA1))

	printf("== corrupt\n");
	snprintf(dir, sizeof(dir), "%s/corrupt", root);
	mkdir(dir, 0777);
	git_odb_new(&odb);
	git_odb_backend_loose(&be, dir, NULL);
	git_odb_add_backend(odb, be, 1);
	git_libgit2_opts(GIT_OPT_ENABLE_STRICT_HASH_VERIFICATION, 0);
	for (lvl = 0; lvl < 2; lvl++)
	for (i = 0; i < sizeof(bodies)/sizeof(bodies[0]); i++) {
		size_t n = deflated(buf, sizeof(buf), (const unsigned char *)bodies[i].body, bodies[i].len, lvl ? 9 : 1);
		printf("body %zu level %zu (%zu bytes)\n", i, lvl, n);
		NEXT_ID();
		put_file(dir, hex, buf, n);
		read_all(odb, &id);
		/* truncated */
		NEXT_ID();
		put_file(dir, hex, buf, n - 2);
		printf(" truncated\n");
		read_all(odb, &id);
		/* trailing garbage */
		memcpy(buf + n, "junk", 4);
		NEXT_ID();
		put_file(dir, hex, buf, n + 4);
		printf(" trailing\n");
		read_all(odb, &id);
	}
	{
		/* Raw garbage, an empty file, a pack-like object. */
		static const unsigned char garbage[] = "this is not zlib at all";
		unsigned char pl[256]; size_t n;
		NEXT_ID();
		put_file(dir, hex, garbage, sizeof(garbage) - 1);
		printf("garbage\n");
		read_all(odb, &id);
		NEXT_ID();
		put_file(dir, hex, garbage, 0);
		printf("empty\n");
		read_all(odb, &id);
		pl[0] = 0x35; /* blob, size 5 */
		n = 1 + deflated(pl + 1, sizeof(pl) - 1, (const unsigned char *)"hello", 5, 1);
		NEXT_ID();
		put_file(dir, hex, pl, n);
		printf("packlike\n");
		read_all(odb, &id);
		pl[0] = 0x3a; /* blob, size 10: short body */
		NEXT_ID();
		put_file(dir, hex, pl, n);
		printf("packlike short\n");
		read_all(odb, &id);
		pl[0] = 0xb5; pl[1] = 0x01; /* blob, size 21, then garbage */
		NEXT_ID();
		put_file(dir, hex, pl, n);
		printf("packlike long header\n");
		read_all(odb, &id);
		pl[0] = 0x65; /* ofs-delta */
		NEXT_ID();
		put_file(dir, hex, pl, n);
		printf("packlike delta\n");
		read_all(odb, &id);
	}
	git_libgit2_opts(GIT_OPT_ENABLE_STRICT_HASH_VERIFICATION, 1);
	/* With verification: a mismatch. */
	{
		size_t n = deflated(buf, sizeof(buf), (const unsigned char *)"blob 5\0hello", 12, 1);
		NEXT_ID();
		put_file(dir, hex, buf, n);
		printf("mismatch\n");
		read_all(odb, &id);
	}
	git_odb_free(odb);
}

static int count_cb(const git_oid *id, void *p)
{
	(*(size_t *)p)++;
	(void)id;
	return *(size_t *)p == 50 ? 77 : 0;
}

static void prefixes(void)
{
	char dir[512];
	git_odb *odb; git_odb_backend *be;
	git_oid ids[64], out, key;
	size_t i, len;
	printf("== prefixes\n");
	snprintf(dir, sizeof(dir), "%s/prefix", root);
	mkdir(dir, 0777);
	git_odb_new(&odb);
	git_odb_backend_loose(&be, dir, NULL);
	git_odb_add_backend(odb, be, 1);
	for (i = 0; i < 64; i++) {
		char d[32]; snprintf(d, sizeof(d), "object %zu\n", i);
		git_odb_write(&ids[i], odb, d, strlen(d), GIT_OBJECT_BLOB);
	}
	for (i = 0; i < 64; i += 7)
	for (len = 3; len <= 41; len += (len < 8 ? 1 : 16)) {
		git_odb_object *obj = NULL; char hex[41]; int r;
		key = ids[i];
		r = git_odb_exists_prefix(&out, odb, &key, len);
		git_oid_tostr(hex, sizeof(hex), &out);
		printf("%zu %zu exists_prefix %d %s\n", i, len, r, r ? "" : hex);
		err("e", r);
		r = git_odb_read_prefix(&obj, odb, &key, len);
		printf("%zu %zu read_prefix %d\n", i, len, r);
		err("r", r);
		git_odb_object_free(obj);
	}
	/* Ambiguity: the first hex digit of every id shares 16 dirs. */
	{
		git_oid z; int r; git_odb_object *obj = NULL;
		memset(&z, 0, sizeof(z)); z.type = GIT_OID_SHA1;
		z.id[0] = ids[0].id[0];
		r = git_odb_exists_prefix(&out, odb, &z, 4);
		err("ambiguous exists", r);
		r = git_odb_read_prefix(&obj, odb, &z, 4);
		err("ambiguous read", r);
	}
	{
		git_odb_expand_id q[4];
		memset(q, 0, sizeof(q));
		q[0].id = ids[1]; q[0].length = 7;
		q[1].id = ids[2]; q[1].length = 40; q[1].type = GIT_OBJECT_COMMIT;
		q[2].id = ids[3]; q[2].length = 3;
		q[3].id = ids[4]; q[3].length = 40;
		err("expand", git_odb_expand_ids(odb, q, 4));
		for (i = 0; i < 4; i++) {
			char hex[41]; git_oid_tostr(hex, sizeof(hex), &q[i].id);
			printf("  q%zu %s %u %d\n", i, hex, q[i].length, q[i].type);
		}
	}
	{
		size_t n = 0;
		int r = git_odb_foreach(odb, count_cb, &n);
		printf("foreach %d %zu\n", r, n);
		err("foreach", r);
	}
	git_odb_free(odb);
}

static void cache(void)
{
	git_odb *odb; git_odb_backend *mp;
	git_oid ids[3000];
	size_t i;
	ssize_t cur, max;
	printf("== cache\n");
	git_odb_new(&odb);
	git_mempack_new(&mp);
	git_odb_add_backend(odb, mp, 1);
	for (i = 0; i < 3000; i++) {
		char d[64]; snprintf(d, sizeof(d), "tree %zu\n", i);
		git_odb_write(&ids[i], odb, d, strlen(d), i % 3 == 0 ? GIT_OBJECT_COMMIT : i % 3 == 1 ? GIT_OBJECT_TREE : GIT_OBJECT_BLOB);
	}
	git_libgit2_opts(GIT_OPT_SET_CACHE_MAX_SIZE, (ssize_t)20000);
	git_libgit2_opts(GIT_OPT_SET_CACHE_OBJECT_LIMIT, GIT_OBJECT_TREE, (size_t)10);
	for (i = 0; i < 3000; i++) {
		git_odb_object *o;
		git_odb_read(&o, odb, &ids[i]);
		git_odb_object_free(o);
	}
	git_libgit2_opts(GIT_OPT_GET_CACHED_MEMORY, &cur, &max);
	printf("cached %zd max %zd\n", cur, max);
	git_mempack_reset(mp);
	for (i = 0; i < 3000; i++)
		putchar(git_odb_exists(odb, &ids[i]) ? '1' : '0');
	putchar('\n');
	/* (git_odb_foreach on a mempack crashes in C: rust/DEFECTS.md.) */
	git_libgit2_opts(GIT_OPT_SET_CACHE_MAX_SIZE, (ssize_t)(256 * 1024 * 1024));
	git_libgit2_opts(GIT_OPT_SET_CACHE_OBJECT_LIMIT, GIT_OBJECT_TREE, (size_t)4096);
	git_odb_free(odb);
	git_libgit2_opts(GIT_OPT_GET_CACHED_MEMORY, &cur, &max);
	printf("after free %zd\n", cur);
}

int main(void)
{
	char cmd[300];
	git_libgit2_init();
	umask(022);
	snprintf(root, sizeof(root), "/tmp/odb_diff.%d", (int)getpid());
	mkdir(root, 0777);
	headers();
	write_loose();
	corrupt();
	prefixes();
	cache();
	snprintf(cmd, sizeof(cmd), "rm -rf %s", root);
	if (system(cmd)) return 1;
	git_libgit2_shutdown();
	return 0;
}
