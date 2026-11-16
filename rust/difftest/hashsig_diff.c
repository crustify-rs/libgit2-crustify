/*
 * Differential test of similarity signatures (git_hashsig): the scores of
 * many pairs of buffers and files, under every option, against the C
 * reference and the Rust port.
 *
 * Inputs: synthetic buffers (random lines; edits of them: changed,
 * inserted, deleted and moved lines; whitespace and CRLF variants; NULs;
 * long lines; tiny, empty and blank contents; more than 127 runs, so both
 * heaps fill; heavy ties) and real files of $BENCH_DATA/git-wd (pairs of
 * files of the same directory, and each file with an edited copy), each
 * signed both from memory and from a file (which reads in 4096-byte
 * blocks).
 *
 * The C reads one byte past the input when it ends in whitespace being
 * skipped (see DEFECTS.md); a file whose 4096-byte block ends so gives
 * the C an indeterminate byte. The synthetic files avoid that case, so
 * that both outputs are determined.
 *
 * difftest-link: libgit2_tests
 */

#include <dirent.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "git2.h"
#include "git2/sys/hashsig.h"

static const int opts[] = {
	GIT_HASHSIG_NORMAL,
	GIT_HASHSIG_IGNORE_WHITESPACE,
	GIT_HASHSIG_SMART_WHITESPACE,
	GIT_HASHSIG_ALLOW_SMALL_FILES,
	GIT_HASHSIG_IGNORE_WHITESPACE | GIT_HASHSIG_ALLOW_SMALL_FILES,
	GIT_HASHSIG_SMART_WHITESPACE | GIT_HASHSIG_ALLOW_SMALL_FILES,
	GIT_HASHSIG_IGNORE_WHITESPACE | GIT_HASHSIG_SMART_WHITESPACE,
};
#define NOPTS (int)(sizeof(opts) / sizeof(opts[0]))

static uint64_t seed = 12345;
static uint32_t rnd(void)
{
	seed = seed * 6364136223846793005ULL + 1442695040888963407ULL;
	return (uint32_t)(seed >> 33);
}

typedef struct {
	char *p;
	size_t n, a;
} buf;

static void put(buf *b, const char *s, size_t n)
{
	if (b->n + n + 1 > b->a) {
		b->a = (b->n + n + 1) * 2;
		b->p = realloc(b->p, b->a);
	}
	memcpy(b->p + b->n, s, n);
	b->n += n;
	b->p[b->n] = 0;
}

static void puts_(buf *b, const char *s)
{
	put(b, s, strlen(s));
}

/* a random "source line" */
static void line(buf *b, int vocab, int ws)
{
	static const char *words[] = { "int", "return", "if", "else", "for", "x", "y", "z", "foo",
				       "bar", "(", ")", "{", "}", ";", "=", "+", "0", "1", "char *" };
	int i, n = 1 + rnd() % 8;
	if (ws && rnd() % 3 == 0)
		put(b, "\t\t", rnd() % 3);
	for (i = 0; i < n; i++) {
		puts_(b, words[rnd() % (unsigned)vocab]);
		if (rnd() % 4)
			put(b, "  ", ws ? 1 + rnd() % 2 : 1);
	}
	if (ws && rnd() % 5 == 0)
		puts_(b, " \t");
	if (ws && rnd() % 4 == 0)
		puts_(b, "\r");
	puts_(b, "\n");
	if (rnd() % 7 == 0)
		puts_(b, "\n");
}

/* an edit of `in`: drop, duplicate, change and whitespace-vary lines */
static void edit(buf *out, const buf *in, int rate)
{
	const char *p = in->p, *end = in->p + in->n;
	while (p < end) {
		const char *nl = memchr(p, '\n', (size_t)(end - p));
		size_t len = nl ? (size_t)(nl - p) + 1 : (size_t)(end - p);
		uint32_t r = rnd() % 100;
		if (r < (uint32_t)rate / 3) {
			/* dropped */
		} else if (r < (uint32_t)rate / 2) {
			line(out, 20, 1);
		} else if (r < (uint32_t)rate) {
			/* whitespace change */
			if (rnd() % 2)
				puts_(out, "  ");
			put(out, p, len);
			if (len > 1 && rnd() % 2)
				puts_(out, "\r\n");
		} else {
			put(out, p, len);
		}
		p += len;
	}
}

static int cnt = 0;

static void sig_buf(git_hashsig **out, const buf *b, int opt, const char *label)
{
	int e = git_hashsig_create(out, b->p ? b->p : "", b->n, opt);
	if (e < 0) {
		const git_error *err = git_error_last();
		printf("%s opt=%d create -> %d %s\n", label, opt, e, err ? err->message : "");
		*out = NULL;
	}
}

static void sig_file(git_hashsig **out, const char *path, int opt, const char *label)
{
	int e = git_hashsig_create_fromfile(out, path, opt);
	if (e < 0) {
		const git_error *err = git_error_last();
		printf("%s opt=%d fromfile -> %d %s\n", label, opt, e, err ? err->message : "");
		*out = NULL;
	}
}

static void write_file(const char *path, const buf *b)
{
	FILE *f = fopen(path, "wb");
	if (b->n)
		fwrite(b->p, 1, b->n, f);
	fclose(f);
}

static char tmpdir[256];

