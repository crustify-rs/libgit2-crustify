/*
 * Differential test of delta.c: git_delta_index_init / _size /
 * git_delta_create_from_index (through git_delta) on random and
 * pathological source/target pairs, and git_delta_apply /
 * git_delta_read_header on valid, mutated and random deltas. Prints return
 * codes, sizes, FNV-1a hashes of the outputs and error messages.
 *
 * difftest-link: libgit2_tests
 */
#include "common.h"
#include "git2/global.h"
#include "git2/errors.h"
#include "delta.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static uint64_t rs = 0x243F6A8885A308D3ull;
static uint64_t rnd(void)
{
	rs ^= rs << 13; rs ^= rs >> 7; rs ^= rs << 17;
	return rs;
}
static size_t below(size_t n) { return n ? (size_t)(rnd() % n) : 0; }

static uint64_t fnv(const void *p, size_t n)
{
	const unsigned char *s = p;
	uint64_t h = 1469598103934665603ull;
	while (n--) { h ^= *s++; h *= 1099511628211ull; }
	return h;
}

static const char *msg(void)
{
	const git_error *e = git_error_last();
	return (e && e->message) ? e->message : "";
}

static unsigned char *mutate(const unsigned char *src, size_t len, size_t *out_len, int edits)
{
	size_t cap = len + edits * 64 + 1, n = len, i;
	unsigned char *v = malloc(cap);
	memcpy(v, src, len);
	for (i = 0; i < (size_t)edits; i++) {
		size_t at = below(n + 1), k;
		switch (rnd() % 4) {
		case 0: if (n) v[below(n)] = (unsigned char)rnd(); break;
		case 1: k = below(48); memmove(v + at + k, v + at, n - at);
			{ size_t j; for (j = 0; j < k; j++) v[at + j] = (unsigned char)rnd(); }
			n += k; break;
		case 2: k = below(48); if (k > n - at) k = n - at;
			memmove(v + at, v + at + k, n - at - k); n -= k; break;
		default: /* duplicate a block */
			if (n) { size_t from = below(n), l = below(48);
				if (l > n - from) l = n - from;
				memmove(v + at + l, v + at, n - at);
				memmove(v + at, v + (from >= at ? from + l : from), l); n += l; }
		}
	}
	*out_len = n;
	return v;
}

static void apply_case(const char *tag, int i, const unsigned char *base, size_t blen,
	const unsigned char *d, size_t dlen)
{
	void *out = (void *)1; size_t olen = 77, b = 77, r = 77;
	int e = git_delta_apply(&out, &olen, base, blen, d, dlen);
	printf("%s %d apply %d %zu %016llx '%s'\n", tag, i, e, olen,
		out ? (unsigned long long)fnv(out, olen + 1) : 0ull, e ? msg() : "");
	git__free(out);
	e = git_delta_read_header(&b, &r, d, dlen);
	printf("%s %d header %d %zu %zu '%s'\n", tag, i, e, b, e ? 77 : r, e ? msg() : "");
}

static void delta_case(int i, const unsigned char *src, size_t slen,
	const unsigned char *trg, size_t tlen, size_t max)
{
	void *d = NULL; size_t dlen = 0;
	git_delta_index *index = NULL;
	int e = git_delta_index_init(&index, src, slen);
	printf("pair %d init %d %zu %zu %zu\n", i, e, slen, tlen,
		index ? git_delta_index_size(index) : 0);
	git_delta_index_free(index);
	e = git_delta(&d, &dlen, src, slen, trg, tlen, max);
	printf("pair %d delta %d max %zu len %zu %016llx '%s'\n", i, e, max, dlen,
		d ? (unsigned long long)fnv(d, dlen) : 0ull, e ? msg() : "");
	if (d) {
		apply_case("pair", i, src, slen, d, dlen);
		/* mutated deltas */
		{ int m; for (m = 0; m < 6; m++) {
			size_t ml; unsigned char *md = mutate(d, dlen, &ml, 1 + m % 3);
			apply_case("mut", i * 10 + m, src, slen, md, ml);
			free(md);
		} }
	}
	free(d);
}

