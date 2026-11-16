/*
 * Differential test of src/util/zstream.c: deflated bytes (the system
 * zlib's in both), inflating valid, truncated and corrupted streams (the
 * C's zlib against the Rust's zlib-rs: return codes, error classes and
 * messages, output), and the chunked streaming API (in_len, written
 * lengths, done/eos at each step).
 */

#include "git2_util.h"
#include "git2/global.h"
#include "git2/errors.h"
#include "zstream.h"
#include "hash.h"

static unsigned long seed = 99;
static unsigned rnd(void)
{
	seed = seed * 6364136223846793005UL + 1442695040888963407UL;
	return (unsigned)(seed >> 33);
}

static void digest(const char *what, int rc, const git_str *s)
{
	unsigned char h[20];
	char x[41];
	git_hash_buf(h, s->ptr, s->size, GIT_HASH_ALGORITHM_SHA1);
	git_hash_fmt(x, h, 20);
	/* Not asize, nor the NUL: when the output fills the buffer exactly
	 * the C leaves it without its NUL, the Rust grows it (DEFECTS.md). */
	printf("%s rc=%d size=%zu %s\n", what, rc, s->size, x);
}

static void result(const char *what, int rc, size_t size)
{
	const git_error *e = git_error_last();
	if (rc < 0)
		printf("%s rc=%d size=%zu class=%d msg=%s\n", what, rc, size, e ? e->klass : -1, e ? e->message : "");
	else
		printf("%s rc=%d size=%zu\n", what, rc, size);
}

static void fill(char *b, size_t n, int kind)
{
	size_t i;
	for (i = 0; i < n; i++) {
		switch (kind) {
		case 0: b[i] = (char)rnd(); break;                      /* incompressible */
		case 1: b[i] = "abcdefgh\n"[rnd() % 9]; break;           /* text-like */
		default: b[i] = (char)((i / 1000) & 0xff); break;      /* runs */
		}
	}
}

static void stream(const char *what, git_zstream_t type, const char *in, size_t len, size_t outsz)
{
	git_zstream zs = GIT_ZSTREAM_INIT;
	char out[4096];
	int rc, steps = 0;
	size_t total = 0;

	rc = git_zstream_init(&zs, type);
	printf("%s init rc=%d\n", what, rc);
	git_zstream_set_input(&zs, in, len);
	while (!git_zstream_done(&zs) && steps < 100000) {
		size_t n = outsz;
		rc = git_zstream_get_output_chunk(out, &n, &zs);
		if (rc < 0) {
			result("  chunk", rc, total);
			break;
		}
		total += n;
		if (steps < 40 || git_zstream_done(&zs))
			printf("  step %d wrote=%zu in_len=%zu eos=%d done=%d suggest=%zu\n", steps, n,
				zs.in_len, git_zstream_eos(&zs), git_zstream_done(&zs),
				git_zstream_suggest_output_len(&zs));
		steps++;
	}
	printf("  total=%zu steps=%d\n", total, steps);
	git_zstream_reset(&zs);
	printf("  reset eos=%d done=%d in_len=%zu\n", git_zstream_eos(&zs), git_zstream_done(&zs), zs.in_len);
	git_zstream_free(&zs);
}