/* compare a and b from memory and from files, under each option */
static void pair(const buf *a, const buf *b, const char *label)
{
	char pa[512], pb[512];
	int o;
	snprintf(pa, sizeof(pa), "%s/a", tmpdir);
	snprintf(pb, sizeof(pb), "%s/b", tmpdir);
	write_file(pa, a);
	write_file(pb, b);
	printf("%s:", label);
	for (o = 0; o < NOPTS; o++) {
		git_hashsig *sa, *sb, *fa, *fb;
		sig_buf(&sa, a, opts[o], label);
		sig_buf(&sb, b, opts[o], label);
		sig_file(&fa, pa, opts[o], label);
		sig_file(&fb, pb, opts[o], label);
		printf(" [%d:", opts[o]);
		if (sa && sb)
			printf(" %d %d", git_hashsig_compare(sa, sb), git_hashsig_compare(sb, sa));
		if (fa && fb)
			printf(" f%d", git_hashsig_compare(fa, fb));
		if (sa && fa)
			printf(" m%d", git_hashsig_compare(sa, fa));
		printf("]");
		git_hashsig_free(sa);
		git_hashsig_free(sb);
		git_hashsig_free(fa);
		git_hashsig_free(fb);
	}
	printf("\n");
	cnt++;
}

/* does the content have a 4096-byte block that ends in skippable space? */
static int ends_block_in_space(const buf *b)
{
	size_t k;
	for (k = 4096; k <= b->n; k += 4096) {
		char c = b->p[k - 1];
		if (c == ' ' || c == '\t' || c == '\f' || c == '\r' || c == '\v')
			return 1;
	}
	return 0;
}

static void synthetic(void)
{
	int round;
	for (round = 0; round < 400; round++) {
		buf a = { 0 }, b = { 0 };
		char label[64];
		int lines = (round % 10 == 0) ? (int)(rnd() % 4) : (int)(rnd() % (round % 3 ? 60 : 2000));
		int vocab = 3 + rnd() % 17, i;
		for (i = 0; i < lines; i++)
			line(&a, vocab, round % 2);
		if (round % 13 == 0)
			put(&a, "bin\0ary\0\0data\n", 14);
		if (round % 17 == 0) {
			for (i = 0; i < 300; i++)
				put(&a, "x", 1);
			puts_(&a, "\n");
		}
		switch (round % 5) {
		case 0: edit(&b, &a, 0); break;
		case 1: edit(&b, &a, 10); break;
		case 2: edit(&b, &a, 40); break;
		case 3: for (i = 0; i < lines; i++) line(&b, vocab, 1); break;
		default: edit(&b, &a, 90); break;
		}
		if (ends_block_in_space(&a) || ends_block_in_space(&b)) {
			free(a.p);
			free(b.p);
			continue;
		}
		snprintf(label, sizeof(label), "syn%d n=%zu,%zu", round, a.n, b.n);
		pair(&a, &b, label);
		free(a.p);
		free(b.p);
	}
	{
		buf e = { 0 }, blank = { 0 }, ws = { 0 };
		puts_(&blank, "\n\n\n");
		puts_(&ws, "   \n\t\n \r\n");
		pair(&e, &e, "empty-empty");
		pair(&e, &blank, "empty-blank");
		pair(&blank, &ws, "blank-ws");
		pair(&ws, &ws, "ws-ws");
		free(blank.p);
		free(ws.p);
	}
}

static int read_file(const char *path, buf *b)
{
	FILE *f = fopen(path, "rb");
	char chunk[65536];
	size_t n;
	if (!f)
		return -1;
	while ((n = fread(chunk, 1, sizeof(chunk), f)) > 0)
		put(b, chunk, n);
	fclose(f);
	return 0;
}

static void real_files(const char *dir, int depth, int *budget)
{
	DIR *d = opendir(dir);
	struct dirent *de;
	char prev[4096] = "";
	if (!d)
		return;
	while ((de = readdir(d)) != NULL && *budget > 0) {
		char path[4096];
		struct stat st;
		if (de->d_name[0] == '.')
			continue;
		snprintf(path, sizeof(path), "%s/%s", dir, de->d_name);
		if (lstat(path, &st) < 0)
			continue;
		if (S_ISDIR(st.st_mode)) {
			if (depth < 2)
				real_files(path, depth + 1, budget);
			continue;
		}
		if (!S_ISREG(st.st_mode) || st.st_size > (1 << 20) || rnd() % 4)
			continue;
		{
			buf a = { 0 }, b = { 0 }, e = { 0 };
			if (read_file(path, &a) < 0)
				continue;
			if (prev[0] && read_file(prev, &b) == 0 && !ends_block_in_space(&a) && !ends_block_in_space(&b))
				pair(&a, &b, "with-previous");
			edit(&e, &a, 15);
			if (!ends_block_in_space(&a) && !ends_block_in_space(&e))
				pair(&a, &e, de->d_name);
			free(a.p);
			free(b.p);
			free(e.p);
			(*budget)--;
		}
		snprintf(prev, sizeof(prev), "%s", path);
	}
	closedir(d);
}

int main(void)
{
	const char *data = getenv("BENCH_DATA");
	char wd[4096];
	int budget = 600;

	git_libgit2_init();
	snprintf(tmpdir, sizeof(tmpdir), "/tmp/hashsig_diff.%d", (int)getpid());
	mkdir(tmpdir, 0700);
	synthetic();
	snprintf(wd, sizeof(wd), "%s/git-wd", data ? data : "/work/bench-data");
	real_files(wd, 0, &budget);
	printf("pairs: %d\n", cnt);
	{
		char p[512];
		snprintf(p, sizeof(p), "%s/a", tmpdir);
		unlink(p);
		snprintf(p, sizeof(p), "%s/b", tmpdir);
		unlink(p);
		rmdir(tmpdir);
	}
	git_libgit2_shutdown();
	return 0;
}
