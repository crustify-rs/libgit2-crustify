/* Per-call cost of the l-obj functions (signature, tag and object header
 * parsing and formatting, object ids, grafts, the object table), through
 * their internal C ABI: ns per call, the minimum over chunks of 200 calls.
 *
 * difftest-link: libgit2_tests
 *
 * Run:    RUST_BUILD=/work/builds/l-obj ./run_lobj.sh [ROUNDS=5] [FILTER]
 */
#include "common.h"
#include "git2/global.h"
#include "git2/object.h"
#include "git2/signature.h"
#include "git2/blob.h"
#include "object.h"
#include "signature.h"
#include "tag.h"
#include "grafts.h"
#include "str.h"
#include <time.h>

static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec * 1e3 + t.tv_nsec / 1e6; }
static volatile size_t sink;
static int g_reps = 5; /* LOBJ_REPS */
static uint64_t st = 0x9E3779B97F4A7C15ull;
static uint64_t rng(void) { st ^= st >> 12; st ^= st << 25; st ^= st >> 27; return st * 2685821657736338717ull; }

/* The minimum over chunks of CHUNK calls (pinned and loaded machines
 * preempt long loops: a minimum over short chunks keeps that noise out),
 * over LOBJ_REPS passes of n calls. */
#define CHUNK 200
#define BENCH(name, n, setup, body) do { double best = 1e18; int r_; size_t i, c_; \
	if (only && !strstr(name, only)) break; \
	for (r_ = 0; r_ < g_reps; r_++) { setup; for (c_ = 0; c_ < (n); c_ += CHUNK) { double t = now(); \
		for (i = c_; i < c_ + CHUNK && i < (n); i++) { body; } t = now() - t; if (t < best) best = t; } } \
	printf("bench %-22s %8.2f ns\n", name, best * 1e6 / CHUNK); } while (0)

#define K 1024
static char sigs[K][160];
static size_t siglen[K];
static char tags[K][600];
static size_t taglen[K];
static git_oid oids[K];
static git_signature parsed[K];

static const char *first[] = { "Junio C Hamano", "Linus Torvalds", "Jeff King", "Ævar Arnfjörð Bjarmason", "René Scharfe", "A", "Johannes Schindelin" };
static const char *mail[] = { "gitster@pobox.com", "torvalds@linux-foundation.org", "peff@peff.net", "avarab@gmail.com", "l.s.r@web.de", "a@b", "johannes.schindelin@gmx.de" };

