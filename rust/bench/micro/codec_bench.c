/*
 * Benchmarks of hash (SHA-1 with collision detection, SHA-256), zstream
 * (inflate, deflate), date parsing and wildmatch, through libgit2's
 * internal C ABI. Linked against the C reference objects and against the
 * Rust port, as util_bench.c. Prints "bench <name> median_ms <x> ...".
 *
 * The text corpus is the C sources of /work/bench-data/libgit2-wd (or
 * $CODEC_BENCH_DIR), concatenated up to 16 MiB; synthetic text if absent.
 */
#include "git2_util.h"
#include "str.h"
#include "hash.h"
#include "zstream.h"
#include "date.h"
#include "wildmatch.h"
#include "git2/global.h"

#include <dirent.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>

static volatile size_t g_sink;
static int g_iters = 7;

#define CHECK(expr) do { if ((expr) < 0) { \
	fprintf(stderr, "FAILED: %s at %s:%d\n", #expr, __FILE__, __LINE__); \
	exit(1); } } while (0)

static double now_ms(void)
{
	struct timespec ts;
	clock_gettime(CLOCK_MONOTONIC, &ts);
	return (double)ts.tv_sec * 1e3 + (double)ts.tv_nsec / 1e6;
}

static int cmp_double(const void *a, const void *b)
{
	double x = *(const double *)a, y = *(const double *)b;
	return (x > y) - (x < y);
}

static uint64_t rng_state = 0x9E3779B97F4A7C15ull;
static uint64_t rng(void)
{
	rng_state ^= rng_state >> 12;
	rng_state ^= rng_state << 25;
	rng_state ^= rng_state >> 27;
	return rng_state * 2685821657736338717ull;
}

/* ------------------------------------------------------------------ */
/* data                                                                */

#define BIG (64u << 20)
#define TEXT_MAX (16u << 20)

static unsigned char *g_random;   /* BIG random bytes */
static git_str g_text = GIT_STR_INIT;     /* source text */
static git_str g_deflated = GIT_STR_INIT; /* g_text, deflated */
static git_str g_small_def = GIT_STR_INIT; /* 4000 small objects, deflated */
static size_t g_small_len[4000], g_small_off[4000];

static void setup_random(void)
{
	size_t i;
	if (g_random)
		return;
	g_random = malloc(BIG);
	for (i = 0; i < BIG / 8; i++) {
		uint64_t r = rng();
		memcpy(g_random + 8 * i, &r, 8);
	}
}

static void add_dir(const char *dir)
{
	DIR *d = opendir(dir);
	struct dirent *e;
	if (!d)
		return;
	while ((e = readdir(d)) != NULL && g_text.size < TEXT_MAX) {
		char path[4096];
		struct stat st;
		size_t n = strlen(e->d_name);
		if (e->d_name[0] == '.')
			continue;
		snprintf(path, sizeof(path), "%s/%s", dir, e->d_name);
		if (stat(path, &st) < 0)
			continue;
		if (S_ISDIR(st.st_mode)) {
			add_dir(path);
		} else if (n > 2 && (!strcmp(e->d_name + n - 2, ".c") || !strcmp(e->d_name + n - 2, ".h"))) {
			FILE *f = fopen(path, "rb");
			char buf[65536];
			size_t r;
			if (!f)
				continue;
			while ((r = fread(buf, 1, sizeof(buf), f)) > 0 && g_text.size < TEXT_MAX)
				CHECK(git_str_put(&g_text, buf, r));
			fclose(f);
		}
	}
	closedir(d);
}

static void setup_text(void)
{
	const char *dir = getenv("CODEC_BENCH_DIR");
	if (g_text.size)
		return;
	add_dir(dir ? dir : "/work/bench-data/libgit2-wd");
	if (g_text.size < (1u << 20)) {
		static const char *w[] = { "int ", "error", " = ", "git_", "str", "(", ")", ";\n", "\t", "if ", "return ", "0", "->", "ptr", "size" };
		while (g_text.size < TEXT_MAX)
			CHECK(git_str_puts(&g_text, w[rng() % 15]));
	}
}

static void setup_inflate(void)
{
	size_t i, off = 0;
	setup_text();
	if (g_deflated.size)
		return;
	CHECK(git_zstream_deflatebuf(&g_deflated, g_text.ptr, g_text.size));
	/* loose-object-sized pieces, 100 B to 8 KiB */
	for (i = 0; i < 4000; i++) {
		size_t len = 100 + rng() % 8000, before = g_small_def.size;
		if (off + len > g_text.size)
			off = 0;
		CHECK(git_zstream_deflatebuf(&g_small_def, g_text.ptr + off, len));
		g_small_off[i] = before;
		g_small_len[i] = g_small_def.size - before;
		off += len;
	}
}

