/* Per-call cost of the u1 functions (oid, pool, pqueue, utf8, varint,
 * parse): best of 5 runs of N calls, in ns per call.
 *
 * difftest-link: libgit2_tests
 *
 * Build:  RUST_BUILD=/work/builds/u1 ../difftest/link.sh u1_ops.c out
 * Run:    ./run_u1.sh   (interleaves out/u1_ops_c and out/u1_ops_rust)
 */
#include "common.h"
#include "git2/global.h"
#include "git2/oid.h"
#include "oid.h"
#include "parse.h"
#include "pool.h"
#include "pqueue.h"
#include "utf8.h"
#include "varint.h"
#include <time.h>

static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec * 1e3 + t.tv_nsec / 1e6; }
static volatile size_t sink;
static uint64_t st = 0x9E3779B97F4A7C15ull;
static uint64_t rng(void) { st ^= st >> 12; st ^= st << 25; st ^= st >> 27; return st * 2685821657736338717ull; }
static int cmp_u64(const void *a, const void *b) { uint64_t x = *(const uint64_t *)a, y = *(const uint64_t *)b; return (x > y) - (x < y); }

#define K 4096
static git_oid oids[K], oids2[K];
static char hex[K][GIT_OID_SHA256_HEXSIZE + 1];
static uint64_t vals[K];
static unsigned char varints[K][16];

#define BENCH(name, n, setup, body) do { double best = 1e18; int r_; size_t i; \
	if (only && !strstr(name, only)) break; \
	for (r_ = 0; r_ < 5; r_++) { double t; setup; t = now(); for (i = 0; i < (n); i++) { body; } t = now() - t; if (t < best) best = t; } \
	printf("bench %-22s %8.2f ns\n", name, best * 1e6 / (n)); } while (0)

int main(int argc, char **argv)
{
	const char *only = argc > 1 ? argv[1] : NULL;
	char buf[80];
	size_t j;
	git_pool pool;
	git_pqueue pq;
	static char text[1 << 20];
	git_parse_ctx ctx;

	git_libgit2_init();
	for (j = 0; j < K; j++) {
		size_t b;
		oids[j].type = (j & 1) ? GIT_OID_SHA256 : GIT_OID_SHA1;
		for (b = 0; b < git_oid_size(oids[j].type); b++)
			oids[j].id[b] = (unsigned char)rng();
		oids2[j] = oids[j];
		if (j % 3 == 0) oids2[j].id[19] ^= 1;
		git_oid_fmt(hex[j], &oids[j]);
		hex[j][git_oid_hexsize(oids[j].type)] = '\0';
		vals[j] = rng() >> (rng() % 64);
		git_encode_varint(varints[j], 16, vals[j]);
	}
	for (j = 0; j + 1 < sizeof(text); j++)
		text[j] = (j % 61 == 60) ? '\n' : (char)('a' + j % 26);

	BENCH("oid_from_string", 10000000, , { git_oid o; git_oid_from_string(&o, hex[i % K], oids[i % K].type); sink += o.id[3]; });
	BENCH("oid_from_prefix7", 10000000, , { git_oid o; git_oid_from_prefix(&o, hex[i % K], 7, GIT_OID_SHA1); sink += o.id[2]; });
	BENCH("oid_fmt", 10000000, , { git_oid_fmt(buf, &oids[i % K]); sink += buf[5]; });
	BENCH("oid_tostr_s", 10000000, , { sink += git_oid_tostr_s(&oids[i % K])[7]; });
	BENCH("oid_tostr8", 10000000, , { sink += git_oid_tostr(buf, 8, &oids[i % K])[3]; });
	BENCH("oid_pathfmt", 10000000, , { git_oid_pathfmt(buf, &oids[i % K]); sink += buf[4]; });
	BENCH("oid_cmp", 20000000, , { sink += (size_t)git_oid_cmp(&oids[i % K], &oids2[i % K]); });
	BENCH("oid_equal", 20000000, , { sink += (size_t)git_oid_equal(&oids[i % K], &oids2[i % K]); });
	BENCH("oid_ncmp7", 20000000, , { sink += (size_t)git_oid_ncmp(&oids[i % K], &oids2[i % K], 7); });
	BENCH("oid_streq", 10000000, , { sink += (size_t)git_oid_streq(&oids[i % K], hex[(i + (i & 1)) % K]); });
	BENCH("oid_is_zero", 20000000, , { sink += (size_t)git_oid_is_zero(&oids[i % K]); });
	{
		git_oid_shorten *os = NULL;
		BENCH("oid_shorten_add", 20000, { git_oid_shorten_free(os); os = git_oid_shorten_new(0); },
			{ sink += (size_t)git_oid_shorten_add(os, hex[(i * 2) % K + 0] ); });
		git_oid_shorten_free(os);
	}
	git_pool_init(&pool, 1);
	BENCH("pool_malloc", 10000000, { git_pool_clear(&pool); }, { sink += (size_t)git_pool_malloc(&pool, (i & 31) + 1); });
	BENCH("pool_strndup", 10000000, { git_pool_clear(&pool); }, { sink += (size_t)git_pool_strndup(&pool, hex[i % K], 12); });
	git_pool_clear(&pool);
	git_pqueue_init(&pq, 0, K, cmp_u64);
	BENCH("pqueue_insert_pop", 10000000, { git_pqueue_clear(&pq); for (j = 0; j < 1000; j++) git_pqueue_insert(&pq, &vals[j]); },
		{ git_pqueue_insert(&pq, &vals[i % K]); sink += *(uint64_t *)git_pqueue_pop(&pq); });
	git_pqueue_free(&pq);
	BENCH("utf8_char_length_61", 5000000, , { sink += git_utf8_char_length(text + (i % 1000), 61); });
	BENCH("utf8_iterate", 20000000, , { uint32_t c; sink += (size_t)git_utf8_iterate(&c, "\xe2\x82\xac", 3) + c; });
	BENCH("varint_decode", 20000000, , { size_t l; sink += git_decode_varint(varints[i % K], &l) + l; });
	BENCH("varint_encode", 20000000, , { sink += (size_t)git_encode_varint((unsigned char *)buf, 16, vals[i % K]); });
	BENCH("parse_advance_line", 10000000, { git_parse_ctx_init(&ctx, text, sizeof(text) - 1); },
		{ if (!ctx.remain_len) git_parse_ctx_init(&ctx, text, sizeof(text) - 1); git_parse_advance_line(&ctx); sink += ctx.line_len; });
	BENCH("parse_ws_digit", 10000000, ,
		{ int64_t v; git_parse_ctx c2; git_parse_ctx_init(&c2, "   12345 x\n", 11); git_parse_advance_ws(&c2); git_parse_advance_digit(&v, &c2, 10); sink += (size_t)v; });
	fprintf(stderr, "sink %zu\n", (size_t)sink);
	git_libgit2_shutdown();
	return 0;
}
