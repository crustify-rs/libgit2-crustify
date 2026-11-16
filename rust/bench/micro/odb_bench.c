/*
 * Per-call micro-benchmarks of the odb layer (WP l-odb), C vs Rust: the
 * cache hit path of git_odb_read / git_odb_exists, git_odb_read_header
 * and git_odb_write (hash + freshen) on a mempack, and the internal
 * git_odb__format_object_header.
 *
 * Build and run: rust/bench/micro/odb_run.sh (links through
 * rust/difftest/link.sh, edge libgit2_tests).
 *
 * difftest-link: libgit2_tests
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "git2.h"
#include "git2/sys/odb_backend.h"
#include "git2/sys/mempack.h"

extern int git_odb__format_object_header(size_t *out_len, char *hdr, size_t hdr_size, git_object_size_t obj_len, git_object_t obj_type);

#define N 4096
static git_odb *odb;
static git_oid commits[N], blobs[N];
static char *datas[N];
static size_t lens[N];

static double now_ms(void)
{
	struct timespec ts;
	clock_gettime(CLOCK_MONOTONIC, &ts);
	return ts.tv_sec * 1e3 + ts.tv_nsec / 1e6;
}

static uint64_t w_read_cached(void)
{
	uint64_t h = 0; int r, i;
	for (r = 0; r < 100; r++)
		for (i = 0; i < N; i++) {
			git_odb_object *o;
			git_odb_read(&o, odb, &commits[i]);
			h += git_odb_object_size(o);
			git_odb_object_free(o);
		}
	return h;
}

static uint64_t w_exists_cached(void)
{
	uint64_t h = 0; int r, i;
	for (r = 0; r < 100; r++)
		for (i = 0; i < N; i++)
			h += git_odb_exists(odb, &commits[i]);
	return h;
}

static uint64_t w_read_blob(void)
{
	uint64_t h = 0; int r, i;
	for (r = 0; r < 20; r++)
		for (i = 0; i < N; i++) {
			git_odb_object *o;
			git_odb_read(&o, odb, &blobs[i]);
			h += git_odb_object_size(o);
			git_odb_object_free(o);
		}
	return h;
}

static uint64_t w_read_header(void)
{
	uint64_t h = 0; int r, i;
	for (r = 0; r < 50; r++)
		for (i = 0; i < N; i++) {
			size_t len; git_object_t t;
			git_odb_read_header(&len, &t, odb, &blobs[i]);
			h += len + t;
		}
	return h;
}

static uint64_t w_write_existing(void)
{
	uint64_t h = 0; int r, i;
	for (r = 0; r < 10; r++)
		for (i = 0; i < N; i++) {
			git_oid id;
			git_odb_write(&id, odb, datas[i], lens[i], GIT_OBJECT_BLOB);
			h += id.id[0];
		}
	return h;
}

static uint64_t w_format_header(void)
{
	uint64_t h = 0; unsigned i;
	char hdr[64];
	for (i = 0; i < 2000000; i++) {
		size_t n;
		git_odb__format_object_header(&n, hdr, sizeof(hdr), i * 7919u, 1 + (i & 3));
		h += n + (unsigned char)hdr[n - 2];
	}
	return h;
}

static const struct { const char *name; uint64_t (*fn)(void); } workloads[] = {
	{ "odb_read_cached", w_read_cached },
	{ "odb_exists_cached", w_exists_cached },
	{ "odb_read_blob_mempack", w_read_blob },
	{ "odb_read_header_mempack", w_read_header },
	{ "odb_write_existing", w_write_existing },
	{ "format_object_header", w_format_header },
};

static int cmp(const void *a, const void *b)
{
	double x = *(const double *)a, y = *(const double *)b;
	return x < y ? -1 : x > y;
}

int main(int argc, char **argv)
{
	int iters = argc > 1 ? atoi(argv[1]) : 7;
	const char *filter = argc > 2 ? argv[2] : NULL;
	git_odb_backend *mp;
	size_t w; int i;

	git_libgit2_init();
	git_odb_new(&odb);
	git_mempack_new(&mp);
	git_odb_add_backend(odb, mp, 1);
	for (i = 0; i < N; i++) {
		char c[256];
		size_t k;
		snprintf(c, sizeof(c), "tree %040d\nparent %040d\nauthor A <a@b> %d +0000\n\ncommit %d\n", i, i + 1, i, i);
		git_odb_write(&commits[i], odb, c, strlen(c), GIT_OBJECT_COMMIT);
		lens[i] = 200 + (i * 37) % 3000;
		datas[i] = malloc(lens[i]);
		for (k = 0; k < lens[i]; k++)
			datas[i][k] = (char)('a' + (i + k) % 23);
		git_odb_write(&blobs[i], odb, datas[i], lens[i], GIT_OBJECT_BLOB);
	}
	/* Fill the cache with the commits. */
	w_read_cached();

	for (w = 0; w < sizeof(workloads) / sizeof(workloads[0]); w++) {
		double t[64], med;
		uint64_t check = 0;
		int n = iters < 64 ? iters : 64;
		if (filter && !strstr(workloads[w].name, filter))
			continue;
		workloads[w].fn(); /* warm up */
		for (i = 0; i < n; i++) {
			double s = now_ms();
			check ^= workloads[w].fn();
			t[i] = now_ms() - s;
		}
		qsort(t, n, sizeof(double), cmp);
		med = t[n / 2];
		printf("bench %s median_ms %.3f min_ms %.3f max_ms %.3f check %llx\n", workloads[w].name, med, t[0], t[n - 1],
			(unsigned long long)check);
	}
	git_odb_free(odb);
	git_libgit2_shutdown();
	return 0;
}
