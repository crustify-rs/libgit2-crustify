/* refs: lookup, iteration, create/delete and reflogs on refs.git (56k refs:
 * 51k packed (1k real from git.git, 50k synthetic tags, 2k branches), 5,000
 * loose branches of which 2,000 shadow packed ones, each with a 10-entry
 * reflog, and refs/heads/main with a 10,000-entry reflog). */
#include "bench.h"

static git_repository *repo;
static const char *tmprepo;

static void open_refs(void *ctx) { (void)ctx; CHECK(git_repository_open(&repo, bench_data("refs.git"))); }
static void close_repo(void *ctx) { (void)ctx; git_repository_free(repo); repo = NULL; }

/* names to look up: loose, packed, shadowed, missing */
#define LOOKUP_N 20000
static char *names[LOOKUP_N];

static void prep_names(void *ctx)
{
	size_t i;
	(void)ctx;
	if (!names[0]) {
		rng_seed(6);
		for (i = 0; i < LOOKUP_N; i++) {
			char buf[128];
			switch (i % 4) {
			case 0: snprintf(buf, sizeof(buf), "refs/heads/loose/b%04u", (unsigned)(rng_next() % 5000)); break;
			case 1: snprintf(buf, sizeof(buf), "refs/tags/synth/t%05u", (unsigned)(rng_next() % 50000)); break;
			case 2: snprintf(buf, sizeof(buf), "refs/tags/v2.%u.0", (unsigned)(rng_next() % 47)); break;
			default: snprintf(buf, sizeof(buf), "refs/heads/missing/m%05u", (unsigned)(rng_next() % 50000)); break;
			}
			names[i] = xstrdup(buf);
		}
	}
	bench_ops = LOOKUP_N;
}

static uint64_t run_lookup(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i;
	(void)ctx;
	for (i = 0; i < LOOKUP_N; i++) {
		git_reference *r;
		int e = git_reference_lookup(&r, repo, names[i]);
		if (e == GIT_ENOTFOUND) { h = fnv_u64(h, 0); continue; }
		CHECK(e);
		h = fnv_oid(h, git_reference_target(r));
		git_reference_free(r);
	}
	return h;
}

static uint64_t run_name_to_id(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i;
	git_oid id;
	(void)ctx;
	for (i = 0; i < LOOKUP_N; i++) {
		int e = git_reference_name_to_id(&id, repo, names[i]);
		if (e == GIT_ENOTFOUND) { h = fnv_u64(h, 0); continue; }
		CHECK(e);
		h = fnv_oid(h, &id);
	}
	return h;
}

/* git_reference_dwim on short names (tries the rev-parse rules in order) */
#define DWIM_N 5000
static uint64_t run_dwim(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i;
	(void)ctx;
	for (i = 0; i < DWIM_N; i++) {
		git_reference *r;
		char buf[64];
		if (i % 2) snprintf(buf, sizeof(buf), "loose/b%04u", (unsigned)(i % 5000));
		else snprintf(buf, sizeof(buf), "synth/t%05u", (unsigned)(i * 7 % 50000));
		CHECK(git_reference_dwim(&r, repo, buf));
		h = fnv_str(h, git_reference_name(r));
		git_reference_free(r);
	}
	return h;
}

/* open refs.git and look up one packed ref, 100 times: the first lookup in a
 * fresh refdb (packed-refs not yet loaded) */
static uint64_t run_packed_load(void *ctx)
{
	uint64_t h = FNV_INIT;
	int i;
	(void)ctx;
	for (i = 0; i < 100; i++) {
		git_repository *rp;
		git_reference *r;
		CHECK(git_repository_open(&rp, bench_data("refs.git")));
		CHECK(git_reference_lookup(&r, rp, "refs/tags/synth/t25000"));
		h = fnv_oid(h, git_reference_target(r));
		git_reference_free(r);
		git_repository_free(rp);
	}
	return h;
}

static uint64_t iterate(const char *glob)
{
	git_reference_iterator *it;
	const char *name;
	uint64_t n = 0, h = FNV_INIT;
	int e;
	if (glob) CHECK(git_reference_iterator_glob_new(&it, repo, glob));
	else CHECK(git_reference_iterator_new(&it, repo));
	while ((e = git_reference_next_name(&name, it)) == 0) {
		n++;
		h ^= fnv_str(FNV_INIT, name); /* order-independent */
	}
	if (e != GIT_ITEROVER) CHECK(e);
	git_reference_iterator_free(it);
	return (n << 40) ^ (h & 0xffffffffffULL);
}

static uint64_t run_iterate_all(void *ctx) { (void)ctx; return iterate(NULL); }
static uint64_t run_iterate_glob(void *ctx) { (void)ctx; return iterate("refs/heads/loose/b1*"); }