/* ------------------------------------------------------------------ */
/* workloads                                                           */

static void hash_big(git_hash_algorithm_t a)
{
	unsigned char out[GIT_HASH_MAX_SIZE];
	CHECK(git_hash_buf(out, g_random, BIG, a));
	g_sink += out[0];
}
static void bench_sha1_64m(void) { hash_big(GIT_HASH_ALGORITHM_SHA1); }
static void bench_sha256_64m(void) { hash_big(GIT_HASH_ALGORITHM_SHA256); }

/* 200000 hashes of `len` bytes (object-sized and smaller) */
static void hash_small(git_hash_algorithm_t a, size_t len)
{
	unsigned char out[GIT_HASH_MAX_SIZE];
	size_t i;
	for (i = 0; i < 200000; i++) {
		CHECK(git_hash_buf(out, g_random + (i % 4096) * 16, len, a));
		g_sink += out[0];
	}
}
static void bench_sha1_100b(void) { hash_small(GIT_HASH_ALGORITHM_SHA1, 100); }
static void bench_sha256_100b(void) { hash_small(GIT_HASH_ALGORITHM_SHA256, 100); }
static void bench_sha1_1k(void) { hash_small(GIT_HASH_ALGORITHM_SHA1, 1000); }
static void bench_sha256_1k(void) { hash_small(GIT_HASH_ALGORITHM_SHA256, 1000); }

/* The per-object pattern of indexer.c / odb.c: one context, 200000 x
 * git_hash_init + update(len) + final. */
static void hash_ctx(git_hash_algorithm_t a, size_t len)
{
	git_hash_ctx ctx;
	unsigned char out[GIT_HASH_MAX_SIZE];
	size_t i;
	CHECK(git_hash_ctx_init(&ctx, a));
	for (i = 0; i < 200000; i++) {
		CHECK(git_hash_init(&ctx));
		CHECK(git_hash_update(&ctx, g_random + (i % 4096) * 16, len));
		CHECK(git_hash_final(out, &ctx));
		g_sink += out[0];
	}
	git_hash_ctx_cleanup(&ctx);
}
static void bench_sha1_ctx_0(void) { hash_ctx(GIT_HASH_ALGORITHM_SHA1, 0); }
static void bench_sha1_ctx_64(void) { hash_ctx(GIT_HASH_ALGORITHM_SHA1, 64); }
static void bench_sha256_ctx_0(void) { hash_ctx(GIT_HASH_ALGORITHM_SHA256, 0); }
static void bench_sha256_ctx_64(void) { hash_ctx(GIT_HASH_ALGORITHM_SHA256, 64); }

static void bench_inflate_text(void)
{
	git_str out = GIT_STR_INIT;
	CHECK(git_zstream_inflatebuf(&out, g_deflated.ptr, g_deflated.size));
	g_sink += out.size;
	git_str_dispose(&out);
}

static void bench_inflate_small(void)
{
	git_str out = GIT_STR_INIT;
	size_t i;
	for (i = 0; i < 4000; i++) {
		git_str_clear(&out);
		CHECK(git_zstream_inflatebuf(&out, g_small_def.ptr + g_small_off[i], g_small_len[i]));
		g_sink += out.size;
	}
	git_str_dispose(&out);
}

/* streaming inflate into a 64 KiB buffer, as pack and loose readers do */
static void bench_inflate_stream(void)
{
	git_zstream zs = GIT_ZSTREAM_INIT;
	char buf[65536];
	CHECK(git_zstream_init(&zs, GIT_ZSTREAM_INFLATE));
	CHECK(git_zstream_set_input(&zs, g_deflated.ptr, g_deflated.size));
	while (!git_zstream_done(&zs)) {
		size_t n = sizeof(buf);
		CHECK(git_zstream_get_output(buf, &n, &zs));
		g_sink += n;
	}
	git_zstream_free(&zs);
}

static void bench_deflate_text(void)
{
	git_str out = GIT_STR_INIT;
	CHECK(git_zstream_deflatebuf(&out, g_text.ptr, 4u << 20));
	g_sink += out.size;
	git_str_dispose(&out);
}