int main(void)
{
	static char raw[300000];
	git_str def = GIT_STR_INIT, inf = GIT_STR_INIT, bad = GIT_STR_INIT;
	size_t sizes[] = { 0, 1, 7, 8, 9, 100, 1000, 65535, 65536, 100000, 300000 };
	size_t s, k, i;
	int rc;

	git_libgit2_init();

	for (k = 0; k < 3; k++) {
		fill(raw, sizeof(raw), (int)k);
		for (s = 0; s < ARRAY_SIZE(sizes); s++) {
			char what[64];
			git_str_clear(&def);
			git_str_clear(&inf);
			rc = git_zstream_deflatebuf(&def, raw, sizes[s]);
			snprintf(what, sizeof(what), "deflate kind=%zu len=%zu", k, sizes[s]);
			digest(what, rc, &def);
			rc = git_zstream_inflatebuf(&inf, def.ptr, def.size);
			snprintf(what, sizeof(what), "inflate kind=%zu len=%zu", k, sizes[s]);
			digest(what, rc, &inf);
			printf("  same=%d\n", inf.size == sizes[s] && memcmp(inf.ptr, raw, sizes[s]) == 0);
		}
	}

	/* truncated and corrupted streams */
	fill(raw, 5000, 1);
	git_str_clear(&def);
	git_zstream_deflatebuf(&def, raw, 5000);
	for (i = 0; i <= def.size; i++) {
		char what[64];
		git_str_clear(&inf);
		git_error_clear();
		rc = git_zstream_inflatebuf(&inf, def.ptr, i);
		snprintf(what, sizeof(what), "truncated %zu", i);
		result(what, rc, inf.size);
	}
	for (i = 0; i < 3000; i++) {
		char what[64];
		size_t flips = 1 + rnd() % 3, f;
		git_str_clear(&bad);
		git_str_put(&bad, def.ptr, def.size);
		for (f = 0; f < flips; f++)
			bad.ptr[rnd() % bad.size] ^= (char)(1 << (rnd() % 8));
		git_str_clear(&inf);
		git_error_clear();
		rc = git_zstream_inflatebuf(&inf, bad.ptr, bad.size);
		snprintf(what, sizeof(what), "corrupt %zu", i);
		result(what, rc, inf.size);
	}
	/* random garbage, and other headers */
	for (i = 0; i < 500; i++) {
		char what[64], g[64];
		size_t n = 1 + rnd() % 64, j;
		for (j = 0; j < n; j++)
			g[j] = (char)rnd();
		if (i % 2) { g[0] = 0x78; if (n > 1) g[1] = (char)(0x9c + (rnd() % 3) - 1); }
		git_str_clear(&inf);
		git_error_clear();
		rc = git_zstream_inflatebuf(&inf, g, n);
		snprintf(what, sizeof(what), "garbage %zu", i);
		result(what, rc, inf.size);
	}
	/* trailing garbage */
	git_str_clear(&bad);
	git_str_put(&bad, def.ptr, def.size);
	git_str_puts(&bad, "trailing");
	git_str_clear(&inf);
	git_error_clear();
	result("trailing", git_zstream_inflatebuf(&inf, bad.ptr, bad.size), inf.size);
	{
		git_zstream zs = GIT_ZSTREAM_INIT;
		char out[10000];
		size_t n = sizeof(out);
		git_zstream_init(&zs, GIT_ZSTREAM_INFLATE);
		git_zstream_set_input(&zs, bad.ptr, bad.size);
		git_error_clear();
		rc = git_zstream_get_output(out, &n, &zs);
		result("get_output trailing 1", rc, n);
		printf("  in_len=%zu eos=%d\n", zs.in_len, git_zstream_eos(&zs));
		n = sizeof(out);
		git_error_clear();
		rc = git_zstream_get_output(out, &n, &zs);
		result("get_output trailing 2", rc, n);
		git_zstream_free(&zs);
	}

	/* streaming with small buffers */
	fill(raw, 20000, 1);
	stream("deflate 7", GIT_ZSTREAM_DEFLATE, raw, 20000, 7);
	stream("deflate 4096", GIT_ZSTREAM_DEFLATE, raw, 20000, 4096);
	git_str_clear(&def);
	git_zstream_deflatebuf(&def, raw, 20000);
	stream("inflate 1", GIT_ZSTREAM_INFLATE, def.ptr, def.size, 1);
	stream("inflate 13", GIT_ZSTREAM_INFLATE, def.ptr, def.size, 13);
	stream("inflate 4096", GIT_ZSTREAM_INFLATE, def.ptr, def.size, 4096);
	stream("inflate truncated", GIT_ZSTREAM_INFLATE, def.ptr, def.size / 2, 100);

	/* get_output with less room than the output */
	{
		git_zstream zs = GIT_ZSTREAM_INIT;
		char out[1000];
		size_t n;
		int r;
		git_zstream_init(&zs, GIT_ZSTREAM_INFLATE);
		git_zstream_set_input(&zs, def.ptr, def.size);
		for (r = 0; r < 30 && !git_zstream_done(&zs); r++) {
			n = sizeof(out);
			rc = git_zstream_get_output(out, &n, &zs);
			printf("get_output rc=%d n=%zu in_len=%zu eos=%d\n", rc, n, zs.in_len, git_zstream_eos(&zs));
		}
		git_zstream_free(&zs);
	}

	/* free and reset of an uninitialized stream */
	{
		git_zstream zs = GIT_ZSTREAM_INIT;
		git_zstream_free(&zs);
		printf("free of GIT_ZSTREAM_INIT ok\n");
	}

	git_str_dispose(&def);
	git_str_dispose(&inf);
	git_str_dispose(&bad);
	git_libgit2_shutdown();
	return 0;
}
