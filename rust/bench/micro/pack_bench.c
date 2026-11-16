/*
 * Per-call benchmarks of the pack reader's internal API (WP l-pack):
 * pack.c and mwindow.c through their C ABI, on git.git's pack
 * ($BENCH_DATA/git.git, 424k objects, deep delta chains).
 *
 *   find            git_pack_entry_find of full ids (index binary search)
 *   find_prefix     ... of 10-digit prefixes (with the ambiguity check)
 *   resolve_header  git_packfile_resolve_header (follows delta chains)
 *   unpack          git_packfile_unpack of random objects (delta chains,
 *                   inflate, the delta base cache)
 *   unpack_seq      ... in pack order (the cache hits)
 *   foreach_offset  git_pack_foreach_entry_offset over every entry
 *   window          git_mwindow_open + git_mwindow_close on an open window
 *
 * Prints `bench <name> median_ns_per_op <x> min_ns_per_op <x>` per
 * workload. Built and run by pack_bench.sh (C reference and Rust port).
 *
 * difftest-link: libgit2_tests
 */

#include "common.h"
#include "git2.h"
#include "pack.h"
#include "mwindow.h"
#include "odb.h"

#include <time.h>

static unsigned long long seed = 42;
static unsigned rnd(void)
{
	seed = seed * 6364136223846793005ULL + 1442695040888963407ULL;
	return (unsigned)(seed >> 33);
}

static double now_ns(void)
{
	struct timespec ts;
	clock_gettime(CLOCK_MONOTONIC, &ts);
	return ts.tv_sec * 1e9 + ts.tv_nsec;
}

typedef struct { git_oid id; off64_t off; } entry;
static entry *entries;
static size_t n, cap;

static int collect(const git_oid *id, off64_t off, void *p)
{
	(void)p;
	if (n == cap) {
		cap = cap ? cap * 2 : 1 << 16;
		entries = realloc(entries, cap * sizeof(*entries));
	}
	git_oid_cpy(&entries[n].id, id);
	entries[n++].off = off;
	return 0;
}

static int by_offset(const void *a, const void *b)
{
	off64_t x = ((const entry *)a)->off, y = ((const entry *)b)->off;
	return x < y ? -1 : x > y;
}

static struct git_pack_file *open_pack(const char *idx)
{
	struct git_pack_file *p;
	if (git_packfile_alloc(&p, idx, GIT_OID_SHA1) < 0)
		abort();
	return p;
}

static unsigned long long sink;
static int count_cb(const git_oid *id, off64_t off, void *p) { (void)p; sink += id->id[0] + (unsigned long long)off; return 0; }

/* One timed run of `ops` operations; returns ns per op. */
static double run(const char *name, struct git_pack_file *p, size_t ops)
{
	size_t i;
	double t0 = now_ns();

	if (!strcmp(name, "find") || !strcmp(name, "find_prefix")) {
		size_t len = name[4] ? 10 : 40;
		for (i = 0; i < ops; i++) {
			struct git_pack_entry e;
			if (git_pack_entry_find(&e, p, &entries[rnd() % n].id, len) == 0)
				sink += (unsigned long long)e.offset;
		}
	} else if (!strcmp(name, "resolve_header")) {
		for (i = 0; i < ops; i++) {
			size_t sz; git_object_t t;
			if (git_packfile_resolve_header(&sz, &t, p, entries[rnd() % n].off) == 0)
				sink += sz;
		}
	} else if (!strcmp(name, "unpack") || !strcmp(name, "unpack_seq")) {
		size_t start = rnd() % n;
		for (i = 0; i < ops; i++) {
			git_rawobj raw;
			off64_t off = name[6] ? entries[(start + i) % n].off : entries[rnd() % n].off;
			if (git_packfile_unpack(&raw, p, &off) == 0) {
				sink += raw.len;
				git__free(raw.data);
			}
		}
	} else if (!strcmp(name, "foreach_offset")) {
		for (i = 0; i < ops; i++)
			git_pack_foreach_entry_offset(p, count_cb, NULL);
	} else if (!strcmp(name, "window")) {
		git_mwindow *w = NULL;
		unsigned int left;
		for (i = 0; i < ops; i++) {
			unsigned char *d = git_mwindow_open(&p->mwf, &w, (off64_t)(rnd() % 1000000), 20, &left);
			sink += d ? d[0] : 0;
			git_mwindow_close(&w);
		}
	}
	return (now_ns() - t0) / (double)ops;
}

static int cmpd(const void *a, const void *b)
{
	double x = *(const double *)a, y = *(const double *)b;
	return x < y ? -1 : x > y;
}

int main(int argc, char **argv)
{
	const char *data = getenv("BENCH_DATA") ? getenv("BENCH_DATA") : "/work/bench-data";
	char idx[4096];
	int iters = argc > 1 ? atoi(argv[1]) : 7;
	const char *filter = argc > 2 ? argv[2] : "";
	static const struct { const char *name; size_t ops; } w[] = {
		{ "find", 200000 }, { "find_prefix", 200000 }, { "resolve_header", 20000 },
		{ "unpack", 5000 }, { "unpack_seq", 20000 }, { "foreach_offset", 3 }, { "window", 1000000 },
	};
	FILE *f;
	size_t k;

	git_libgit2_init();
	snprintf(idx, sizeof(idx), "%s/git.git/objects/pack", data);
	{
		char cmd[4200];
		snprintf(cmd, sizeof(cmd), "ls %s/*.idx", idx);
		f = popen(cmd, "r");
		if (!f || !fgets(idx, sizeof(idx), f))
			return 1;
		idx[strcspn(idx, "\n")] = 0;
		pclose(f);
	}
	{
		struct git_pack_file *p = open_pack(idx);
		git_pack_foreach_entry_offset(p, collect, NULL);
		qsort(entries, n, sizeof(*entries), by_offset);
		git_packfile_free(p, 0);
	}

	for (k = 0; k < sizeof(w) / sizeof(w[0]); k++) {
		double t[64];
		int i;
		if (*filter && !strstr(w[k].name, filter))
			continue;
		for (i = -1; i < iters; i++) {
			/* A fresh pack per run: cold delta base cache, index opened. */
			struct git_pack_file *p = open_pack(idx);
			struct git_pack_entry e;
			git_pack_entry_find(&e, p, &entries[0].id, 40);
			seed = 42 + (unsigned)k;
			if (i >= 0)
				t[i] = run(w[k].name, p, w[k].ops);
			else
				run(w[k].name, p, w[k].ops / 10 + 1);
			git_packfile_free(p, 0);
		}
		qsort(t, (size_t)iters, sizeof(double), cmpd);
		printf("bench %s median_ns_per_op %.1f min_ns_per_op %.1f\n", w[k].name, t[iters / 2], t[0]);
		fflush(stdout);
	}
	git_libgit2_shutdown();
	return sink == 1 ? 1 : 0;
}
