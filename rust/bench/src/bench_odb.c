/* odb: loose objects (loose.git, ~34k loose objects), the git.git pack
 * (~425k objects in one 330 MB pack), and the mempack backend.
 *
 * Every read workload opens a fresh git_odb in setup() (untimed), so the
 * odb's object cache and the pack's delta-base cache start cold; the OS page
 * cache is warm after the warm-up runs. */
#include "bench.h"

/* ------------------------------------------------------------------ */
/* loose */

static git_oid *loose_ids;
static size_t loose_n;
static git_odb *odb;

static int collect_cb(const git_oid *id, void *payload)
{
	size_t *cap = payload;
	if (loose_n == *cap) loose_ids = xrealloc(loose_ids, (*cap = *cap ? *cap * 2 : 4096) * sizeof(git_oid));
	loose_ids[loose_n++] = *id;
	return 0;
}

/* loose.git holds ~34k objects (870 MB inflated).  The workloads use
 * random subsets: LOOSE_READ objects smaller than 32 KiB (commits, trees and
 * typical source files) for full reads, LOOSE_HDR for headers/exists. */
#define LOOSE_READ 2000
#define LOOSE_HDR 10000
#define LOOSE_MISS 2000
static git_oid loose_small[LOOSE_READ];

static void prep_loose(void *ctx)
{
	static int done;
	(void)ctx;
	if (!done) {
		git_odb *o;
		size_t cap = 0, i, k = 0, len;
		git_object_t t;
		CHECK(git_odb_open(&o, bench_data("loose.git/objects")));
		CHECK(git_odb_foreach(o, collect_cb, &cap));
		if (loose_n < LOOSE_HDR) { fprintf(stderr, "loose.git too small\n"); exit(2); }
		rng_seed(2);
		shuffle(loose_ids, loose_n, sizeof(git_oid));
		for (i = 0; i < loose_n && k < LOOSE_READ; i++) {
			CHECK(git_odb_read_header(&len, &t, o, &loose_ids[loose_n - 1 - i]));
			if (len < 32768) loose_small[k++] = loose_ids[loose_n - 1 - i];
		}
		git_odb_free(o);
		done = 1;
	}
	bench_ops = LOOSE_HDR;
}
static void prep_loose_read(void *c) { prep_loose(c); bench_ops = LOOSE_READ; }
static void prep_loose_foreach(void *c) { prep_loose(c); bench_ops = loose_n; }

static void open_loose(void *ctx) { (void)ctx; CHECK(git_odb_open(&odb, bench_data("loose.git/objects"))); }
static void open_pack(void *ctx) { (void)ctx; CHECK(git_odb_open(&odb, bench_data("git.git/objects"))); }
static void close_odb(void *ctx) { (void)ctx; git_odb_free(odb); odb = NULL; }

static uint64_t read_ids(const git_oid *ids, size_t n)
{
	uint64_t h = FNV_INIT;
	size_t i;
	for (i = 0; i < n; i++) {
		git_odb_object *o;
		CHECK(git_odb_read(&o, odb, &ids[i]));
		h = fnv_u64(h, git_odb_object_type(o));
		h = fnv_sample(h, git_odb_object_data(o), git_odb_object_size(o));
		git_odb_object_free(o);
	}
	return h;
}

static uint64_t header_ids(const git_oid *ids, size_t n)
{
	uint64_t h = FNV_INIT;
	size_t i, len;
	git_object_t t;
	for (i = 0; i < n; i++) {
		CHECK(git_odb_read_header(&len, &t, odb, &ids[i]));
		h = fnv_u64(h, len * 8 + (uint64_t)t);
	}
	return h;
}

/* ids that are not in any odb: flip bits of existing ones */
static git_oid *missing_ids(const git_oid *ids, size_t n)
{
	git_oid *m = xmalloc(n * sizeof(*m));
	size_t i;
	for (i = 0; i < n; i++) {
		m[i] = ids[i];
		m[i].id[19] ^= 0x5a;
		m[i].id[10] ^= 0xa5;
	}
	return m;
}

