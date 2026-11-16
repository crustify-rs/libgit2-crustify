/*
 * Minimal benchmark harness for the libgit2 C/Rust benchmark suite.
 *
 * Each program defines an array of bench_t and calls bench_main().  For each
 * selected workload: prep() once (untimed), then warm-up runs and N timed
 * repetitions; setup()/teardown() run untimed around every run().  run()
 * returns a 64-bit check value that depends on the work done (counts,
 * checksums); it must be identical across repetitions and across builds.
 *
 * Output: one TSV line per workload on stdout:
 *   component  workload  reps  median_ns  min_ns  ops  check  samples_ns(,-sep)
 * Lines starting with '#' are comments.
 *
 * Environment:
 *   BENCH_DATA    data directory (default /work/bench-data)
 *   BENCH_TMP     scratch directory parent (default $TMPDIR or /tmp)
 *   BENCH_REPS    timed repetitions (default 10)
 *   BENCH_WARMUP  warm-up runs (default 2)
 *   BENCH_FILTER  fnmatch(3) pattern on "component/workload"; several
 *                 patterns may be separated by ','. Default: all.
 *   BENCH_LIST    if set, list the selected workloads and exit.
 */
#ifndef BENCH_H
#define BENCH_H

#define _GNU_SOURCE
#include <git2.h>
#include <git2/sys/odb_backend.h>
#include <git2/sys/mempack.h>
#include <git2/sys/hashsig.h>

#include <errno.h>
#include <fcntl.h>
#include <fnmatch.h>
#include <ftw.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

typedef struct {
	const char *component;
	const char *name;
	void (*prep)(void *ctx);      /* once, before the first run (untimed) */
	void (*setup)(void *ctx);     /* before every run (untimed) */
	uint64_t (*run)(void *ctx);   /* timed; returns the check value */
	void (*teardown)(void *ctx);  /* after every run (untimed) */
	void *ctx;
	uint64_t ops;                 /* operations per run (informational);
	                                 0: the value prep() put in bench_ops */
} bench_t;

/* prep() may set this to the number of operations per run (reported). */
static uint64_t bench_ops;

/* ------------------------------------------------------------------ */
/* errors */

#define CHECK(expr) do { \
	int _e = (expr); \
	if (_e < 0) bench_fail(#expr, _e, __FILE__, __LINE__); \
} while (0)

static void bench_fail(const char *what, int code, const char *file, int line)
{
	const git_error *e = git_error_last();
	fprintf(stderr, "%s:%d: %s failed: %d (%s)\n", file, line, what, code,
		e && e->message ? e->message : "no message");
	exit(2);
}

static __attribute__((unused)) void *xmalloc(size_t n)
{
	void *p = malloc(n ? n : 1);
	if (!p) { perror("malloc"); exit(2); }
	return p;
}

static __attribute__((unused)) void *xrealloc(void *p, size_t n)
{
	p = realloc(p, n ? n : 1);
	if (!p) { perror("realloc"); exit(2); }
	return p;
}

static __attribute__((unused)) char *xstrdup(const char *s)
{
	char *p = strdup(s);
	if (!p) { perror("strdup"); exit(2); }
	return p;
}

/* ------------------------------------------------------------------ */
/* checksums and deterministic randomness */

#define FNV_INIT 0xcbf29ce484222325ULL

static inline uint64_t fnv(uint64_t h, const void *data, size_t len)
{
	const unsigned char *p = data;
	while (len--) { h ^= *p++; h *= 0x100000001b3ULL; }
	return h;
}

static inline uint64_t fnv_u64(uint64_t h, uint64_t v)
{
	return fnv(h, &v, sizeof(v));
}

static inline uint64_t fnv_str(uint64_t h, const char *s)
{
	return s ? fnv(h, s, strlen(s) + 1) : fnv_u64(h, 0);
}

static inline uint64_t fnv_oid(uint64_t h, const git_oid *id)
{
	return fnv(h, id->id, 20);
}

/* checksum of a large buffer: length, head, tail and a sparse sample */
static inline uint64_t fnv_sample(uint64_t h, const void *data, size_t len)
{
	const unsigned char *p = data;
	size_t i;
	h = fnv_u64(h, len);
	h = fnv(h, p, len < 32 ? len : 32);
	if (len > 32)
		h = fnv(h, p + len - 32, 32);
	for (i = 4096; i < len; i += 4096)
		h = fnv(h, p + i, 1);
	return h;
}

