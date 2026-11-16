/* oid: parse/format hex, compare, shorten.  Input: the 424k object ids of
 * git.git's pack (idx order, i.e. sorted, then shuffled deterministically). */
#include "bench.h"

static git_oid *ids;      /* shuffled */
static char (*hex)[41];   /* hex of ids */
static git_oid *work;     /* scratch for sorting */
static size_t n;

static void prep(void *ctx)
{
	static int done;
	size_t i;
	pack_entry *e;
	(void)ctx;
	if (!done) {
		e = pack_entries(bench_data("git.git/objects"), &n);
		ids = xmalloc(n * sizeof(*ids));
		hex = xmalloc(n * sizeof(*hex));
		work = xmalloc(n * sizeof(*work));
		for (i = 0; i < n; i++)
			ids[i] = e[i].id;
		free(e);
		rng_seed(1);
		shuffle(ids, n, sizeof(*ids));
		for (i = 0; i < n; i++)
			git_oid_tostr(hex[i], 41, &ids[i]);
		done = 1;
	}
	bench_ops = n;
}

static uint64_t run_from_string(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i;
	git_oid id;
	(void)ctx;
	for (i = 0; i < n; i++) {
		CHECK(git_oid_from_string(&id, hex[i], GIT_OID_SHA1));
		h ^= id.id[0] | ((uint64_t)id.id[19] << 8);
		h = h * 31 + id.id[7];
	}
	return h;
}

static uint64_t run_from_prefix(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i;
	git_oid id;
	(void)ctx;
	for (i = 0; i < n; i++) {
		CHECK(git_oid_from_prefix(&id, hex[i], 7 + (i % 34), GIT_OID_SHA1));
		h = h * 31 + id.id[3] + id.id[19];
	}
	return h;
}

static uint64_t run_fmt(void *ctx)
{
	uint64_t h = FNV_INIT;
	char buf[41];
	size_t i;
	(void)ctx;
	buf[40] = 0;
	for (i = 0; i < n; i++) {
		CHECK(git_oid_fmt(buf, &ids[i]));
		h = h * 31 + (unsigned char)buf[0] + (unsigned char)buf[39];
	}
	return h;
}

static uint64_t run_tostr(void *ctx)
{
	uint64_t h = FNV_INIT;
	char buf[41];
	size_t i;
	(void)ctx;
	for (i = 0; i < n; i++) {
		git_oid_tostr(buf, 1 + (i % 41), &ids[i]);
		h = h * 31 + strlen(buf);
		h = h * 31 + (unsigned char)git_oid_tostr_s(&ids[i])[5];
	}
	return h;
}

static uint64_t run_pathfmt(void *ctx)
{
	uint64_t h = FNV_INIT;
	char buf[42];
	size_t i;
	(void)ctx;
	for (i = 0; i < n; i++) {
		CHECK(git_oid_pathfmt(buf, &ids[i]));
		h = h * 31 + (unsigned char)buf[2] + (unsigned char)buf[40];
	}
	return h;
}

static int cmp_oid(const void *a, const void *b)
{
	return git_oid_cmp(a, b);
}

#define SORT_N 100000

static void setup_sort(void *ctx)
{
	(void)ctx;
	memcpy(work, ids, SORT_N * sizeof(*ids));
}

static uint64_t run_sort_cmp(void *ctx)
{
	(void)ctx;
	qsort(work, SORT_N, sizeof(*work), cmp_oid);
	return fnv_oid(fnv_oid(FNV_INIT, &work[0]), &work[SORT_N / 2]);
}

static uint64_t run_equal_ncmp(void *ctx)
{
	uint64_t c = 0;
	size_t i;
	(void)ctx;
	for (i = 1; i < n; i++) {
		c += git_oid_equal(&ids[i], &ids[i - 1]);
		c += git_oid_equal(&ids[i], &ids[i]);
		c += git_oid_ncmp(&ids[i], &ids[i - 1], 1 + i % 40) < 0;
		c += git_oid_streq(&ids[i], hex[i]) == 0;
		c += git_oid_strcmp(&ids[i], hex[i - 1]) < 0;
		c += git_oid_is_zero(&ids[i]);
	}
	return c;
}

/* The shortener's trie is limited to 32767 nodes: 20 sets of 12,000 ids. */
#define SHORTEN_SETS 20
#define SHORTEN_N 12000

static uint64_t run_shorten(void *ctx)
{
	uint64_t h = 0;
	size_t s, i;
	(void)ctx;
	for (s = 0; s < SHORTEN_SETS; s++) {
		git_oid_shorten *os = git_oid_shorten_new(4);
		int len = 0;
		if (!os) bench_fail("git_oid_shorten_new", -1, __FILE__, __LINE__);
		for (i = 0; i < SHORTEN_N; i++) {
			len = git_oid_shorten_add(os, hex[s * SHORTEN_N + i]);
			CHECK(len);
			h += (uint64_t)len;
		}
		git_oid_shorten_free(os);
	}
	return h;
}

static void prep_sort(void *ctx) { prep(ctx); bench_ops = SORT_N; }
static void prep_shorten(void *ctx) { prep(ctx); bench_ops = SHORTEN_SETS * SHORTEN_N; }

static bench_t benches[] = {
	{ "oid", "from_string", prep, NULL, run_from_string, NULL, NULL, 0 },
	{ "oid", "from_prefix", prep, NULL, run_from_prefix, NULL, NULL, 0 },
	{ "oid", "fmt", prep, NULL, run_fmt, NULL, NULL, 0 },
	{ "oid", "tostr", prep, NULL, run_tostr, NULL, NULL, 0 },
	{ "oid", "pathfmt", prep, NULL, run_pathfmt, NULL, NULL, 0 },
	{ "oid", "sort_cmp", prep_sort, setup_sort, run_sort_cmp, NULL, NULL, 0 },
	{ "oid", "equal_ncmp_streq", prep, NULL, run_equal_ncmp, NULL, NULL, 0 },
	{ "oid", "shorten", prep_shorten, NULL, run_shorten, NULL, NULL, 0 },
};

BENCH_MAIN(benches)
