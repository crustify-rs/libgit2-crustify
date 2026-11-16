/*
 * Per-call benchmarks of WP l-repo: the repository's internal accessors that
 * the C that stays calls on hot paths (the lazily created odb/config/refdb
 * weak pointers, item paths, workdir paths), plus open/free and the options
 * shim.  Linked against the C reference objects or against the Rust port
 * (same symbols); see build_lib.sh.  Prints "bench <name> median_ms <x> ...".
 *
 *   BENCH=repo_bench ./build_lib.sh && BENCH=repo_bench ./run.sh
 */
#include "common.h"
#include "repository.h"
#include "git2/global.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static volatile size_t g_sink;
static int g_iters = 7;
static git_repository *g_repo;

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

#define N 2000000

static void b_odb_weakptr(void)
{
	size_t i, s = 0;
	git_odb *odb;
	for (i = 0; i < N; i++) {
		CHECK(git_repository_odb__weakptr(&odb, g_repo));
		s += (size_t)odb;
	}
	g_sink += s;
}

static void b_config_weakptr(void)
{
	size_t i, s = 0;
	git_config *cfg;
	for (i = 0; i < N; i++) {
		CHECK(git_repository_config__weakptr(&cfg, g_repo));
		s += (size_t)cfg;
	}
	g_sink += s;
}

static void b_refdb_weakptr(void)
{
	size_t i, s = 0;
	git_refdb *r;
	for (i = 0; i < N; i++) {
		CHECK(git_repository_refdb__weakptr(&r, g_repo));
		s += (size_t)r;
	}
	g_sink += s;
}

static void b_path_getters(void)
{
	size_t i, s = 0;
	for (i = 0; i < N; i++) {
		s += (size_t)git_repository_path(g_repo);
		s += (size_t)git_repository_workdir(g_repo);
		s += (size_t)git_repository_is_bare(g_repo);
		s += (size_t)git_repository_oid_type(g_repo);
	}
	g_sink += s;
}

#define NP 500000

static void b_item_path(void)
{
	git_str s = GIT_STR_INIT;
	size_t i, t = 0;
	for (i = 0; i < NP; i++) {
		CHECK(git_repository__item_path(&s, g_repo, (git_repository_item_t)(i % 14)));
		t += s.size;
	}
	git_str_dispose(&s);
	g_sink += t;
}

static void b_workdir_path(void)
{
	git_str s = GIT_STR_INIT;
	size_t i, t = 0;
	for (i = 0; i < NP; i++) {
		CHECK(git_repository_workdir_path(&s, g_repo, "src/some/file.c"));
		t += s.size;
	}
	git_str_dispose(&s);
	g_sink += t;
}

static void b_opts(void)
{
	size_t i, t = 0, v;
	for (i = 0; i < N; i++) {
		CHECK(git_libgit2_opts(GIT_OPT_GET_MWINDOW_SIZE, &v));
		t += v;
	}
	g_sink += t;
}

static const char *g_path;

static void b_open_free(void)
{
	size_t i, t = 0;
	for (i = 0; i < 2000; i++) {
		git_repository *r;
		CHECK(git_repository_open(&r, g_path));
		t += strlen(git_repository_path(r));
		git_repository_free(r);
	}
	g_sink += t;
}

static const struct { const char *name; void (*fn)(void); } benches[] = {
	{ "odb_weakptr_x2M", b_odb_weakptr },
	{ "config_weakptr_x2M", b_config_weakptr },
	{ "refdb_weakptr_x2M", b_refdb_weakptr },
	{ "path_getters_x2M", b_path_getters },
	{ "item_path_x500k", b_item_path },
	{ "workdir_path_x500k", b_workdir_path },
	{ "opts_get_mwindow_x2M", b_opts },
	{ "open_free_x2000", b_open_free },
};

int main(int argc, char **argv)
{
	const char *filter = argc > 2 ? argv[2] : NULL;
	size_t b;
	git_odb *odb;
	git_refdb *refdb;
	git_config *cfg;

	if (argc > 1)
		g_iters = atoi(argv[1]);
	git_libgit2_init();
	g_path = getenv("REPO_BENCH_PATH") ? getenv("REPO_BENCH_PATH") : "/work/bench-data/git-wd";
	CHECK(git_repository_open(&g_repo, g_path));
	CHECK(git_repository_odb__weakptr(&odb, g_repo));
	CHECK(git_repository_refdb__weakptr(&refdb, g_repo));
	CHECK(git_repository_config__weakptr(&cfg, g_repo));

	for (b = 0; b < sizeof(benches) / sizeof(benches[0]); b++) {
		double *t = malloc(sizeof(double) * g_iters);
		int it;
		if (filter && strcmp(filter, benches[b].name) != 0)
			continue;
		benches[b].fn(); /* warm-up */
		for (it = 0; it < g_iters; it++) {
			double s = now_ms();
			benches[b].fn();
			t[it] = now_ms() - s;
		}
		qsort(t, g_iters, sizeof(double), cmp_double);
		printf("bench %s median_ms %.3f min_ms %.3f max_ms %.3f\n",
		       benches[b].name, t[g_iters / 2], t[0], t[g_iters - 1]);
		free(t);
	}

	git_repository_free(g_repo);
	git_libgit2_shutdown();
	return (int)(g_sink & 0);
}