int main(int argc, char **argv)
{
	const char *only = argc > 1 ? argv[1] : NULL;
	size_t j;
	git_str out = GIT_STR_INIT;
	git_str grafts_text = GIT_STR_INIT;
	git_grafts *grafts;
	static char blob[4096];

	if (getenv("LOBJ_REPS"))
		g_reps = atoi(getenv("LOBJ_REPS"));
	git_libgit2_init();
	for (j = 0; j < K; j++) {
		size_t b;
		int off = (int)(rng() % 1500) - 750;
		oids[j].type = GIT_OID_SHA1;
		for (b = 0; b < 20; b++)
			oids[j].id[b] = (unsigned char)rng();
		siglen[j] = (size_t)snprintf(sigs[j], sizeof(sigs[j]), "author %s <%s> %u %c%02d%02d\n",
			first[j % 7], mail[(j / 7) % 7], (unsigned)(1100000000 + rng() % 600000000),
			off < 0 ? '-' : '+', abs(off) / 60, abs(off) % 60);
		taglen[j] = (size_t)snprintf(tags[j], sizeof(tags[j]),
			"object %s\ntype commit\ntag v2.%zu.%zu\ntagger %s <%s> 1700000000 +0100\n\nGit %zu.%zu\n-----BEGIN PGP SIGNATURE-----\n\niQIzBAABCAAdFiEE4fA2sf7nIh/HeOzvsLXohpav5ssFAmXfRF4ACgkQsLXohpav\n5stVcQ/+NxXr9lUYsApZw4qGHahhVfR4q13t5bYRSDu3k1n7qoqy5aT0VoaMEFG4\n-----END PGP SIGNATURE-----\n",
			git_oid_tostr_s(&oids[j]), j / 10, j % 10, first[j % 7], mail[j % 7], j / 10, j % 10);
		memset(&parsed[j], 0, sizeof(parsed[j]));
		{
			const char *p = sigs[j];
			git_signature__parse(&parsed[j], &p, sigs[j] + siglen[j], "author ", '\n');
		}
		git_str_printf(&grafts_text, "%s", git_oid_tostr_s(&oids[j]));
		if (j % 2)
			git_str_printf(&grafts_text, " %s", git_oid_tostr_s(&oids[(j + 1) % K]));
		git_str_putc(&grafts_text, '\n');
	}
	for (j = 0; j < sizeof(blob); j++)
		blob[j] = (char)('a' + j % 26);

	BENCH("signature_parse", 2000000, , {
		git_signature s; const char *p = sigs[i % K];
		git_signature__parse(&s, &p, sigs[i % K] + siglen[i % K], "author ", '\n');
		sink += s.when.offset; git__free(s.name); git__free(s.email); });
	BENCH("signature_from_buffer", 1000000, , {
		git_signature *s; git_signature_from_buffer(&s, sigs[i % K] + 7);
		sink += (size_t)s->when.time; git_signature_free(s); });
	BENCH("signature_new", 1000000, , {
		git_signature *s; git_signature_new(&s, first[i % 7], mail[i % 7], 1234567890, 60);
		sink += s->name[0]; git_signature_free(s); });
	BENCH("signature_dup", 2000000, , {
		git_signature *s; git_signature_dup(&s, &parsed[i % K]);
		sink += s->email[0]; git_signature_free(s); });
	BENCH("signature_writebuf", 2000000, git_str_clear(&out), {
		if (out.size > 1 << 20) git_str_clear(&out);
		git_signature__writebuf(&out, "committer ", &parsed[i % K]); });
	sink += out.size;
	BENCH("signature_equal", 10000000, , { sink += git_signature__equal(&parsed[i % K], &parsed[(i + 1) % K]); });
	BENCH("tag_from_raw", 1000000, , {
		git_object *o; git_object__from_raw(&o, tags[i % K], taglen[i % K], GIT_OBJECT_TAG, GIT_OID_SHA1);
		sink += ((git_tag *)o)->type; git_object_free(o); });
	BENCH("tag_rawcontent_valid", 1000000, , {
		int v; git_object_rawcontent_is_valid(&v, tags[i % K], taglen[i % K], GIT_OBJECT_TAG, GIT_OID_SHA1);
		sink += v; });
	BENCH("blob_from_raw_4k", 500000, , {
		git_object *o; git_object__from_raw(&o, blob, 64 + i % 4000, GIT_OBJECT_BLOB, GIT_OID_SHA1);
		sink += git_blob_rawsize((git_blob *)o); git_object_free(o); });
	BENCH("id_from_buffer_64", 2000000, , {
		git_oid id; git_object_id_from_buffer(&id, blob + i % 64, 64, NULL); sink += id.id[0]; });
	BENCH("parse_oid_header", 10000000, , {
		git_oid id; const char *p = tags[i % K];
		git_object__parse_oid_header(&id, &p, tags[i % K] + taglen[i % K], "object ", GIT_OID_SHA1);
		sink += id.id[1]; });
	BENCH("write_oid_header", 5000000, git_str_clear(&out), {
		if (out.size > 1 << 20) git_str_clear(&out);
		git_object__write_oid_header(&out, "parent ", &oids[i % K]); });
	BENCH("type2string", 50000000, , { sink += git_object_type2string((git_object_t)(i % 6))[0]; });
	BENCH("stringn2type", 20000000, , { sink += git_object_stringn2type(i & 1 ? "blob" : "commit", i & 1 ? 4 : 6); });
	BENCH("grafts_parse_1k", 2000, , {
		git_grafts *g; git_grafts_new(&g, GIT_OID_SHA1);
		git_grafts_parse(g, grafts_text.ptr, grafts_text.size); sink += git_grafts_size(g); git_grafts_free(g); });
	git_grafts_new(&grafts, GIT_OID_SHA1);
	git_grafts_parse(grafts, grafts_text.ptr, grafts_text.size);
	BENCH("grafts_get", 20000000, , {
		git_commit_graft *c = NULL; sink += git_grafts_get(&c, grafts, &oids[(i * 7) % K]) == 0; });
	BENCH("grafts_get_miss", 20000000, , {
		git_commit_graft *c = NULL; git_oid o = oids[i % K]; o.id[5] ^= 0x55;
		sink += git_grafts_get(&c, grafts, &o) == 0; });
	git_grafts_free(grafts);
	git_str_dispose(&out);
	git_str_dispose(&grafts_text);
	fprintf(stderr, "sink %zu\n", (size_t)sink);
	return 0;
}