static uint64_t rng_state = 0x9e3779b97f4a7c15ULL;
static inline uint64_t rng_next(void)
{
	/* splitmix64 */
	uint64_t z = (rng_state += 0x9e3779b97f4a7c15ULL);
	z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ULL;
	z = (z ^ (z >> 27)) * 0x94d049bb133111ebULL;
	return z ^ (z >> 31);
}
static inline void rng_seed(uint64_t s) { rng_state = s; }

static __attribute__((unused)) void shuffle(void *base, size_t n, size_t size)
{
	unsigned char *b = base, tmp[256];
	size_t i;
	if (size > sizeof(tmp)) abort();
	for (i = n; i > 1; i--) {
		size_t j = rng_next() % i;
		memcpy(tmp, b + (i - 1) * size, size);
		memcpy(b + (i - 1) * size, b + j * size, size);
		memcpy(b + j * size, tmp, size);
	}
}

/* ------------------------------------------------------------------ */
/* paths and scratch space */

static char bench_tmp_root[4096];
static unsigned bench_tmp_seq;

static __attribute__((unused)) const char *bench_data(const char *rel)
{
	static char buf[8][4096];
	static int slot;
	const char *d = getenv("BENCH_DATA");
	char *out = buf[slot++ & 7];
	snprintf(out, 4096, "%s/%s", d && *d ? d : "/work/bench-data", rel);
	return out;
}

static int rm_cb(const char *path, const struct stat *sb, int flag, struct FTW *ftw)
{
	(void)sb; (void)flag; (void)ftw;
	return remove(path);
}

static __attribute__((unused)) void rm_rf(const char *path)
{
	nftw(path, rm_cb, 64, FTW_DEPTH | FTW_PHYS);
}

static void bench_cleanup(void)
{
	if (bench_tmp_root[0])
		rm_rf(bench_tmp_root);
}

/* A fresh, empty, unique directory under the scratch root. */
static __attribute__((unused)) const char *bench_tmpdir(const char *tag)
{
	static char buf[8][4096];
	static int slot;
	char *out = buf[slot++ & 7];
	snprintf(out, 4096, "%s/%s-%u", bench_tmp_root, tag, bench_tmp_seq++);
	rm_rf(out);
	if (mkdir(out, 0777) < 0) { perror(out); exit(2); }
	return out;
}

static __attribute__((unused)) void write_file(const char *path, const void *data, size_t len)
{
	FILE *f = fopen(path, "wb");
	if (!f || fwrite(data, 1, len, f) != len || fclose(f)) { perror(path); exit(2); }
}

static __attribute__((unused)) char *read_file(const char *path, size_t *len)
{
	FILE *f = fopen(path, "rb");
	char *buf;
	long n;
	if (!f) { perror(path); exit(2); }
	fseek(f, 0, SEEK_END);
	n = ftell(f);
	fseek(f, 0, SEEK_SET);
	buf = xmalloc((size_t)n + 1);
	if (fread(buf, 1, (size_t)n, f) != (size_t)n) { perror(path); exit(2); }
	buf[n] = 0;
	fclose(f);
	if (len) *len = (size_t)n;
	return buf;
}

static __attribute__((unused)) void copy_file(const char *from, const char *to)
{
	size_t len;
	char *data = read_file(from, &len);
	write_file(to, data, len);
	free(data);
}

/* A bare repository in a fresh scratch dir whose objects come from `alt`
 * (an objects directory) through objects/info/alternates. */
static __attribute__((unused)) const char *bench_alt_repo(const char *tag, const char *alt)
{
	const char *dir = bench_tmpdir(tag);
	git_repository *repo;
	char path[4096];
	CHECK(git_repository_init(&repo, dir, 1));
	git_repository_free(repo);
	snprintf(path, sizeof(path), "%s/objects/info/alternates", dir);
	{
		char line[4096];
		int n = snprintf(line, sizeof(line), "%s\n", alt);
		write_file(path, line, (size_t)n);
	}
	return dir;
}

/* ------------------------------------------------------------------ */
/* git.git pack index (idx v2) parsing, to get objects in pack order */

typedef struct {
	git_oid id;
	uint64_t offset;
	int type; /* type bits from the pack entry header (1-4, 6 ofs, 7 ref) */
	uint64_t base_offset; /* OFS_DELTA: offset of the base, else 0 */
	uint64_t size;        /* size from the entry header (delta size for deltas) */
} pack_entry;

static int pack_entry_cmp_offset(const void *a, const void *b)
{
	const pack_entry *x = a, *y = b;
	return x->offset < y->offset ? -1 : x->offset > y->offset;
}