/* n lookups of existing ids, then nmiss of missing ones (each miss makes
 * the odb refresh its backends, as in the C) */
static uint64_t exists_ids(const git_oid *ids, size_t n, const git_oid *miss, size_t nmiss)
{
	uint64_t c = 0;
	size_t i;
	for (i = 0; i < n; i++)
		c += (uint64_t)git_odb_exists(odb, &ids[i]);
	for (i = 0; i < nmiss; i++)
		c += (uint64_t)git_odb_exists(odb, &miss[i]) * 1000000;
	return c;
}

static int count_cb(const git_oid *id, void *payload)
{
	uint64_t *c = payload;
	c[0]++;
	c[1] ^= id->id[0] | ((uint64_t)id->id[1] << 8) | ((uint64_t)id->id[19] << 16);
	return 0;
}

static uint64_t run_foreach(void *ctx)
{
	uint64_t c[2] = { 0, 0 };
	(void)ctx;
	CHECK(git_odb_foreach(odb, count_cb, c));
	return c[0] << 32 ^ c[1];
}

static uint64_t run_loose_read(void *ctx) { (void)ctx; return read_ids(loose_small, LOOSE_READ); }
static uint64_t run_loose_header(void *ctx) { (void)ctx; return header_ids(loose_ids, LOOSE_HDR); }

static git_oid *loose_missing;
static void prep_loose_exists(void *ctx)
{
	prep_loose(ctx);
	if (!loose_missing) loose_missing = missing_ids(loose_ids, LOOSE_MISS);
	bench_ops = LOOSE_HDR + LOOSE_MISS;
}
static uint64_t run_loose_exists(void *ctx) { (void)ctx; return exists_ids(loose_ids, LOOSE_HDR, loose_missing, LOOSE_MISS); }

/* write: 500 objects (blobs of 0..64 KiB, tree-like and commit-like
 * payloads) into a fresh, empty loose odb */
#define WRITE_N 500
static char *write_buf;
static size_t write_len[WRITE_N];
static size_t write_off[WRITE_N];
static const char *write_dir;

static void prep_write(void *ctx)
{
	size_t i, total = 0;
	(void)ctx;
	if (!write_buf) {
		for (i = 0; i < WRITE_N; i++) {
			write_len[i] = (i % 10 == 0) ? 16384 + (i * 37) % 49152 : 64 + (i * 131) % 4000;
			write_off[i] = total;
			total += write_len[i];
		}
		write_buf = xmalloc(total);
		rng_seed(3);
		for (i = 0; i < total; i++)
			write_buf[i] = "abcdefghij klmnop\nqrstuvwxyz0123456789{}();"[rng_next() % 43];
	}
	bench_ops = WRITE_N;
}

static void setup_write(void *ctx)
{
	(void)ctx;
	write_dir = bench_tmpdir("odb-write");
	CHECK(git_odb_open(&odb, write_dir));
}

static uint64_t run_write(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i;
	git_oid id;
	(void)ctx;
	for (i = 0; i < WRITE_N; i++) {
		CHECK(git_odb_write(&id, odb, write_buf + write_off[i], write_len[i], GIT_OBJECT_BLOB));
		h = fnv_oid(h, &id);
	}
	return h;
}

static void teardown_write(void *ctx)
{
	(void)ctx;
	git_odb_free(odb);
	odb = NULL;
	rm_rf(write_dir);
}

/* write the same objects again: they exist, so git_odb_write only freshens */
static void setup_rewrite(void *ctx)
{
	size_t i;
	git_oid id;
	setup_write(ctx);
	for (i = 0; i < WRITE_N; i++)
		CHECK(git_odb_write(&id, odb, write_buf + write_off[i], write_len[i], GIT_OBJECT_BLOB));
	git_odb_free(odb);
	CHECK(git_odb_open(&odb, write_dir));
}

/* ------------------------------------------------------------------ */
/* pack */

