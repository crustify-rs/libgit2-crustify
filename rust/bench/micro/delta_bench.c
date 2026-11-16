/*
 * Benchmarks of delta.c through libgit2's internal C ABI (WP l-odb):
 * git_delta_apply on realistic deltas, git_delta_index_init and
 * git_delta_create_from_index. Linked against the C reference objects and
 * against the Rust port with rust/difftest/link.sh. Prints
 * "bench <name> median_ms <x> min_ms <x> max_ms <x>".
 *
 * Corpus: the files of /work/bench-data/git-wd/ *.c (or $DELTA_BENCH_DIR),
 * each paired with an edited version of itself (insertions, deletions,
 * moved blocks), as successive revisions of a source file are.
 *
 * difftest-link: libgit2_tests
 */
#include "common.h"
#include "git2/global.h"
#include "delta.h"

#include <dirent.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static volatile size_t g_sink;

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
static uint64_t rs = 0x9E3779B97F4A7C15ull;
static uint64_t rnd(void) { rs ^= rs >> 12; rs ^= rs << 25; rs ^= rs >> 27; return rs * 2685821657736338717ull; }

typedef struct { unsigned char *src, *trg, *delta; size_t slen, tlen, dlen; } pair;
static pair pairs[4096];
static size_t npairs, total_src;

static unsigned char *edit(const unsigned char *s, size_t n, size_t *out)
{
	unsigned char *v = malloc(n * 2 + 4096); size_t o = 0, i = 0;
	while (i < n) {
		size_t run = 200 + rnd() % 3000;
		if (run > n - i) run = n - i;
		memcpy(v + o, s + i, run); o += run; i += run;
		switch (rnd() % 3) {
		case 0: { size_t k = rnd() % 80, j; for (j = 0; j < k; j++) v[o++] = "abc def\n"[rnd() % 8]; break; }
		case 1: i += rnd() % 60; break;
		default: if (i > 500) { size_t from = rnd() % (i - 100), l = rnd() % 100; memcpy(v + o, s + from, l); o += l; }
		}
	}
	*out = o;
	return v;
}

static void load(const char *dir)
{
	DIR *d = opendir(dir); struct dirent *de; char path[4096];
	if (!d) { fprintf(stderr, "no %s\n", dir); exit(1); }
	while ((de = readdir(d)) && npairs < 4096) {
		size_t l = strlen(de->d_name); FILE *f; long sz;
		if (l < 3 || strcmp(de->d_name + l - 2, ".c")) continue;
		snprintf(path, sizeof(path), "%s/%s", dir, de->d_name);
		if (!(f = fopen(path, "rb"))) continue;
		fseek(f, 0, SEEK_END); sz = ftell(f); fseek(f, 0, SEEK_SET);
		if (sz < 64) { fclose(f); continue; }
		pairs[npairs].src = malloc(sz);
		pairs[npairs].slen = fread(pairs[npairs].src, 1, sz, f);
		fclose(f);
		pairs[npairs].trg = edit(pairs[npairs].src, pairs[npairs].slen, &pairs[npairs].tlen);
		if (git_delta((void **)&pairs[npairs].delta, &pairs[npairs].dlen, pairs[npairs].src, pairs[npairs].slen,
			pairs[npairs].trg, pairs[npairs].tlen, 0) < 0 || !pairs[npairs].delta) { fprintf(stderr, "delta failed\n"); exit(1); }
		total_src += pairs[npairs].slen;
		npairs++;
	}
	closedir(d);
}

static void bench_apply(void)
{
	size_t i; void *out; size_t len;
	for (i = 0; i < npairs; i++) {
		if (git_delta_apply(&out, &len, pairs[i].src, pairs[i].slen, pairs[i].delta, pairs[i].dlen) < 0) exit(2);
		g_sink += len; git__free(out);
	}
}
static void bench_index(void)
{
	size_t i; git_delta_index *idx;
	for (i = 0; i < npairs; i++) {
		if (git_delta_index_init(&idx, pairs[i].src, pairs[i].slen) < 0) exit(2);
		g_sink += git_delta_index_size(idx); git_delta_index_free(idx);
	}
}
static void bench_create(void)
{
	size_t i; void *out; size_t len;
	for (i = 0; i < npairs; i++) {
		if (git_delta(&out, &len, pairs[i].src, pairs[i].slen, pairs[i].trg, pairs[i].tlen, 0) < 0) exit(2);
		g_sink += len; free(out);
	}
}
static unsigned char *small_base, small_delta[64]; static size_t small_dlen;
static void bench_apply_small(void)
{
	int i; void *out; size_t len;
	for (i = 0; i < 200000; i++) {
		if (git_delta_apply(&out, &len, small_base, 512, small_delta, small_dlen) < 0) exit(2);
		g_sink += len; git__free(out);
	}
}

static void run(const char *name, void (*fn)(void), int reps, int iters, const char *filter)
{
	double t[64]; int k, r;
	if (filter && !strstr(name, filter)) return;
	fn();
	for (k = 0; k < iters; k++) {
		double s = now_ms();
		for (r = 0; r < reps; r++) fn();
		t[k] = now_ms() - s;
	}
	qsort(t, iters, sizeof(double), cmp_double);
	printf("bench %s median_ms %.3f min_ms %.3f max_ms %.3f\n", name, t[iters / 2], t[0], t[iters - 1]);
}

int main(int argc, char **argv)
{
	int iters = argc > 1 ? atoi(argv[1]) : 7;
	const char *filter = argc > 2 ? argv[2] : NULL;
	const char *dir = getenv("DELTA_BENCH_DIR");
	size_t j;
	git_libgit2_init();
	load(dir ? dir : "/work/bench-data/git-wd");
	fprintf(stderr, "%zu pairs, %zu bytes of sources\n", npairs, total_src);
	small_base = malloc(512);
	for (j = 0; j < 512; j++) small_base[j] = (unsigned char)j;
	/* 512 -> 432: copy 0..200, insert 20, copy 300..512 */
	{ unsigned char d[] = { 0x80, 0x04, 0xb0, 0x03, 0x90, 200, 20,
		'0','1','2','3','4','5','6','7','8','9','0','1','2','3','4','5','6','7','8','9',
		0x93, 0x2c, 0x01, 212 };
	  memcpy(small_delta, d, sizeof(d)); small_dlen = sizeof(d); }
	{ size_t b, r; git_delta_read_header(&b, &r, small_delta, small_dlen);
	  void *o; size_t l; if (git_delta_apply(&o, &l, small_base, 512, small_delta, small_dlen) < 0) { fprintf(stderr, "small delta bad: %s\n", git_error_last()->message); return 1; } git__free(o); }
	run("delta_apply", bench_apply, 20, iters, filter);
	run("delta_apply_small_x200k", bench_apply_small, 1, iters, filter);
	run("delta_index_init", bench_index, 5, iters, filter);
	run("delta_create", bench_create, 2, iters, filter);
	git_libgit2_shutdown();
	return 0;
}
