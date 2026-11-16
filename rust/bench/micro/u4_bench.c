/*
 * Benchmarks of WP u4: URL parsing and formatting (net.c), the sorted
 * cache (sortedcache.c) and process spawning (unix/process.c), through
 * their internal C ABI.  Linked against the C reference objects or against
 * the Rust port (same symbols).  Prints "bench <name> median_ms <x> ...".
 *
 *   BENCH=u4_bench ./build.sh && BENCH=u4_bench ./run.sh
 */
#include "git2_util.h"
#include "git2/global.h"
#include "net.h"
#include "process.h"
#include "sortedcache.h"
#include "str.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
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

static const char *urls[] = {
	"https://github.com/libgit2/libgit2.git",
	"http://user:secret@example.com:8080/path/to/repo.git?service=git-upload-pack",
	"ssh://git@gitlab.example.org/group/sub/project.git",
	"https://[fe80::dcad:beff:fe00:0001]:9090/resource?x=1#frag",
	"git://git.kernel.org/pub/scm/git/git.git",
	"https://us%65r:pa%73s@host.example.com/a%20b/c",
	"file:///home/user/repos/project",
	"http://localhost/",
};
#define NURLS (sizeof(urls) / sizeof(urls[0]))

static const char *scps[] = {
	"git@github.com:libgit2/libgit2.git",
	"[git@github.com:22]:libgit2/libgit2.git",
	"user@[fe80::dcad:beff:fe00:0001]:repo",
	"host.example.com:path/to/repo",
};
#define NSCPS (sizeof(scps) / sizeof(scps[0]))

static void bench_url_parse(void)
{
	size_t i;
	for (i = 0; i < 400000; i++) {
		git_net_url u;
		CHECK(git_net_url_parse(&u, urls[i % NURLS]));
		g_sink += strlen(u.host ? u.host : "");
		git_net_url_dispose(&u);
	}
}

static void bench_url_parse_scp(void)
{
	size_t i;
	for (i = 0; i < 400000; i++) {
		git_net_url u;
		CHECK(git_net_url_parse_standard_or_scp(&u, scps[i % NSCPS]));
		g_sink += strlen(u.path);
		git_net_url_dispose(&u);
	}
}

static git_net_url parsed[NURLS];

static void bench_url_fmt(void)
{
	git_str buf = GIT_STR_INIT;
	size_t i;
	for (i = 0; i < 1000000; i++) {
		git_net_url *u = &parsed[i % NURLS];
		if (!u->host)
			u = &parsed[0];
		git_str_clear(&buf);
		CHECK(git_net_url_fmt(&buf, u));
		g_sink += buf.size;
	}
	git_str_dispose(&buf);
}

static void bench_url_join_redirect(void)
{
	size_t i;
	for (i = 0; i < 200000; i++) {
		git_net_url j = GIT_NET_URL_INIT;
		CHECK(git_net_url_joinpath(&j, &parsed[0], "/info/refs?service=git-upload-pack"));
		CHECK(git_net_url_apply_redirect(&j, "/elsewhere/info/refs?service=git-upload-pack",
			false, "/info/refs?service=git-upload-pack"));
		g_sink += strlen(j.path);
		git_net_url_dispose(&j);
	}
}

static void bench_url_pattern(void)
{
	const char *list = "localhost,127.0.0.1,.internal.example.com,*.corp.example.org:8080,"
		"10.0.0.1,foo.bar,*.github.com,example.net,.local,::1";
	size_t i;
	for (i = 0; i < 1000000; i++) {
		git_net_url *u = &parsed[i % NURLS];
		if (!u->host || !u->port)
			u = &parsed[0];
		g_sink += git_net_url_matches_pattern_list(u, list);
		g_sink += git_net_hostname_matches_cert(u->host, "*.example.com");
	}
}

/* ---- sortedcache: the packed-refs pattern of refdb_fs ---- */

typedef struct {
	unsigned char oid[20];
	unsigned int flags;
	char name[GIT_FLEX_ARRAY];
} packref;

static int packref_cmp(const void *a, const void *b)
{
	return strcmp(((const packref *)a)->name, ((const packref *)b)->name);
}

static const char *g_packed = "/work/bench-data/refs.git/packed-refs";
static char **g_names;
static size_t g_nnames;

static git_sortedcache *load_cache(void)
{
	git_sortedcache *sc;
	git_str buf = GIT_STR_INIT;
	char *scan, *eol;

	CHECK(git_sortedcache_new(&sc, offsetof(packref, name), NULL, NULL, packref_cmp, g_packed));
	if (git_sortedcache_lockandload(sc, &buf) <= 0) {
		fprintf(stderr, "cannot load %s\n", g_packed);
		exit(1);
	}
	for (scan = buf.ptr; *scan; scan = eol + 1) {
		packref *ref;
		if (!(eol = strchr(scan, '\n')))
			break;
		*eol = '\0';
		if (*scan == '#' || *scan == '^' || eol - scan < 42)
			continue;
		CHECK(git_sortedcache_upsert((void **)&ref, sc, scan + 41));
		memcpy(ref->oid, scan, 20);
	}
	git_sortedcache_wunlock(sc);
	git_str_dispose(&buf);
	return sc;
}

