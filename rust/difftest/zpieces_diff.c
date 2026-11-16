/* From the u2-r1 review. */
/* pack.c-style streaming: input in windows, re-set each call; print in_len etc. */
#include "git2_util.h"
#include "git2/global.h"
#include "git2/errors.h"
#include "zstream.h"

static unsigned long seed = 7;
static unsigned rnd(void) { seed = seed * 6364136223846793005UL + 1442695040888963407UL; return (unsigned)(seed >> 33); }

int main(void)
{
	static char raw[200000];
	git_str def = GIT_STR_INIT;
	size_t i, wins[] = {1, 2, 3, 5, 8, 17, 64, 257, 4096};
	size_t outs[] = {1, 7, 100, 258, 4096, 65536};
	size_t w, o;
	int kind;
	git_libgit2_init();
	for (kind = 0; kind < 3; kind++) {
		for (i = 0; i < sizeof(raw); i++)
			raw[i] = kind == 0 ? (char)rnd() : kind == 1 ? "abcdefgh\n"[rnd() % 9] : (char)((i / 999) & 0xff);
		git_str_clear(&def);
		git_zstream_deflatebuf(&def, raw, kind == 0 ? 5000 : sizeof(raw));
		git_str_puts(&def, "TRAILINGJUNK");
		for (w = 0; w < ARRAY_SIZE(wins); w++)
		for (o = 0; o < ARRAY_SIZE(outs); o++) {
			git_zstream zs = GIT_ZSTREAM_INIT;
			static char out[65536];
			size_t pos = 0, total = 0, steps = 0;
			unsigned long h = 0;
			git_zstream_init(&zs, GIT_ZSTREAM_INFLATE);
			while (steps < 2000000) {
				size_t win = def.size - pos < wins[w] ? def.size - pos : wins[w];
				size_t n = outs[o], j;
				int rc;
				git_zstream_set_input(&zs, def.ptr + pos, win);
				git_error_clear();
				rc = git_zstream_get_output_chunk(out, &n, &zs);
				if (rc < 0) { printf("k%d w%zu o%zu step %zu rc=%d msg=%s\n", kind, wins[w], outs[o], steps, rc, git_error_last() ? git_error_last()->message : ""); break; }
				pos += win - zs.in_len;
				for (j = 0; j < n; j++) h = h * 31 + (unsigned char)out[j];
				total += n;
				if (steps % 97 == 0) printf("k%d w%zu o%zu step %zu n=%zu pos=%zu in_len=%zu eos=%d\n", kind, wins[w], outs[o], steps, n, pos, zs.in_len, git_zstream_eos(&zs));
				steps++;
				if (git_zstream_eos(&zs)) break;
				if (win == 0 && n == 0) break;
			}
			printf("k%d w%zu o%zu END total=%zu pos=%zu steps=%zu h=%lx eos=%d\n", kind, wins[w], outs[o], total, pos, steps, h, git_zstream_eos(&zs));
			git_zstream_free(&zs);
		}
	}
	git_str_dispose(&def);
	return 0;
}