static inline uint32_t be32(const unsigned char *p)
{
	return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) | ((uint32_t)p[2] << 8) | p[3];
}

static __attribute__((unused)) void *map_file(const char *path, size_t *len)
{
	int fd = open(path, O_RDONLY);
	struct stat st;
	void *m;
	if (fd < 0 || fstat(fd, &st) < 0) { perror(path); exit(2); }
	m = mmap(NULL, (size_t)st.st_size, PROT_READ, MAP_PRIVATE, fd, 0);
	if (m == MAP_FAILED) { perror(path); exit(2); }
	close(fd);
	*len = (size_t)st.st_size;
	return m;
}

/* Entries of the largest pack in `objects_dir`/pack, sorted by offset. */
static __attribute__((unused)) pack_entry *pack_entries(const char *objects_dir, size_t *count)
{
	char cmd[4096], idxpath[4096], packpath[4096];
	FILE *p;
	size_t idxlen, packlen, n, i;
	const unsigned char *idx, *pack, *names, *offs, *big;
	pack_entry *e;

	snprintf(cmd, sizeof(cmd), "ls -S %s/pack/*.idx | head -1", objects_dir);
	p = popen(cmd, "r");
	if (!p || !fgets(idxpath, sizeof(idxpath), p)) { fprintf(stderr, "no pack in %s\n", objects_dir); exit(2); }
	pclose(p);
	idxpath[strcspn(idxpath, "\n")] = 0;
	snprintf(packpath, sizeof(packpath), "%.*s.pack", (int)(strlen(idxpath) - 4), idxpath);

	idx = map_file(idxpath, &idxlen);
	pack = map_file(packpath, &packlen);
	if (be32(idx) != 0xff744f63 || be32(idx + 4) != 2) { fprintf(stderr, "%s: not idx v2\n", idxpath); exit(2); }
	n = be32(idx + 8 + 255 * 4);
	names = idx + 8 + 256 * 4;
	offs = names + n * 20 + n * 4;
	big = offs + n * 4;
	e = xmalloc(n * sizeof(*e));
	for (i = 0; i < n; i++) {
		uint32_t o = be32(offs + i * 4);
		uint64_t off = o;
		if (o & 0x80000000u) {
			const unsigned char *b = big + (o & 0x7fffffffu) * 8;
			off = ((uint64_t)be32(b) << 32) | be32(b + 4);
		}
		CHECK(git_oid_from_raw(&e[i].id, names + i * 20, GIT_OID_SHA1));
		e[i].offset = off;
		e[i].type = (pack[off] >> 4) & 7;
		e[i].base_offset = 0;
		{
			const unsigned char *q = pack + off;
			uint64_t sz = *q & 15;
			int shift = 4;
			while (*q++ & 0x80) { sz |= (uint64_t)(*q & 0x7f) << shift; shift += 7; }
			e[i].size = sz;
			if (e[i].type == 6) {
				uint64_t rel = *q & 0x7f;
				while (*q++ & 0x80) rel = ((rel + 1) << 7) | (*q & 0x7f);
				e[i].base_offset = off - rel;
			}
		}
	}
	munmap((void *)idx, idxlen);
	munmap((void *)pack, packlen);
	qsort(e, n, sizeof(*e), pack_entry_cmp_offset);
	*count = n;
	return e;
}

/* Commit ids reachable from all refs of `repo` (rev-list --all order). */
static __attribute__((unused)) git_oid *all_commits(git_repository *repo, git_sort_t sort, size_t *count)
{
	git_revwalk *w;
	git_oid id, *out = NULL;
	size_t n = 0, cap = 0;
	CHECK(git_revwalk_new(&w, repo));
	git_revwalk_sorting(w, sort);
	CHECK(git_revwalk_push_glob(w, "refs/*"));
	while (git_revwalk_next(&id, w) == 0) {
		if (n == cap) out = xrealloc(out, (cap = cap ? cap * 2 : 4096) * sizeof(*out));
		out[n++] = id;
	}
	git_revwalk_free(w);
	*count = n;
	return out;
}

/* ------------------------------------------------------------------ */
/* timing and the driver */