static const char *g_dates[] = {
	"2008-12-05 10:11:12 +0200", "Thu, 5 Dec 2008 10:11:12 -0130", "@1234567890 +0100",
	"1234567890", "July 17, 2003", "2003-7-17 08:23i+03", "Fri Jun 14 09:30:00 2024 +0000",
	"12/05/2008", "10:11:12 PST"
};
static const char *g_approx[] = { "2 days ago", "yesterday", "last friday", "3.weeks.ago", "noon" };

static void bench_date_parse(void)
{
	size_t i;
	for (i = 0; i < 100000; i++) {
		git_time_t t;
		int off;
		CHECK(git_date_offset_parse(&t, &off, g_dates[i % ARRAY_SIZE(g_dates)]));
		g_sink += (size_t)t;
	}
}

static void bench_date_approx(void)
{
	size_t i;
	for (i = 0; i < 20000; i++) {
		git_time_t t;
		CHECK(git_date_parse(&t, g_approx[i % ARRAY_SIZE(g_approx)]));
		g_sink += (size_t)t;
	}
}

static const char *g_wm[][2] = {
	{ "*.c", "src/libgit2/repository.c" },
	{ "src/**/*.h", "src/util/hash/sha1dc/ubc_check.h" },
	{ "**/tests/**", "deps/zlib/tests/resources/foo/bar" },
	{ "*[0-9][0-9][0-9]*", "objects/pack/pack-94cbf0849bcb58d0.idx" },
	{ "[!.]*.o", "build/CMakeFiles/libgit2.dir/blob.c.o" },
	{ "doc/*.txt", "doc/sub/readme.txt" },
	{ "*foo*bar*baz*", "a/very/long/path/without/the/words/we/are/looking/for" },
	{ "[[:alpha:]]*", "Makefile" },
};

static void bench_wildmatch(void)
{
	size_t i;
	for (i = 0; i < 1000000; i++) {
		const char **p = g_wm[i % ARRAY_SIZE(g_wm)];
		g_sink += (size_t)(wildmatch(p[0], p[1], (i & 1) ? WM_PATHNAME : WM_CASEFOLD) + 1);
	}
}

/* ------------------------------------------------------------------ */

static void run(const char *name, void (*setup)(void), void (*fn)(void))
{
	double *t = malloc(sizeof(double) * g_iters);
	int k;

	if (setup) setup();
	fn(); /* warm-up */
	for (k = 0; k < g_iters; k++) {
		double s = now_ms();
		fn();
		t[k] = now_ms() - s;
	}
	qsort(t, g_iters, sizeof(double), cmp_double);
	printf("bench %s median_ms %.3f min_ms %.3f max_ms %.3f\n",
		name, t[g_iters / 2], t[0], t[g_iters - 1]);
	fflush(stdout);
	free(t);
}

int main(int argc, char **argv)
{
	const char *only = NULL;

	if (argc > 1) g_iters = atoi(argv[1]);
	if (g_iters < 1) g_iters = 7;
	if (argc > 2) only = argv[2];

	if (git_libgit2_init() < 0) {
		fprintf(stderr, "git_libgit2_init failed\n");
		return 1;
	}

#define RUN(n, s, f) if (!only || strstr(n, only)) run(n, s, f)
	RUN("sha1_64m", setup_random, bench_sha1_64m);
	RUN("sha256_64m", setup_random, bench_sha256_64m);
	RUN("sha1_100b_x200k", setup_random, bench_sha1_100b);
	RUN("sha256_100b_x200k", setup_random, bench_sha256_100b);
	RUN("sha1_1k_x200k", setup_random, bench_sha1_1k);
	RUN("sha256_1k_x200k", setup_random, bench_sha256_1k);
	RUN("sha1_ctx0_x200k", setup_random, bench_sha1_ctx_0);
	RUN("sha1_ctx64_x200k", setup_random, bench_sha1_ctx_64);
	RUN("sha256_ctx0_x200k", setup_random, bench_sha256_ctx_0);
	RUN("sha256_ctx64_x200k", setup_random, bench_sha256_ctx_64);
	RUN("inflate_text_16m", setup_inflate, bench_inflate_text);
	RUN("inflate_stream_16m", setup_inflate, bench_inflate_stream);
	RUN("inflate_small_x4000", setup_inflate, bench_inflate_small);
	RUN("deflate_text_4m", setup_text, bench_deflate_text);
	RUN("date_parse_x100k", NULL, bench_date_parse);
	RUN("date_approx_x20k", NULL, bench_date_approx);
	RUN("wildmatch_x1m", NULL, bench_wildmatch);

	fprintf(stderr, "sink %zu text %zu\n", (size_t)g_sink, g_text.size);
	git_libgit2_shutdown();
	return 0;
}