static pack_entry *pe;     /* by offset */
static size_t pe_n;
static git_oid *rand_ids;  /* random sample */
#define PACK_RANDOM 2000
#define PACK_SEQ 4000
#define PACK_HEADER 20000
#define PACK_EXISTS 50000
#define PACK_MISS 2000
#define PACK_PREFIX 20000
#define PACK_RPREFIX 1000
static git_oid *seq_ids, *hdr_ids, *exists_miss, *prefix_ids;

static void prep_pack(void *ctx)
{
	size_t i;
	(void)ctx;
	if (pe) return;
	pe = pack_entries(bench_data("git.git/objects"), &pe_n);
	rand_ids = xmalloc(PACK_EXISTS * sizeof(git_oid));
	rng_seed(4);
	for (i = 0; i < PACK_EXISTS; i++)
		rand_ids[i] = pe[rng_next() % pe_n].id;
	/* sequential: a contiguous window of the pack (trees and blobs, many
	 * of them deltified), in pack order */
	seq_ids = xmalloc(PACK_SEQ * sizeof(git_oid));
	for (i = 0; i < PACK_SEQ; i++)
		seq_ids[i] = pe[pe_n / 3 + i].id;
	hdr_ids = rand_ids;
	exists_miss = missing_ids(rand_ids, PACK_MISS);
	/* prefixes: from the random sample, zero beyond the 7th nibble */
	prefix_ids = xmalloc(PACK_PREFIX * sizeof(git_oid));
	for (i = 0; i < PACK_PREFIX; i++) {
		prefix_ids[i] = rand_ids[PACK_EXISTS - 1 - i];
		prefix_ids[i].id[3] &= 0xf0;
		memset(prefix_ids[i].id + 4, 0, 16);
	}
}

static void prep_pack_random(void *c) { prep_pack(c); bench_ops = PACK_RANDOM; }
static void prep_pack_seq(void *c) { prep_pack(c); bench_ops = PACK_SEQ; }
static void prep_pack_header(void *c) { prep_pack(c); bench_ops = PACK_HEADER; }
static void prep_pack_exists(void *c) { prep_pack(c); bench_ops = PACK_EXISTS + PACK_MISS; }
static void prep_pack_prefix(void *c) { prep_pack(c); bench_ops = PACK_PREFIX; }
static void prep_pack_rprefix(void *c) { prep_pack(c); bench_ops = PACK_RPREFIX; }
static void prep_pack_foreach(void *c) { prep_pack(c); bench_ops = pe_n; }

static uint64_t run_pack_random(void *ctx) { (void)ctx; return read_ids(rand_ids, PACK_RANDOM); }
static uint64_t run_pack_seq(void *ctx) { (void)ctx; return read_ids(seq_ids, PACK_SEQ); }
static uint64_t run_pack_header(void *ctx) { (void)ctx; return header_ids(hdr_ids, PACK_HEADER); }
static uint64_t run_pack_exists(void *ctx) { (void)ctx; return exists_ids(rand_ids, PACK_EXISTS, exists_miss, PACK_MISS); }

static uint64_t run_pack_prefix(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i;
	git_oid out;
	(void)ctx;
	for (i = 0; i < PACK_PREFIX; i++) {
		int e = git_odb_exists_prefix(&out, odb, &prefix_ids[i], 7);
		if (e == GIT_EAMBIGUOUS) { h = fnv_u64(h, 1); continue; }
		CHECK(e);
		h = fnv_oid(h, &out);
	}
	return h;
}

static uint64_t run_pack_rprefix(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i;
	(void)ctx;
	for (i = 0; i < PACK_RPREFIX; i++) {
		git_odb_object *o;
		int e = git_odb_read_prefix(&o, odb, &prefix_ids[i], 7);
		if (e == GIT_EAMBIGUOUS) { h = fnv_u64(h, 1); continue; }
		CHECK(e);
		h = fnv_oid(h, git_odb_object_id(o));
		h = fnv_u64(h, git_odb_object_size(o));
		git_odb_object_free(o);
	}
	return h;
}

/* ------------------------------------------------------------------ */
/* mempack */

#define MEMPACK_N 20000
static git_odb_backend *mempack;
static git_oid mem_ids[MEMPACK_N];