static uint64_t run_iterate_objects(void *ctx)
{
	git_reference_iterator *it;
	git_reference *r;
	uint64_t n = 0, h = 0;
	int e;
	(void)ctx;
	CHECK(git_reference_iterator_new(&it, repo));
	while ((e = git_reference_next(&r, it)) == 0) {
		n++;
		if (git_reference_type(r) == GIT_REFERENCE_DIRECT)
			h += git_reference_target(r)->id[0];
		git_reference_free(r);
	}
	if (e != GIT_ITEROVER) CHECK(e);
	git_reference_iterator_free(it);
	return (n << 32) ^ h;
}

static int foreach_cb(const char *name, void *payload)
{
	uint64_t *h = payload;
	*h += strlen(name);
	return 0;
}

static uint64_t run_foreach_glob(void *ctx)
{
	uint64_t h = 0;
	(void)ctx;
	CHECK(git_reference_foreach_glob(repo, "refs/tags/*", foreach_cb, &h));
	return h;
}

/* create / delete loose refs in a scratch repository (objects from git.git) */
#define CREATE_N 2000
static git_oid targets[CREATE_N];

static void prep_create(void *ctx)
{
	(void)ctx;
	if (git_oid_is_zero(&targets[0])) {
		git_repository *r;
		git_revwalk *w;
		size_t i;
		CHECK(git_repository_open(&r, bench_data("git.git")));
		CHECK(git_revwalk_new(&w, r));
		CHECK(git_revwalk_push_ref(w, "refs/heads/master"));
		for (i = 0; i < CREATE_N; i++)
			CHECK(git_revwalk_next(&targets[i], w));
		git_revwalk_free(w);
		git_repository_free(r);
	}
	bench_ops = CREATE_N;
}

static void setup_tmprepo(void *ctx)
{
	(void)ctx;
	tmprepo = bench_alt_repo("refs", bench_data("git.git/objects"));
	CHECK(git_repository_open(&repo, tmprepo));
}

static void teardown_tmprepo(void *ctx)
{
	close_repo(ctx);
	rm_rf(tmprepo);
}

static uint64_t create_refs(int with_log)
{
	uint64_t h = FNV_INIT;
	size_t i;
	for (i = 0; i < CREATE_N; i++) {
		git_reference *r;
		char name[64];
		snprintf(name, sizeof(name), "refs/heads/topic/t%04zu", i);
		CHECK(git_reference_create(&r, repo, name, &targets[i], 0, with_log ? "bench: create" : NULL));
		h = fnv_str(h, git_reference_name(r));
		git_reference_free(r);
	}
	return h;
}

static uint64_t run_create(void *ctx) { (void)ctx; return create_refs(0); }

static void setup_delete(void *ctx)
{
	setup_tmprepo(ctx);
	create_refs(0);
	git_repository_free(repo);
	CHECK(git_repository_open(&repo, tmprepo));
}

static uint64_t run_delete(void *ctx)
{
	size_t i;
	(void)ctx;
	for (i = 0; i < CREATE_N; i++) {
		char name[64];
		snprintf(name, sizeof(name), "refs/heads/topic/t%04zu", i);
		CHECK(git_reference_remove(repo, name));
	}
	return iterate(NULL);
}

/* delete 20 packed refs from a 56k-entry packed-refs (rewrites it each time) */
#define PACKED_DEL 20
static void setup_packed(void *ctx)
{
	char path[4096];
	setup_tmprepo(ctx);
	snprintf(path, sizeof(path), "%s/packed-refs", tmprepo);
	copy_file(bench_data("refs.git/packed-refs"), path);
	git_repository_free(repo);
	CHECK(git_repository_open(&repo, tmprepo));
}

static uint64_t run_delete_packed(void *ctx)
{
	size_t i;
	(void)ctx;
	for (i = 0; i < PACKED_DEL; i++) {
		char name[64];
		snprintf(name, sizeof(name), "refs/tags/synth/t%05zu", i * 2311);
		CHECK(git_reference_remove(repo, name));
	}
	return iterate("refs/tags/synth/*");
}

/* update one branch 1,000 times with a reflog message (core.logAllRefUpdates) */
#define UPDATE_N 1000
static void setup_update(void *ctx)
{
	git_config *cfg;
	git_reference *r;
	setup_tmprepo(ctx);
	CHECK(git_repository_config(&cfg, repo));
	CHECK(git_config_set_string(cfg, "core.logallrefupdates", "true"));
	git_config_free(cfg);
	CHECK(git_reference_create(&r, repo, "refs/heads/main", &targets[0], 1, "init"));
	git_reference_free(r);
}

static uint64_t run_update_logged(void *ctx)
{
	git_reference *r;
	size_t i;
	git_reflog *log;
	uint64_t n;
	(void)ctx;
	for (i = 1; i <= UPDATE_N; i++) {
		CHECK(git_reference_lookup(&r, repo, "refs/heads/main"));
		{
			git_reference *nr;
			CHECK(git_reference_set_target(&nr, r, &targets[i % CREATE_N], "bench: update"));
			git_reference_free(nr);
		}
		git_reference_free(r);
	}
	CHECK(git_reflog_read(&log, repo, "refs/heads/main"));
	n = git_reflog_entrycount(log);
	git_reflog_free(log);
	return n;
}

