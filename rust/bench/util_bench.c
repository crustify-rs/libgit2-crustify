/*
 * Micro-benchmarks for libgit2's internal util layer (git_str, git_vector).
 * Linked either against the C reference objects or against the Rust port
 * (same C ABI / symbols). Prints "bench <name> median_ms <x>" per workload.
 */
#include "git2_util.h"
#include "str.h"
#include "vector.h"
#include "git2/global.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define ITERS_DEFAULT 11

static volatile size_t g_sink; /* defeat dead-code elimination */
static int g_iters = ITERS_DEFAULT;

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

/* deterministic xorshift64* PRNG */
static uint64_t rng_state = 0x9E3779B97F4A7C15ull;
static uint64_t rng(void)
{
	rng_state ^= rng_state >> 12;
	rng_state ^= rng_state << 25;
	rng_state ^= rng_state >> 27;
	return rng_state * 2685821657736338717ull;
}

/* ------------------------------------------------------------------ */
/* shared string data                                                  */

#define NSTRINGS 1000000   /* string pool; also vector_insert/bsearch N */
#define NSORT    250000    /* vector_sort N */
static char **g_strings;        /* random, unique-ish path-like names */
static char **g_sorted;         /* sorted copy of the pointers */

static const char *g_words[] = {
	"a", "src", "include", "libgit2", "refs/heads/main", "objects",
	"x", "README.md", "0123456789abcdef", "tests/resources/testrepo",
	".git", "HEAD", "config", "packed-refs", "z/y", "worktree-name"
};
#define NWORDS (sizeof(g_words) / sizeof(g_words[0]))

static void setup_strings(void)
{
	size_t i;
	g_strings = malloc(NSTRINGS * sizeof(char *));
	g_sorted = malloc(NSTRINGS * sizeof(char *));
	for (i = 0; i < NSTRINGS; i++) {
		char tmp[64];
		uint64_t r = rng();
		snprintf(tmp, sizeof(tmp), "%s/%016llx/%zu",
			g_words[r % NWORDS], (unsigned long long)(r >> 8), i);
		g_strings[i] = strdup(tmp);
	}
	memcpy(g_sorted, g_strings, NSTRINGS * sizeof(char *));
	/* g_sorted is sorted in main() */
}

/* qsort comparator over char** */
static int qsort_strptr(const void *a, const void *b)
{
	return strcmp(*(char *const *)a, *(char *const *)b);
}

/* ------------------------------------------------------------------ */
/* workloads                                                           */

static void bench_str_append(void)
{
	git_str big = GIT_STR_INIT;
	size_t i, j, total = 0;

	for (i = 0; i < 2000000; i++) {
		const char *w = g_words[i % NWORDS];
		CHECK(git_str_puts(&big, w));
		CHECK(git_str_putc(&big, '/'));
		CHECK(git_str_put(&big, "0123456789", (i % 10) + 1));
	}
	total += big.size;
	git_str_dispose(&big);

	/* fresh buffers: growth from empty */
	for (i = 0; i < 200000; i++) {
		git_str s = GIT_STR_INIT;
		for (j = 0; j < 8; j++) {
			CHECK(git_str_puts(&s, g_words[(i + j) % NWORDS]));
			CHECK(git_str_putc(&s, ':'));
		}
		total += s.size;
		git_str_dispose(&s);
	}
	g_sink += total;
}

static void bench_str_printf(void)
{
	git_str buf = GIT_STR_INIT;
	size_t i, total = 0;

	for (i = 0; i < 600000; i++) {
		if ((i & 1023) == 0) {
			total += buf.size;
			git_str_clear(&buf);
		}
		CHECK(git_str_printf(&buf, "%s/%d/%08x",
			g_words[i % NWORDS], (int)i, (unsigned)(i * 2654435761u)));
	}
	total += buf.size;
	git_str_dispose(&buf);
	g_sink += total;
}

static void bench_str_join(void)
{
	git_str buf = GIT_STR_INIT, out = GIT_STR_INIT;
	size_t i, total = 0;

	for (i = 0; i < 700000; i++) {
		const char *a = g_words[i % NWORDS];
		const char *b = g_words[(i * 7 + 3) % NWORDS];
		const char *c = g_words[(i * 13 + 5) % NWORDS];

		CHECK(git_str_joinpath(&out, a, b));
		total += out.size;
		CHECK(git_str_join(&out, '/', a, "trailing/"));
		total += out.size;
		CHECK(git_str_join_n(&out, '/', 3, a, b, c));
		total += out.size;

		/* self-referential idiom: first arg is the buffer's own content */
		if ((i & 15) == 0)
			CHECK(git_str_sets(&buf, "root"));
		CHECK(git_str_joinpath(&buf, buf.ptr, b));
		total += buf.size;
	}
	git_str_dispose(&buf);
	git_str_dispose(&out);
	g_sink += total;
}

static void bench_vector_insert(void)
{
	size_t rep, i, total = 0;

	for (rep = 0; rep < 25; rep++) {
		git_vector v = GIT_VECTOR_INIT;
		CHECK(git_vector_init(&v, 0, NULL));
		for (i = 0; i < NSTRINGS; i++)
			CHECK(git_vector_insert(&v, g_strings[i]));
		total += v.length + (size_t)(uintptr_t)git_vector_get(&v, v.length / 2);
		git_vector_dispose(&v);
	}
	g_sink += total;
}