static size_t mempack_count(void)
{
	size_t c;
	CHECK(git_mempack_object_count(&c, mempack));
	return c;
}

static void prep_mempack(void *c) { prep_write(c); bench_ops = MEMPACK_N; }

static void setup_mempack(void *ctx)
{
	(void)ctx;
	CHECK(git_odb_new(&odb));
	CHECK(git_mempack_new(&mempack));
	CHECK(git_odb_add_backend(odb, mempack, 1000));
}

static uint64_t run_mempack_write(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i;
	(void)ctx;
	for (i = 0; i < MEMPACK_N; i++) {
		size_t k = i % WRITE_N, len = write_len[k] / 4 + i / WRITE_N;
		CHECK(git_odb_write(&mem_ids[i], odb, write_buf + write_off[k], len, GIT_OBJECT_BLOB));
		h = fnv_oid(h, &mem_ids[i]);
	}
	return fnv_u64(h, mempack_count());
}

static void setup_mempack_full(void *ctx)
{
	setup_mempack(ctx);
	run_mempack_write(ctx);
}

static uint64_t run_mempack_read(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i, j;
	(void)ctx;
	/* fresh odb objects each time: bypass nothing, read through the cache */
	for (j = 0; j < 2; j++)
		for (i = 0; i < MEMPACK_N; i++) {
			git_odb_object *o;
			size_t len;
			git_object_t t;
			CHECK(git_odb_read(&o, odb, &mem_ids[i]));
			h = fnv_u64(h, git_odb_object_size(o));
			git_odb_object_free(o);
			CHECK(git_odb_read_header(&len, &t, odb, &mem_ids[i]));
			h += len;
			h += (uint64_t)git_odb_exists(odb, &mem_ids[i]);
		}
	return h;
}

static uint64_t run_mempack_reset(void *ctx)
{
	(void)ctx;
	CHECK(git_mempack_reset(mempack));
	return mempack_count();
}

static bench_t benches[] = {
	{ "odb", "loose_read", prep_loose_read, open_loose, run_loose_read, close_odb, NULL, 0 },
	{ "odb", "loose_read_header", prep_loose, open_loose, run_loose_header, close_odb, NULL, 0 },
	{ "odb", "loose_exists", prep_loose_exists, open_loose, run_loose_exists, close_odb, NULL, 0 },
	{ "odb", "loose_foreach", prep_loose_foreach, open_loose, run_foreach, close_odb, NULL, 0 },
	{ "odb", "loose_write", prep_write, setup_write, run_write, teardown_write, NULL, 0 },
	{ "odb", "loose_write_existing", prep_write, setup_rewrite, run_write, teardown_write, NULL, 0 },
	{ "odb", "pack_read_random", prep_pack_random, open_pack, run_pack_random, close_odb, NULL, 0 },
	{ "odb", "pack_read_sequential", prep_pack_seq, open_pack, run_pack_seq, close_odb, NULL, 0 },
	{ "odb", "pack_read_header", prep_pack_header, open_pack, run_pack_header, close_odb, NULL, 0 },
	{ "odb", "pack_exists", prep_pack_exists, open_pack, run_pack_exists, close_odb, NULL, 0 },
	{ "odb", "pack_exists_prefix", prep_pack_prefix, open_pack, run_pack_prefix, close_odb, NULL, 0 },
	{ "odb", "pack_read_prefix", prep_pack_rprefix, open_pack, run_pack_rprefix, close_odb, NULL, 0 },
	{ "odb", "pack_foreach", prep_pack_foreach, open_pack, run_foreach, close_odb, NULL, 0 },
	{ "odb", "mempack_write", prep_mempack, setup_mempack, run_mempack_write, close_odb, NULL, 0 },
	{ "odb", "mempack_read", prep_mempack, setup_mempack_full, run_mempack_read, close_odb, NULL, 0 },
	{ "odb", "mempack_reset", prep_mempack, setup_mempack_full, run_mempack_reset, close_odb, NULL, 0 },
};

BENCH_MAIN(benches)
