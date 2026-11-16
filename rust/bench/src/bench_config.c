/* config: big.config (~22k entries: 2,000 remotes, 2,000 branches, a
 * 2,000-value multivar, 8 included files of 500 entries each) and a typical
 * small repository config. */
#include "bench.h"

static git_config *cfg, *snap;
static const char *tmpdir;
static char tmpcfg[4096];

static void open_big(void *ctx) { (void)ctx; CHECK(git_config_open_ondisk(&cfg, bench_data("config/big.config"))); }
static void close_cfg(void *ctx) { (void)ctx; git_config_free(cfg); cfg = NULL; }

static void open_big_snap(void *ctx)
{
	open_big(ctx);
	CHECK(git_config_snapshot(&snap, cfg));
}
static void close_snap(void *ctx) { git_config_free(snap); snap = NULL; close_cfg(ctx); }

/* open + parse (including the 8 includes) 20 times */
static uint64_t run_open_big(void *ctx)
{
	uint64_t h = 0;
	int i;
	(void)ctx;
	for (i = 0; i < 20; i++) {
		git_config *c;
		int32_t v;
		CHECK(git_config_open_ondisk(&c, bench_data("config/big.config")));
		git_config_entry *e;
		CHECK(git_config_get_entry(&e, c, "sect7.sub49.key9")); /* from an include */
		h += strlen(e->value);
		git_config_entry_free(e);
		CHECK(git_config_get_int32(&v, c, "core.compression"));
		h += (uint64_t)v;
		git_config_free(c);
	}
	return h;
}

static uint64_t run_open_small(void *ctx)
{
	uint64_t h = 0;
	int i;
	(void)ctx;
	for (i = 0; i < 2000; i++) {
		git_config *c;
		int v;
		CHECK(git_config_open_ondisk(&c, bench_data("config/small.config")));
		CHECK(git_config_get_bool(&v, c, "core.filemode"));
		h += (uint64_t)v;
		git_config_free(c);
	}
	return h;
}

/* typed gets on a snapshot: strings, bools, ints, in included files too */
#define GET_N 20000
static uint64_t run_get_string(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i;
	(void)ctx;
	for (i = 0; i < GET_N; i++) {
		char key[128];
		const char *v;
		switch (i % 4) {
		case 0: snprintf(key, sizeof(key), "remote.r%04zu.url", i * 7 % 2000); break;
		case 1: snprintf(key, sizeof(key), "branch.feature/b%04zu.merge", i * 13 % 2000); break;
		case 2: snprintf(key, sizeof(key), "sect%zu.sub%zu.key%zu", i % 8, i % 50, i % 10); break;
		default: snprintf(key, sizeof(key), "remote.r%04zu.pushurl", i * 3 % 2000); break;
		}
		CHECK(git_config_get_string(&v, snap, key));
		h = fnv_str(h, v);
	}
	return h;
}

static uint64_t run_get_typed(void *ctx)
{
	uint64_t h = 0;
	size_t i;
	(void)ctx;
	for (i = 0; i < GET_N; i++) {
		char key[128];
		int b;
		int64_t v64;
		snprintf(key, sizeof(key), "branch.feature/b%04zu.rebase", i % 2000);
		CHECK(git_config_get_bool(&b, snap, key));
		h += (uint64_t)b;
		CHECK(git_config_get_int64(&v64, snap, "core.bigfilethreshold"));
		h += (uint64_t)v64 >> 20;
		if (git_config_get_bool(&b, snap, "core.missing") != GIT_ENOTFOUND) abort();
	}
	return h;
}

/* gets on the live (non-snapshot) config: git_config_get_entry and
 * git_config_get_string_buf (git_buf) */
#define GET_LIVE_N 5000
static uint64_t run_get_entry(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i;
	(void)ctx;
	for (i = 0; i < GET_LIVE_N; i++) {
		char key[128];
		git_config_entry *e;
		git_buf buf = GIT_BUF_INIT;
		snprintf(key, sizeof(key), "remote.r%04zu.url", i * 7 % 2000);
		CHECK(git_config_get_entry(&e, cfg, key));
		h = fnv_str(h, e->value);
		h = fnv_u64(h, (uint64_t)e->level);
		git_config_entry_free(e);
		snprintf(key, sizeof(key), "sect%zu.sub%zu.key%zu", i % 8, i % 50, i % 10);
		CHECK(git_config_get_string_buf(&buf, cfg, key));
		h = fnv(h, buf.ptr, buf.size);
		git_buf_dispose(&buf);
	}
	return h;
}