/* sort: setup is untimed, so it's done by the driver via these globals */
static git_vector g_sortvec;
static int g_nearly_sorted;

static void setup_vector_sort(void)
{
	size_t i;
	git_vector_dispose(&g_sortvec);
	CHECK(git_vector_init(&g_sortvec, NSORT, git__strcmp_cb));
	if (!g_nearly_sorted) {
		for (i = 0; i < NSORT; i++)
			CHECK(git_vector_insert(&g_sortvec, g_strings[i]));
	} else {
		/* sorted with ~1% random swaps */
		/* every 4th element of the global sorted order: still sorted */
		for (i = 0; i < NSORT; i++)
			CHECK(git_vector_insert(&g_sortvec, g_sorted[i * (NSTRINGS / NSORT)]));
		for (i = 0; i < NSORT / 100; i++) {
			size_t x = rng() % NSORT, y = rng() % NSORT;
			void *t = g_sortvec.contents[x];
			g_sortvec.contents[x] = g_sortvec.contents[y];
			g_sortvec.contents[y] = t;
		}
	}
	git_vector_set_sorted(&g_sortvec, 0);
}

static void bench_vector_sort(void)
{
	git_vector_sort(&g_sortvec);
	g_sink += (size_t)(uintptr_t)git_vector_get(&g_sortvec, 0) +
		(size_t)(uintptr_t)git_vector_get(&g_sortvec, NSORT - 1);
}

static git_vector g_searchvec;

static void bench_vector_bsearch(void)
{
	size_t i, pos, found = 0;
	for (i = 0; i < 50000; i++) {
		const char *key = g_strings[(i * 7919) % NSTRINGS];
		if (git_vector_bsearch(&pos, &g_searchvec, key) == 0)
			found += pos;
	}
	/* some misses too */
	for (i = 0; i < 5000; i++) {
		if (git_vector_bsearch(&pos, &g_searchvec, "zzz-missing") == 0)
			found++;
		found += pos;
	}
	g_sink += found;
}

static void bench_vector_insert_sorted(void)
{
	git_vector v = GIT_VECTOR_INIT;
	size_t i;
	/* insert_sorted is O(n) memmove per insert; keep N moderate */
	CHECK(git_vector_init(&v, 0, git__strcmp_cb));
	for (i = 0; i < 40000; i++)
		CHECK(git_vector_insert_sorted(&v, g_strings[i], NULL));
	g_sink += v.length + (size_t)(uintptr_t)git_vector_get(&v, 0);
	git_vector_dispose(&v);
}

/* ------------------------------------------------------------------ */

static void run(const char *name, void (*setup)(void), void (*fn)(void))
{
	double *t = malloc(sizeof(double) * g_iters);
	int k;

	/* warm-up */
	if (setup) setup();
	fn();

	for (k = 0; k < g_iters; k++) {
		double s;
		if (setup) setup();
		s = now_ms();
		fn();
		t[k] = now_ms() - s;
	}
	qsort(t, g_iters, sizeof(double), cmp_double);
	printf("bench %s median_ms %.3f min_ms %.3f max_ms %.3f\n",
		name, t[g_iters / 2], t[0], t[g_iters - 1]);
	fflush(stdout);
	free(t);
}

static void setup_sort_random(void) { g_nearly_sorted = 0; setup_vector_sort(); }
static void setup_sort_nearly(void) { g_nearly_sorted = 1; setup_vector_sort(); }

int main(int argc, char **argv)
{
	size_t i;
	const char *only = NULL;

	if (argc > 1) g_iters = atoi(argv[1]);
	if (g_iters < 1) g_iters = ITERS_DEFAULT;
	if (argc > 2) only = argv[2];

	if (git_libgit2_init() < 0) {
		fprintf(stderr, "git_libgit2_init failed\n");
		return 1;
	}

	setup_strings();
	qsort(g_sorted, NSTRINGS, sizeof(char *), qsort_strptr);

	CHECK(git_vector_init(&g_searchvec, NSTRINGS, git__strcmp_cb));
	for (i = 0; i < NSTRINGS; i++)
		CHECK(git_vector_insert(&g_searchvec, g_sorted[i]));
	git_vector_set_sorted(&g_searchvec, 1);

#define RUN(n, s, f) if (!only || strstr(n, only)) run(n, s, f)
	RUN("str_append", NULL, bench_str_append);
	RUN("str_printf", NULL, bench_str_printf);
	RUN("str_join", NULL, bench_str_join);
	RUN("vector_insert", NULL, bench_vector_insert);
	RUN("vector_sort_random", setup_sort_random, bench_vector_sort);
	RUN("vector_sort_nearly", setup_sort_nearly, bench_vector_sort);
	RUN("vector_bsearch", NULL, bench_vector_bsearch);
	RUN("vector_insert_sorted", NULL, bench_vector_insert_sorted);

	fprintf(stderr, "sink %zu\n", (size_t)g_sink);

	git_vector_dispose(&g_sortvec);
	git_vector_dispose(&g_searchvec);
	for (i = 0; i < NSTRINGS; i++)
		free(g_strings[i]);
	free(g_strings);
	free(g_sorted);
	git_libgit2_shutdown();
	return 0;
}