/* reflogs of refs.git */
static uint64_t sum_reflog(const char *name)
{
	git_reflog *log;
	size_t i, n;
	uint64_t h = 0;
	CHECK(git_reflog_read(&log, repo, name));
	n = git_reflog_entrycount(log);
	for (i = 0; i < n; i++) {
		const git_reflog_entry *e = git_reflog_entry_byindex(log, i);
		h += (uint64_t)git_reflog_entry_committer(e)->when.time;
		h += git_reflog_entry_id_new(e)->id[0] + strlen(git_reflog_entry_message(e));
	}
	git_reflog_free(log);
	return (h << 16) ^ n;
}

static uint64_t run_reflog_read_big(void *ctx)
{
	uint64_t h = 0;
	int i;
	(void)ctx;
	for (i = 0; i < 5; i++)
		h += sum_reflog("refs/heads/main");
	return h;
}

static uint64_t run_reflog_read_many(void *ctx)
{
	uint64_t h = 0;
	size_t i;
	(void)ctx;
	for (i = 0; i < 2000; i++) {
		char name[64];
		snprintf(name, sizeof(name), "refs/heads/loose/b%04zu", i);
		h += sum_reflog(name);
	}
	return h;
}

/* read the 10,000-entry reflog, append 200 entries, write it back */
static void setup_reflog_copy(void *ctx)
{
	char path[4096];
	setup_tmprepo(ctx);
	snprintf(path, sizeof(path), "%s/logs/refs/heads", tmprepo);
	{
		char p2[4096];
		snprintf(p2, sizeof(p2), "%s/logs", tmprepo);
		mkdir(p2, 0777);
		snprintf(p2, sizeof(p2), "%s/logs/refs", tmprepo);
		mkdir(p2, 0777);
		mkdir(path, 0777);
	}
	snprintf(path, sizeof(path), "%s/logs/refs/heads/main", tmprepo);
	copy_file(bench_data("refs.git/logs/refs/heads/main"), path);
	{
		git_reference *r;
		CHECK(git_reference_create(&r, repo, "refs/heads/main", &targets[0], 1, NULL));
		git_reference_free(r);
	}
}

static uint64_t run_reflog_append(void *ctx)
{
	git_reflog *log;
	git_signature *sig;
	size_t i;
	uint64_t n;
	(void)ctx;
	CHECK(git_signature_new(&sig, "Bench Mark", "bench@example.com", 1700000000, 0));
	CHECK(git_reflog_read(&log, repo, "refs/heads/main"));
	for (i = 0; i < 200; i++)
		CHECK(git_reflog_append(log, &targets[i], sig, "bench: append"));
	CHECK(git_reflog_write(log));
	git_reflog_free(log);
	git_signature_free(sig);
	n = sum_reflog("refs/heads/main");
	return n;
}

static bench_t benches[] = {
	{ "refs", "open_lookup_packed_x100", NULL, NULL, run_packed_load, NULL, NULL, 100 },
	{ "refs", "lookup", prep_names, open_refs, run_lookup, close_repo, NULL, 0 },
	{ "refs", "name_to_id", prep_names, open_refs, run_name_to_id, close_repo, NULL, 0 },
	{ "refs", "dwim", NULL, open_refs, run_dwim, close_repo, NULL, DWIM_N },
	{ "refs", "iterate_all_names", NULL, open_refs, run_iterate_all, close_repo, NULL, 56028 },
	{ "refs", "iterate_all_refs", NULL, open_refs, run_iterate_objects, close_repo, NULL, 56028 },
	{ "refs", "iterate_glob", NULL, open_refs, run_iterate_glob, close_repo, NULL, 1 },
	{ "refs", "foreach_glob_tags", NULL, open_refs, run_foreach_glob, close_repo, NULL, 1 },
	{ "refs", "create_loose", prep_create, setup_tmprepo, run_create, teardown_tmprepo, NULL, 0 },
	{ "refs", "delete_loose", prep_create, setup_delete, run_delete, teardown_tmprepo, NULL, 0 },
	{ "refs", "delete_packed", prep_create, setup_packed, run_delete_packed, teardown_tmprepo, NULL, PACKED_DEL },
	{ "refs", "update_with_reflog", prep_create, setup_update, run_update_logged, teardown_tmprepo, NULL, UPDATE_N },
	{ "refs", "reflog_read_10k_entries_x5", NULL, open_refs, run_reflog_read_big, close_repo, NULL, 5 },
	{ "refs", "reflog_read_2000_reflogs", NULL, open_refs, run_reflog_read_many, close_repo, NULL, 2000 },
	{ "refs", "reflog_append_write", prep_create, setup_reflog_copy, run_reflog_append, teardown_tmprepo, NULL, 200 },
};

BENCH_MAIN(benches)