static uint64_t iterate(git_config *c, const char *re)
{
	git_config_iterator *it;
	git_config_entry *e;
	uint64_t n = 0, h = FNV_INIT;
	int err;
	if (re) CHECK(git_config_iterator_glob_new(&it, c, re));
	else CHECK(git_config_iterator_new(&it, c));
	while ((err = git_config_next(&e, it)) == 0) {
		n++;
		h = fnv_str(h, e->name);
		h = fnv_str(h, e->value);
	}
	if (err != GIT_ITEROVER) CHECK(err);
	git_config_iterator_free(it);
	return (n << 40) ^ (h & 0xffffffffffULL);
}

static uint64_t run_iterate(void *ctx) { (void)ctx; return iterate(snap, NULL) + iterate(cfg, NULL); }
static uint64_t run_iterate_glob(void *ctx) { (void)ctx; return iterate(snap, "^remote\\..*\\.url$"); }

static int foreach_cb(const git_config_entry *e, void *payload)
{
	uint64_t *h = payload;
	*h = *h * 31 + strlen(e->value);
	return 0;
}

/* the 2,000-value multivar, all values and a regex-filtered subset */
#define MV_ROUNDS 2
static uint64_t run_multivar_iter(void *ctx)
{
	uint64_t h = 0;
	git_config_iterator *it;
	git_config_entry *e;
	int err, i;
	(void)ctx;
	for (i = 0; i < MV_ROUNDS; i++) {
		CHECK(git_config_multivar_iterator_new(&it, snap, "multi.value", NULL));
		while ((err = git_config_next(&e, it)) == 0)
			h = h * 31 + strlen(e->value);
		if (err != GIT_ITEROVER) CHECK(err);
		git_config_iterator_free(it);
		CHECK(git_config_multivar_iterator_new(&it, snap, "multi.value", "1[0-9]7"));
		while ((err = git_config_next(&e, it)) == 0)
			h = h * 31 + strlen(e->value);
		if (err != GIT_ITEROVER) CHECK(err);
		git_config_iterator_free(it);
	}
	return h;
}

/* the two-valued remote.<name>.fetch of 20 remotes (each call scans the
 * whole configuration) */
#define MV_KEYS 20
static uint64_t run_multivar_foreach(void *ctx)
{
	uint64_t h = 0;
	int i;
	(void)ctx;
	for (i = 0; i < MV_KEYS; i++) {
		char key[64];
		snprintf(key, sizeof(key), "remote.r%04d.fetch", i * 97);
		CHECK(git_config_get_multivar_foreach(snap, key, NULL, foreach_cb, &h));
	}
	return h;
}

static uint64_t run_snapshot(void *ctx)
{
	uint64_t h = 0;
	int i;
	(void)ctx;
	for (i = 0; i < 5; i++) {
		git_config *s;
		const char *v;
		CHECK(git_config_snapshot(&s, cfg));
		CHECK(git_config_get_string(&v, s, "user.name"));
		h += strlen(v);
		git_config_free(s);
	}
	return h;
}

/* set: rewrite the file on every set */
static void setup_set(void *ctx, const char *which)
{
	(void)ctx;
	tmpdir = bench_tmpdir("config");
	snprintf(tmpcfg, sizeof(tmpcfg), "%s/config", tmpdir);
	copy_file(bench_data(which), tmpcfg);
	CHECK(git_config_open_ondisk(&cfg, tmpcfg));
}
static void setup_set_big(void *ctx) { setup_set(ctx, "config/big.config"); }
static void setup_set_small(void *ctx) { setup_set(ctx, "config/small.config"); }
static void teardown_set(void *ctx) { close_cfg(ctx); rm_rf(tmpdir); }

static uint64_t do_sets(int n)
{
	int i;
	size_t len;
	char *data;
	uint64_t h;
	for (i = 0; i < n; i++) {
		char key[64], val[64];
		switch (i % 4) {
		case 0:
			snprintf(key, sizeof(key), "remote.r%04d.url", i * 7 % 2000);
			snprintf(val, sizeof(val), "https://example.org/new/%d.git", i);
			CHECK(git_config_set_string(cfg, key, val));
			break;
		case 1:
			snprintf(key, sizeof(key), "bench.new%d", i);
			CHECK(git_config_set_int64(cfg, key, (int64_t)i * 1000));
			break;
		case 2:
			snprintf(key, sizeof(key), "branch.feature/b%04d.rebase", i % 2000);
			CHECK(git_config_set_bool(cfg, key, i & 1));
			break;
		default:
			snprintf(key, sizeof(key), "bench.multi");
			snprintf(val, sizeof(val), "v%d", i);
			CHECK(git_config_set_multivar(cfg, key, "^$", val));
			break;
		}
	}
	data = read_file(tmpcfg, &len);
	h = fnv(FNV_INIT, data, len);
	free(data);
	return h;
}