static void bench_sortedcache_load(void)
{
	int i;
	for (i = 0; i < 5; i++) {
		git_sortedcache *sc = load_cache();
		g_sink += git_sortedcache_entrycount(sc);
		git_sortedcache_free(sc);
	}
}

static git_sortedcache *g_sc;

static void bench_sortedcache_lookup(void)
{
	size_t i, pos;
	int r;
	CHECK(git_sortedcache_rlock(g_sc));
	for (i = 0; i < 1000000; i++) {
		const char *name = g_names[(i * 7919) % g_nnames];
		packref *ref = git_sortedcache_lookup(g_sc, name);
		g_sink += ref ? ref->oid[0] : 0;
		if ((i & 7) == 0) {
			r = git_sortedcache_lookup_index(&pos, g_sc, name);
			g_sink += pos + (size_t)r;
		}
		if ((i & 15) == 0)
			g_sink += (git_sortedcache_lookup(g_sc, "refs/heads/does-not-exist") == NULL);
	}
	for (i = 0; i < git_sortedcache_entrycount(g_sc); i++)
		g_sink += ((packref *)git_sortedcache_entry(g_sc, i))->flags;
	git_sortedcache_runlock(g_sc);
}

/* ---- process ---- */

static void bench_process_spawn(void)
{
	const char *args[] = { "/bin/true" };
	int i;
	for (i = 0; i < 200; i++) {
		git_process *p;
		git_process_options opts = GIT_PROCESS_OPTIONS_INIT;
		git_process_result r = GIT_PROCESS_RESULT_INIT;
		opts.capture_out = 1;
		CHECK(git_process_new(&p, args, 1, NULL, 0, &opts));
		CHECK(git_process_start(p));
		CHECK(git_process_wait(&r, p));
		git_process_close(p);
		git_process_free(p);
		g_sink += r.exitcode;
	}
}

static void bench_process_pipe(void)
{
	const char *args[] = { "/bin/cat" };
	static char chunk[16384];
	char rbuf[16384];
	int i;
	for (i = 0; i < 20; i++) {
		git_process *p;
		git_process_options opts = GIT_PROCESS_OPTIONS_INIT;
		size_t written = 0, total = 0;
		ssize_t n;
		int k;
		opts.capture_in = 1;
		opts.capture_out = 1;
		CHECK(git_process_new(&p, args, 1, NULL, 0, &opts));
		CHECK(git_process_start(p));
		for (k = 0; k < 64; k++) {
			size_t off = 0;
			while (off < sizeof(chunk)) {
				n = git_process_write(p, chunk + off, sizeof(chunk) - off);
				CHECK(n);
				off += (size_t)n;
			}
			written += off;
			/* read back what was written: the pipes never fill */
			while (total < written) {
				n = git_process_read(p, rbuf, sizeof(rbuf));
				CHECK(n);
				if (n == 0)
					break;
				total += (size_t)n;
			}
		}
		CHECK(git_process_close_in(p));
		while ((n = git_process_read(p, rbuf, sizeof(rbuf))) > 0)
			total += (size_t)n;
		CHECK(git_process_wait(NULL, p));
		git_process_close(p);
		git_process_free(p);
		g_sink += total;
	}
}

typedef struct { const char *name; void (*fn)(void); } bench;
static const bench benches[] = {
	{ "url_parse", bench_url_parse },
	{ "url_parse_scp", bench_url_parse_scp },
	{ "url_fmt", bench_url_fmt },
	{ "url_join_redirect", bench_url_join_redirect },
	{ "url_pattern", bench_url_pattern },
	{ "sortedcache_load", bench_sortedcache_load },
	{ "sortedcache_lookup", bench_sortedcache_lookup },
	{ "process_spawn", bench_process_spawn },
	{ "process_pipe", bench_process_pipe },
};

int main(int argc, char **argv)
{
	const char *filter = argc > 2 ? argv[2] : NULL;
	size_t b, i;

	if (argc > 1)
		g_iters = atoi(argv[1]);
	git_libgit2_init();

	for (i = 0; i < NURLS; i++)
		CHECK(git_net_url_parse(&parsed[i], urls[i]));

	g_sc = load_cache();
	g_nnames = git_sortedcache_entrycount(g_sc);
	g_names = malloc(g_nnames * sizeof(char *));
	for (i = 0; i < g_nnames; i++)
		g_names[i] = strdup(((packref *)git_sortedcache_entry(g_sc, i))->name);

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

	for (i = 0; i < g_nnames; i++)
		free(g_names[i]);
	free(g_names);
	git_sortedcache_free(g_sc);
	for (i = 0; i < NURLS; i++)
		git_net_url_dispose(&parsed[i]);
	git_libgit2_shutdown();
	return (int)(g_sink & 0);
}
