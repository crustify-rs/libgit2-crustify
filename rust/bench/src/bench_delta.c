/* delta: applying deltas, through the public API.  Reading a deltified pack
 * object applies its whole delta chain (git_delta_apply per link, with the
 * pack's delta-base cache); reading its header uses git_delta_read_header;
 * the indexer resolves every delta of a pack it indexes.
 *
 * Objects come from git.git's pack (ofs-deltas, chains up to depth 50). */
#include "bench.h"

static pack_entry *pe;
static size_t pe_n;
static int *depth;
static git_odb *odb;

static size_t find_offset(uint64_t off)
{
	size_t lo = 0, hi = pe_n;
	while (lo < hi) {
		size_t mid = (lo + hi) / 2;
		if (pe[mid].offset < off) lo = mid + 1;
		else hi = mid;
	}
	if (lo == pe_n || pe[lo].offset != off) { fprintf(stderr, "bad base offset\n"); exit(2); }
	return lo;
}

static int chain_depth(size_t i)
{
	if (depth[i] >= 0) return depth[i];
	if (pe[i].type != 6) return depth[i] = 0; /* base object (or ref-delta: not in this pack) */
	return depth[i] = 1 + chain_depth(find_offset(pe[i].base_offset));
}

#define RANDOM_N 1500
#define DEEP_N 300
#define HDR_N 20000
static git_oid random_ids[RANDOM_N], deep_ids[DEEP_N], hdr_ids[HDR_N];
static uint64_t deep_depth_sum;

static void prep(void *ctx)
{
	size_t i, k, nd = 0;
	size_t *deltas;
	(void)ctx;
	if (pe) return;
	pe = pack_entries(bench_data("git.git/objects"), &pe_n);
	depth = xmalloc(pe_n * sizeof(*depth));
	for (i = 0; i < pe_n; i++) depth[i] = -1;
	deltas = xmalloc(pe_n * sizeof(*deltas));
	for (i = 0; i < pe_n; i++)
		if (pe[i].type == 6 && chain_depth(i) > 0)
			deltas[nd++] = i;
	rng_seed(8);
	shuffle(deltas, nd, sizeof(*deltas));
	for (i = 0; i < RANDOM_N; i++)
		random_ids[i] = pe[deltas[i]].id;
	for (i = 0; i < HDR_N; i++)
		hdr_ids[i] = pe[deltas[nd - 1 - i]].id;
	/* the deepest chains: depth >= 45 */
	for (i = 0, k = 0; i < nd && k < DEEP_N; i++)
		if (depth[deltas[i]] >= 45) {
			deep_ids[k++] = pe[deltas[i]].id;
			deep_depth_sum += (uint64_t)depth[deltas[i]];
		}
	if (k < DEEP_N) { fprintf(stderr, "only %zu deep chains\n", k); exit(2); }
	free(deltas);
	printf("# delta: %zu ofs-deltas of %zu objects; deep sample: mean depth %.1f\n",
		nd, pe_n, (double)deep_depth_sum / DEEP_N);
}

static void prep_random(void *c) { prep(c); bench_ops = RANDOM_N; }
static void prep_deep(void *c) { prep(c); bench_ops = DEEP_N; }
static void prep_hdr(void *c) { prep(c); bench_ops = HDR_N; }

static void open_odb(void *ctx) { (void)ctx; CHECK(git_odb_open(&odb, bench_data("git.git/objects"))); }
static void close_odb(void *ctx) { (void)ctx; git_odb_free(odb); odb = NULL; }

static uint64_t read_ids(const git_oid *ids, size_t n)
{
	uint64_t h = FNV_INIT;
	size_t i;
	for (i = 0; i < n; i++) {
		git_odb_object *o;
		CHECK(git_odb_read(&o, odb, &ids[i]));
		h = fnv_sample(h, git_odb_object_data(o), git_odb_object_size(o));
		git_odb_object_free(o);
	}
	return h;
}

static uint64_t run_random(void *ctx) { (void)ctx; return read_ids(random_ids, RANDOM_N); }
static uint64_t run_deep(void *ctx) { (void)ctx; return read_ids(deep_ids, DEEP_N); }

static uint64_t run_header(void *ctx)
{
	uint64_t h = FNV_INIT;
	size_t i, len;
	git_object_t t;
	(void)ctx;
	for (i = 0; i < HDR_N; i++) {
		CHECK(git_odb_read_header(&len, &t, odb, &hdr_ids[i]));
		h = fnv_u64(h, len * 8 + (uint64_t)t);
	}
	return h;
}

/* indexer: index a self-contained pack of v2.46.0..v2.47.0 (made by
 * prepare-data.sh), resolving all its deltas */
static char *pack_data;
static size_t pack_len;
static const char *idxdir;
static uint64_t pack_objects;

static void prep_indexer(void *ctx)
{
	(void)ctx;
	if (!pack_data) {
		pack_data = read_file(bench_data("packs/v2.46.0-v2.47.0.pack"), &pack_len);
		pack_objects = be32((unsigned char *)pack_data + 8);
		printf("# delta: indexer input %zu bytes, %" PRIu64 " objects\n", pack_len, pack_objects);
	}
	bench_ops = pack_objects;
}

static void setup_indexer(void *ctx) { (void)ctx; idxdir = bench_tmpdir("indexer"); }
static void teardown_indexer(void *ctx) { (void)ctx; rm_rf(idxdir); }

static uint64_t run_indexer(void *ctx)
{
	git_indexer *ix;
	git_indexer_options opts;
	git_indexer_progress st;
	size_t off;
	uint64_t h;
	(void)ctx;
	CHECK(git_indexer_options_init(&opts, GIT_INDEXER_OPTIONS_VERSION));
	CHECK(git_indexer_new(&ix, idxdir, &opts));
	for (off = 0; off < pack_len; off += 65536) {
		size_t n = pack_len - off < 65536 ? pack_len - off : 65536;
		CHECK(git_indexer_append(ix, pack_data + off, n, &st));
	}
	CHECK(git_indexer_commit(ix, &st));
	h = fnv_str(FNV_INIT, git_indexer_name(ix));
	h = fnv_u64(h, ((uint64_t)st.indexed_objects << 32) | st.total_objects);
	h = fnv_u64(h, st.indexed_deltas);
	git_indexer_free(ix);
	return h;
}

static bench_t benches[] = {
	{ "delta", "read_deltified_random", prep_random, open_odb, run_random, close_odb, NULL, 0 },
	{ "delta", "read_deep_chains", prep_deep, open_odb, run_deep, close_odb, NULL, 0 },
	{ "delta", "read_header_deltified", prep_hdr, open_odb, run_header, close_odb, NULL, 0 },
	{ "delta", "indexer_resolve", prep_indexer, setup_indexer, run_indexer, teardown_indexer, NULL, 0 },
};

BENCH_MAIN(benches)
