/*
 * Differential test of src/util/hash.c and its backends (sha1dc with
 * collision detection, SHA-256): digests of every length up to a few
 * blocks, fed whole and in random pieces, git_hash_vec, git_hash_fmt, the
 * errors (unknown algorithm, the shattered collision) and the digest that
 * git_hash_final writes on a collision.
 */

#include "git2_util.h"
#include "git2/global.h"
#include "git2/errors.h"
#include "hash.h"

static unsigned long seed = 777;
static unsigned rnd(void)
{
	seed = seed * 6364136223846793005UL + 1442695040888963407UL;
	return (unsigned)(seed >> 33);
}

static void hex(const char *what, int rc, const unsigned char *d, size_t n)
{
	char s[GIT_HASH_MAX_SIZE * 2 + 1];
	git_hash_fmt(s, (unsigned char *)d, n);
	printf("%s rc=%d %s\n", what, rc, s);
}

static void err(const char *what, int rc)
{
	const git_error *e = git_error_last();
	printf("%s rc=%d class=%d msg=%s\n", what, rc, e ? e->klass : -1, e ? e->message : "");
}

static int read_file(const char *path, unsigned char **out, size_t *len)
{
	FILE *f = fopen(path, "rb");
	long n;
	if (!f)
		return -1;
	fseek(f, 0, SEEK_END);
	n = ftell(f);
	rewind(f);
	*out = malloc(n);
	*len = fread(*out, 1, n, f);
	fclose(f);
	return 0;
}

int main(int argc, char **argv)
{
	static unsigned char data[4096];
	unsigned char out[GIT_HASH_MAX_SIZE];
	git_hash_ctx ctx;
	git_hash_algorithm_t algs[] = { GIT_HASH_ALGORITHM_SHA1, GIT_HASH_ALGORITHM_SHA256 };
	size_t a, n, i;
	const char *shattered = argc > 1 ? argv[1] : "tests/resources/sha1/shattered-1.pdf";

	git_libgit2_init();
	for (i = 0; i < sizeof(data); i++)
		data[i] = (unsigned char)rnd();

	for (a = 0; a < 2; a++) {
		size_t hs = git_hash_size(algs[a]);
		for (n = 0; n <= 1100; n++) {
			int rc = git_hash_buf(out, data, n, algs[a]);
			char what[64];
			snprintf(what, sizeof(what), "buf %d %zu", (int)algs[a], n);
			hex(what, rc, out, hs);
			if (n % 13 == 0) {
				size_t off = 0;
				rc = git_hash_ctx_init(&ctx, algs[a]);
				while (rc == 0 && off < n) {
					size_t k = 1 + rnd() % 150;
					if (k > n - off)
						k = n - off;
					rc = git_hash_update(&ctx, data + off, k);
					off += k;
				}
				if (rc == 0)
					rc = git_hash_final(out, &ctx);
				snprintf(what, sizeof(what), "ctx %d %zu", (int)algs[a], n);
				hex(what, rc, out, hs);
				/* restart and reuse */
				rc = git_hash_init(&ctx);
				if (rc == 0) rc = git_hash_update(&ctx, data, n / 2);
				if (rc == 0) rc = git_hash_final(out, &ctx);
				snprintf(what, sizeof(what), "reinit %d %zu", (int)algs[a], n / 2);
				hex(what, rc, out, hs);
				git_hash_ctx_cleanup(&ctx);
			}
		}
		{
			git_str_vec vec[3] = { { data, 100 }, { data + 100, 0 }, { data + 100, 900 } };
			int rc = git_hash_vec(out, vec, 3, algs[a]);
			hex("vec", rc, out, hs);
			rc = git_hash_vec(out, vec, 0, algs[a]);
			hex("vec0", rc, out, hs);
		}
	}

	/* a large input */
	{
		size_t big = 3 * 1024 * 1024 + 17;
		unsigned char *b = malloc(big);
		for (i = 0; i < big; i++)
			b[i] = (unsigned char)(i * 131 + (i >> 7));
		for (a = 0; a < 2; a++)
			hex("big", git_hash_buf(out, b, big, algs[a]), out, git_hash_size(algs[a]));
		free(b);
	}

	/* errors */
	git_error_clear();
	err("buf none", git_hash_buf(out, data, 10, GIT_HASH_ALGORITHM_NONE));
	git_error_clear();
	err("buf 7", git_hash_buf(out, data, 10, 7));
	git_error_clear();
	err("ctx_init none", git_hash_ctx_init(&ctx, GIT_HASH_ALGORITHM_NONE));
	git_error_clear();
	err("init none", git_hash_init(&ctx));
	git_error_clear();
	err("update none", git_hash_update(&ctx, data, 3));
	git_error_clear();
	err("final none", git_hash_final(out, &ctx));
	git_hash_ctx_cleanup(&ctx);

	/* the shattered collision: an error, and sha1dc's safe digest */
	{
		unsigned char *pdf;
		size_t len;
		if (read_file(shattered, &pdf, &len) == 0) {
			int rc;
			memset(out, 0, sizeof(out));
			git_error_clear();
			rc = git_hash_buf(out, pdf, len, GIT_HASH_ALGORITHM_SHA1);
			err("shattered buf", rc);
			hex("shattered buf digest", rc, out, 20);
			memset(out, 0, sizeof(out));
			git_error_clear();
			rc = git_hash_ctx_init(&ctx, GIT_HASH_ALGORITHM_SHA1);
			for (i = 0; rc == 0 && i < len; i += 1000)
				rc = git_hash_update(&ctx, pdf + i, len - i < 1000 ? len - i : 1000);
			rc = git_hash_final(out, &ctx);
			err("shattered ctx", rc);
			hex("shattered ctx digest", rc, out, 20);
			hex("shattered sha256", git_hash_buf(out, pdf, len, GIT_HASH_ALGORITHM_SHA256), out, 32);
			git_hash_ctx_cleanup(&ctx);
			free(pdf);
		} else {
			printf("no %s\n", shattered);
		}
	}

	git_libgit2_shutdown();
	return 0;
}