static unsigned char *gen(size_t len, int kind)
{
	unsigned char *v = malloc(len + 1); size_t i;
	for (i = 0; i < len; i++) {
		switch (kind) {
		case 0: v[i] = (unsigned char)rnd(); break;
		case 1: v[i] = "abcd"[rnd() % 4]; break;
		case 2: v[i] = 'a'; break;
		case 3: v[i] = (unsigned char)(i % 16); break;
		case 4: v[i] = (unsigned char)((i / 7) % 251); break;
		default: v[i] = " etaoinshrdlu\n"[rnd() % 14]; break;
		}
	}
	return v;
}

int main(void)
{
	static const size_t sizes[] = { 0, 1, 2, 15, 16, 17, 31, 32, 33, 100, 1000, 4095, 4096,
		4097, 8192, 20000, 70000, 140000, 300000 };
	static const size_t maxes[] = { 0, 1, 4, 10, 50, 100, 1000, 8000, 9000, 100000 };
	int i = 0, k, s;
	git_libgit2_init();

	for (k = 0; k < 6; k++)
		for (s = 0; s < (int)ARRAY_SIZE(sizes); s++) {
			size_t slen = sizes[s], tlen;
			unsigned char *src = gen(slen, k), *trg;
			int mode = (int)below(4);
			if (mode == 0) { trg = gen(below(slen * 2 + 2), k); tlen = 0; tlen = below(slen * 2 + 2); free(trg); trg = gen(tlen, k); }
			else if (mode == 1) { trg = malloc(slen + 1); memcpy(trg, src, slen); tlen = slen; }
			else { trg = mutate(src, slen, &tlen, 1 + (int)below(40)); }
			delta_case(i++, src, slen, trg, tlen, maxes[below(ARRAY_SIZE(maxes))]);
			delta_case(i++, src, slen, trg, tlen, 0);
			free(src); free(trg);
		}

	/* periodic sources whose buckets overflow HASH_LIMIT (culling:
	 * 64..127 entries per bucket keep one, more keep every n-th) */
	{
		static const int periods[] = { 17, 3, 5, 33, 257 };
		static const int counts[] = { 1, 63, 64, 65, 100, 127, 128, 129, 200, 255, 256, 513, 1000 };
		size_t pi, ci;
		for (pi = 0; pi < ARRAY_SIZE(periods); pi++)
			for (ci = 0; ci < ARRAY_SIZE(counts); ci++) {
				size_t slen = (size_t)periods[pi] * 16 * counts[ci] + 1 + below(40), tlen, j;
				unsigned char *src = malloc(slen + 1), *trg;
				for (j = 0; j < slen; j++)
					src[j] = (unsigned char)('a' + (j % periods[pi]) % 26 + (j % periods[pi]) / 26);
				for (j = 0; j < slen / 997; j++)
					src[below(slen)] = (unsigned char)rnd();
				trg = mutate(src, slen, &tlen, 1 + (int)below(30));
				delta_case(i++, src, slen, trg, tlen, 0);
				delta_case(i++, src, slen, src, slen, 0);
				free(src); free(trg);
			}
	}

	/* many random small pairs */
	for (k = 0; k < 3000; k++) {
		size_t slen = below(3000), tlen;
		unsigned char *src = gen(slen, (int)below(6)), *trg = mutate(src, slen, &tlen, 1 + (int)below(30));
		delta_case(i++, src, slen, trg, tlen, below(3) ? 0 : below(4000));
		free(src); free(trg);
	}

	/* random deltas with a valid header */
	for (k = 0; k < 20000; k++) {
		unsigned char base[300], d[64]; size_t blen = below(sizeof(base)), dlen = 2 + below(60), j;
		for (j = 0; j < blen; j++) base[j] = (unsigned char)rnd();
		for (j = 0; j < dlen; j++) d[j] = (unsigned char)rnd();
		if (rnd() % 2) { d[0] = (unsigned char)(blen & 0x7f) | (blen > 127 ? 0x80 : 0);
			if (blen > 127) { memmove(d + 2, d + 1, dlen - 2); d[1] = (unsigned char)(blen >> 7); } }
		apply_case("rand", k, base, blen, d, dlen);
	}

	/* headers: truncation and overflow */
	for (k = 0; k < 14; k++) {
		unsigned char h[16]; memset(h, 0x80, sizeof(h)); h[k] = 0x01;
		apply_case("hdr", k, NULL, 0, h, (size_t)k + 1);
		apply_case("hdrt", k, NULL, 0, h, (size_t)k);
	}

	git_libgit2_shutdown();
	return 0;
}