static inline uint64_t now_ns(void)
{
	struct timespec ts;
	clock_gettime(CLOCK_MONOTONIC, &ts);
	return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

static int u64cmp(const void *a, const void *b)
{
	uint64_t x = *(const uint64_t *)a, y = *(const uint64_t *)b;
	return x < y ? -1 : x > y;
}

static __attribute__((unused)) int env_int(const char *name, int def)
{
	const char *v = getenv(name);
	return v && *v ? atoi(v) : def;
}

static int bench_selected(const bench_t *b)
{
	const char *f = getenv("BENCH_FILTER");
	char full[256], *pats, *tok, *save;
	int hit = 0;
	if (!f || !*f)
		return 1;
	snprintf(full, sizeof(full), "%s/%s", b->component, b->name);
	pats = xstrdup(f);
	for (tok = strtok_r(pats, ",", &save); tok && !hit; tok = strtok_r(NULL, ",", &save))
		hit = fnmatch(tok, full, 0) == 0 || fnmatch(tok, b->component, 0) == 0;
	free(pats);
	return hit;
}

static void bench_global_init(void)
{
	const char *t = getenv("BENCH_TMP");
	const char *empty;
	if (!t || !*t) t = getenv("TMPDIR");
	if (!t || !*t) t = "/tmp";
	snprintf(bench_tmp_root, sizeof(bench_tmp_root), "%s/libgit2-bench-%d", t, (int)getpid());
	rm_rf(bench_tmp_root);
	if (mkdir(bench_tmp_root, 0777) < 0) { perror(bench_tmp_root); exit(2); }
	atexit(bench_cleanup);

	CHECK(git_libgit2_init());
	/* Isolate from the user's/system configuration. */
	empty = bench_tmpdir("empty-config");
	CHECK(git_libgit2_opts(GIT_OPT_SET_SEARCH_PATH, GIT_CONFIG_LEVEL_SYSTEM, empty));
	CHECK(git_libgit2_opts(GIT_OPT_SET_SEARCH_PATH, GIT_CONFIG_LEVEL_GLOBAL, empty));
	CHECK(git_libgit2_opts(GIT_OPT_SET_SEARCH_PATH, GIT_CONFIG_LEVEL_XDG, empty));
}

static int bench_main(bench_t *benches, size_t n)
{
	int reps = env_int("BENCH_REPS", 10);
	int warmup = env_int("BENCH_WARMUP", 2);
	int failed = 0;
	size_t i;
	uint64_t *samples;

	if (getenv("BENCH_LIST")) {
		for (i = 0; i < n; i++)
			if (bench_selected(&benches[i]))
				printf("%s/%s\n", benches[i].component, benches[i].name);
		return 0;
	}
	if (reps < 1) reps = 1;
	bench_global_init();
	samples = xmalloc(sizeof(*samples) * (size_t)reps);
	{
		int major, minor, rev;
		git_libgit2_version(&major, &minor, &rev);
		printf("# libgit2 %d.%d.%d reps=%d warmup=%d\n", major, minor, rev, reps, warmup);
	}

	for (i = 0; i < n; i++) {
		bench_t *b = &benches[i];
		uint64_t check = 0, sorted_median;
		int r, consistent = 1;

		if (!bench_selected(b))
			continue;
		bench_ops = b->ops;
		if (b->prep) b->prep(b->ctx);
		for (r = -warmup; r < reps; r++) {
			uint64_t t0, t1, c;
			if (b->setup) b->setup(b->ctx);
			t0 = now_ns();
			c = b->run(b->ctx);
			t1 = now_ns();
			if (b->teardown) b->teardown(b->ctx);
			if (r == -warmup)
				check = c;
			else if (c != check)
				consistent = 0;
			if (r >= 0)
				samples[r] = t1 - t0;
		}
		printf("%s\t%s\t%d\t", b->component, b->name, reps);
		{
			uint64_t *s = xmalloc(sizeof(*s) * (size_t)reps);
			memcpy(s, samples, sizeof(*s) * (size_t)reps);
			qsort(s, (size_t)reps, sizeof(*s), u64cmp);
			sorted_median = reps % 2 ? s[reps / 2] : (s[reps / 2 - 1] + s[reps / 2]) / 2;
			printf("%" PRIu64 "\t%" PRIu64 "\t", sorted_median, s[0]);
			free(s);
		}
		printf("%" PRIu64 "\t", b->ops ? b->ops : bench_ops);
		if (consistent)
			printf("%016" PRIx64 "\t", check);
		else {
			printf("INCONSISTENT\t");
			failed = 1;
		}
		for (r = 0; r < reps; r++)
			printf("%s%" PRIu64, r ? "," : "", samples[r]);
		printf("\n");
		fflush(stdout);
	}
	free(samples);
	git_libgit2_shutdown();
	return failed;
}

#define BENCH_MAIN(arr) \
	int main(void) { return bench_main(arr, sizeof(arr) / sizeof(arr[0])); }

#endif