static uint64_t run_set_big(void *ctx) { (void)ctx; return do_sets(10); }
static uint64_t run_set_small(void *ctx) { (void)ctx; return do_sets(400); }

static uint64_t run_delete_entries(void *ctx)
{
	int i;
	size_t len;
	char *data;
	uint64_t h;
	(void)ctx;
	for (i = 0; i < 10; i++) {
		char key[64];
		snprintf(key, sizeof(key), "remote.r%04d.pushurl", i * 37 % 2000);
		CHECK(git_config_delete_entry(cfg, key));
	}
	data = read_file(tmpcfg, &len);
	h = fnv(FNV_INIT, data, len);
	free(data);
	return h;
}

/* config of a repository (local + empty global/system), and a lookup */
static uint64_t run_repo_config(void *ctx)
{
	uint64_t h = 0;
	int i;
	(void)ctx;
	for (i = 0; i < 500; i++) {
		git_repository *r;
		git_config *c;
		int v;
		CHECK(git_repository_open(&r, bench_data("git-wd")));
		CHECK(git_repository_config_snapshot(&c, r));
		CHECK(git_config_get_bool(&v, c, "core.bare"));
		h += (uint64_t)v + 1;
		git_config_free(c);
		git_repository_free(r);
	}
	return h;
}

static uint64_t run_parse_values(void *ctx)
{
	uint64_t h = 0;
	int i;
	(void)ctx;
	for (i = 0; i < 100000; i++) {
		int b;
		int32_t v32;
		int64_t v64;
		static const char *bools[] = { "true", "yes", "on", "false", "no", "off", "1", "0" };
		static const char *ints[] = { "42", "-7", "512k", "64m", "1g", "0x10" };
		CHECK(git_config_parse_bool(&b, bools[i % 8]));
		h += (uint64_t)b;
		if (git_config_parse_int32(&v32, ints[i % 5]) == 0) h += (uint64_t)v32;
		CHECK(git_config_parse_int64(&v64, ints[i % 5]));
		h += (uint64_t)v64;
	}
	return h;
}

static bench_t benches[] = {
	{ "config", "open_big_x20", NULL, NULL, run_open_big, NULL, NULL, 20 },
	{ "config", "open_small_x2000", NULL, NULL, run_open_small, NULL, NULL, 2000 },
	{ "config", "get_string_snapshot", NULL, open_big_snap, run_get_string, close_snap, NULL, GET_N },
	{ "config", "get_bool_int_snapshot", NULL, open_big_snap, run_get_typed, close_snap, NULL, GET_N * 3 },
	{ "config", "get_entry_live", NULL, open_big, run_get_entry, close_cfg, NULL, GET_LIVE_N * 2 },
	{ "config", "iterate_all", NULL, open_big_snap, run_iterate, close_snap, NULL, 2 },
	{ "config", "iterate_glob", NULL, open_big_snap, run_iterate_glob, close_snap, NULL, 1 },
	{ "config", "multivar_iterate", NULL, open_big_snap, run_multivar_iter, close_snap, NULL, MV_ROUNDS * 2 },
	{ "config", "multivar_foreach", NULL, open_big_snap, run_multivar_foreach, close_snap, NULL, MV_KEYS },
	{ "config", "snapshot_x5", NULL, open_big, run_snapshot, close_cfg, NULL, 5 },
	{ "config", "set_big_x10", NULL, setup_set_big, run_set_big, teardown_set, NULL, 10 },
	{ "config", "set_small_x400", NULL, setup_set_small, run_set_small, teardown_set, NULL, 400 },
	{ "config", "delete_big_x10", NULL, setup_set_big, run_delete_entries, teardown_set, NULL, 10 },
	{ "config", "repository_config_x500", NULL, NULL, run_repo_config, NULL, NULL, 500 },
	{ "config", "parse_bool_int", NULL, NULL, run_parse_values, NULL, NULL, 300000 },
};

BENCH_MAIN(benches)
