/*
 * Differential test of work package l-config: every line printed must be
 * the same when linked against the C reference and against the Rust port.
 *
 * The corpus is every configuration file under tests/resources (config*,
 * .gitmodules, ...), the config_file fuzzer corpus, and deterministic
 * mutations of each (bytes flipped, deleted, inserted; the syntax bytes
 * [ ] " \ ; # = newline NUL space tab and high bytes favoured). For each
 * input it prints:
 *
 * - the entries the memory backend reads (names, values, levels, types,
 *   origins), or the parse error (code, class, message with the line and
 *   column);
 * - the entries the file backend reads (with includes, from the input's
 *   own directory for the unmutated resources);
 * - for a share of the inputs, the file after each of a set of writes
 *   (set, set of an existing and of a new subsection key, set_multivar,
 *   delete, delete_multivar), byte for byte, and the result of each;
 * - the value parsers (bool, int32, int64) on each value read.
 *
 * Then the value parsers and name normalization on random strings.
 *
 * difftest-link: libgit2_tests
 *
 * Build and compare: RUST_BUILD=/work/builds/l-config rust/difftest/run.sh rust/difftest/config_diff.c
 */

#include "common.h"
#include "git2/global.h"
#include "git2/errors.h"
#include "git2/config.h"
#include "git2/sys/config.h"
#include "config_backend.h"
#include "futils.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#define TMPDIR "/tmp/l-config-difftest"
#define TMPFILE TMPDIR "/config"

static unsigned long seed = 20261116;
static unsigned rnd(void)
{
	seed = seed * 6364136223846793005UL + 1442695040888963407UL;
	return (unsigned)(seed >> 33);
}

static void put_escaped(const char *s, size_t len)
{
	size_t i;
	for (i = 0; i < len; i++) {
		unsigned char c = (unsigned char)s[i];
		if (c == '\\')
			fputs("\\\\", stdout);
		else if (c >= 0x20 && c < 0x7f)
			putchar(c);
		else
			printf("\\x%02x", c);
	}
}

static void put_str(const char *s)
{
	if (!s) {
		fputs("(NULL)", stdout);
		return;
	}
	putchar('"');
	put_escaped(s, strlen(s));
	putchar('"');
}

static void show_rc(const char *what, int rc)
{
	const git_error *e = git_error_last();
	printf("%s rc=%d", what, rc);
	if (rc < 0 && e) {
		printf(" class=%d msg=", e->klass);
		put_str(e->message);
	}
	printf("\n");
	git_error_clear();
}

static void show_values(const char *value)
{
	int b;
	int32_t i32;
	int64_t i64;
	int rc;

	rc = git_config_parse_bool(&b, value);
	printf("    bool rc=%d", rc);
	if (!rc)
		printf(" v=%d", b);
	else
		printf(" msg=%s", git_error_last() ? git_error_last()->message : "-");
	git_error_clear();
	rc = git_config_parse_int32(&i32, value);
	printf(" i32 rc=%d", rc);
	if (!rc)
		printf(" v=%d", (int)i32);
	git_error_clear();
	rc = git_config_parse_int64(&i64, value);
	printf(" i64 rc=%d", rc);
	if (!rc)
		printf(" v=%lld", (long long)i64);
	git_error_clear();
	printf("\n");
}

/* Whether the last show_entries saw a value-less entry. */
static int has_null;

/* Print every entry of `cfg`; the first entry's name goes to `first`. */
static void show_entries(git_config *cfg, char *first, size_t first_len, int values)
{
	has_null = 0;
	git_config_iterator *it;
	git_config_entry *e;
	int rc, n = 0;

	if (first)
		first[0] = '\0';
	if ((rc = git_config_iterator_new(&it, cfg)) < 0) {
		show_rc("  iterator", rc);
		return;
	}
	while ((rc = git_config_next(&e, it)) == 0) {
		printf("  [%d] ", n++);
		put_str(e->name);
		printf(" = ");
		put_str(e->value);
		printf(" level=%d depth=%u type=", (int)e->level, e->include_depth);
		put_str(e->backend_type);
		printf(" origin=");
		put_str(e->origin_path);
		printf("\n");
		if (values)
			show_values(e->value);
		if (!e->value)
			has_null = 1;
		if (first && !first[0] && strlen(e->name) < first_len)
			strcpy(first, e->name);
	}
	if (rc != GIT_ITEROVER)
		show_rc("  next", rc);
	git_error_clear();
	git_config_iterator_free(it);
}

static void memory(const char *data, size_t len)
{
	git_config *cfg;
	git_config_backend *b;
	git_config_backend_memory_options opts = GIT_CONFIG_BACKEND_MEMORY_OPTIONS_INIT;
	int rc;

	opts.backend_type = "mem";
	opts.origin_path = "origin";
	git_config_new(&cfg);
	if ((rc = git_config_backend_from_string(&b, data, len, &opts)) < 0) {
		show_rc(" mem new", rc);
		git_config_free(cfg);
		return;
	}
	if ((rc = git_config_add_backend(cfg, b, GIT_CONFIG_LEVEL_APP, NULL, 0)) < 0) {
		show_rc(" mem", rc);
		b->free(b);
		git_config_free(cfg);
		return;
	}
	printf(" mem ok\n");
	show_entries(cfg, NULL, 0, 1);
	git_config_free(cfg);
}

static void write_file(const char *path, const char *data, size_t len)
{
	FILE *f = fopen(path, "wb");
	if (!f) {
		perror(path);
		exit(1);
	}
	fwrite(data, 1, len, f);
	fclose(f);
}

static void show_file(const char *path)
{
	git_str buf = GIT_STR_INIT;
	if (git_futils_readbuffer(&buf, path) < 0) {
		printf("   file: (none)\n");
		git_error_clear();
		return;
	}
	printf("   file: ");
	put_escaped(buf.ptr, buf.size);
	printf("\n");
	git_str_dispose(&buf);
}

static git_config *open_tmp(const char *data, size_t len)
{
	git_config *cfg;
	int rc;
	unlink(TMPFILE ".lock");
	write_file(TMPFILE, data, len);
	if ((rc = git_config_open_ondisk(&cfg, TMPFILE)) < 0) {
		show_rc("   open", rc);
		return NULL;
	}
	return cfg;
}

static void writes(const char *data, size_t len, const char *first, int nulls)
{
	static const char *subjects[] = { "core.difftest", "Sec.Sub.Key", "a.b", NULL };
	const char *names[8];
	size_t n = 0, i;
	git_config *cfg;
	int rc;

	if (first && *first)
		names[n++] = first;
	for (i = 0; subjects[i]; i++)
		names[n++] = subjects[i];

	for (i = 0; i < n; i++) {
		const char *name = names[i];
		int op;
		for (op = 0; op < 5; op++) {
			/*
			 * The C crashes matching a pattern against a value-less
			 * variable (DEFECTS.md): no pattern writes then.
			 */
			if (nulls && (op == 2 || op == 4))
				continue;
			if (!(cfg = open_tmp(data, len)))
				return;
			printf("  op%d ", op);
			put_str(name);
			switch (op) {
			case 0: rc = git_config_set_string(cfg, name, "a ;b\"c\\d\te"); break;
			case 1: rc = git_config_set_string(cfg, name, " x y "); break;
			case 2: rc = git_config_set_multivar(cfg, name, ".*", "mv"); break;
			case 3: rc = git_config_delete_entry(cfg, name); break;
			default: rc = git_config_delete_multivar(cfg, name, "^[a-m]"); break;
			}
			show_rc("", rc);
			show_file(TMPFILE);
			if (op == 0) {
				/* What was written reads back. */
				git_config_entry *e;
				if ((rc = git_config_get_entry(&e, cfg, name)) == 0) {
					printf("   reads ");
					put_str(e->value);
					printf("\n");
					git_config_entry_free(e);
				} else {
					show_rc("   reads", rc);
				}
			}
			git_config_free(cfg);
		}
	}
}

static void file(const char *path, const char *data, size_t len, int do_writes)
{
	git_config *cfg;
	char first[256];
	int rc;

	if (path) {
		/* In place: includes resolve against its own directory. */
		if ((rc = git_config_open_ondisk(&cfg, path)) < 0) {
			show_rc(" file", rc);
			return;
		}
	} else if (!(cfg = open_tmp(data, len))) {
		return;
	}
	printf(" file ok\n");
	show_entries(cfg, first, sizeof(first), 0);
	git_config_free(cfg);
	if (do_writes)
		writes(data, len, first, has_null);
}

static const char syntax[] = "[]\"\\;#= \t\n\r.-\0\x80\xff";

static char *mutate(const char *data, size_t len, size_t *out_len)
{
	size_t cap = len + 16, n = len, k, edits = 1 + rnd() % 4;
	char *m = malloc(cap);
	memcpy(m, data, len);
	for (k = 0; k < edits; k++) {
		unsigned what = rnd() % 3;
		size_t pos = n ? rnd() % (n + 1) : 0;
		char c = (rnd() % 3) ? syntax[rnd() % (sizeof(syntax) - 1)] : (char)(rnd() & 0xff);
		if (what == 0 && pos < n) {
			m[pos] = c;
		} else if (what == 1 && pos < n) {
			memmove(m + pos, m + pos + 1, n - pos - 1);
			n--;
		} else if (n + 1 < cap) {
			memmove(m + pos + 1, m + pos, n - pos);
			m[pos] = c;
			n++;
		}
	}
	*out_len = n;
	return m;
}

static void one(const char *name, const char *path)
{
	git_str buf = GIT_STR_INIT;
	int k;

	if (git_futils_readbuffer(&buf, path) < 0) {
		git_error_clear();
		return;
	}
	fflush(stdout);
	printf("== %s\n", name);
	memory(buf.ptr, buf.size);
	file(path, buf.ptr, buf.size, 0);
	file(NULL, buf.ptr, buf.size, 1);

	for (k = 0; k < 24; k++) {
		size_t mlen;
		char *m = mutate(buf.ptr, buf.size, &mlen);
		printf("== %s ~%d\n", name, k);
		memory(m, mlen);
		file(NULL, m, mlen, k % 6 == 0);
		free(m);
	}
	git_str_dispose(&buf);
}

static void random_values(void)
{
	static const char alpha[] = "0123456789-+kKmMgGxXtrueoyfalsn \t";
	char s[24];
	int i, j;
	for (i = 0; i < 20000; i++) {
		int len = rnd() % 12;
		for (j = 0; j < len; j++)
			s[j] = alpha[rnd() % (sizeof(alpha) - 1)];
		s[len] = '\0';
		printf("v ");
		put_str(s);
		show_values(s);
	}
	show_values(NULL);
}

static void random_names(void)
{
	static const char alpha[] = "aZ09-._\n \"#x";
	git_config *cfg;
	git_config_backend *b;
	char s[16];
	int i, j, rc;

	git_config_new(&cfg);
	git_config_backend_from_string(&b, "[a]\nb=1\n[a \"B\"]\nc=2\n", 22, NULL);
	git_config_add_backend(cfg, b, GIT_CONFIG_LEVEL_APP, NULL, 0);
	for (i = 0; i < 20000; i++) {
		git_config_entry *e = NULL;
		int len = rnd() % 8;
		for (j = 0; j < len; j++)
			s[j] = alpha[rnd() % (sizeof(alpha) - 1)];
		s[len] = '\0';
		rc = git_config_get_entry(&e, cfg, s);
		printf("n ");
		put_str(s);
		show_rc("", rc);
		git_config_entry_free(e);
	}
	git_config_free(cfg);
}

int main(void)
{
	FILE *list;
	char line[4096];

	git_libgit2_init();
	mkdir(TMPDIR, 0777);

	list = popen("find tests/resources fuzzers/corpora/config_file -type f "
		"\\( -path '*/resources/config/*' -o -name config -o -name '.gitmodules' "
		"-o -name gitmodules -o -name '*.config' -o -name 'config.worktree' "
		"-o -name '.gitconfig' -o -path '*/corpora/config_file/*' \\) | LC_ALL=C sort", "r");
	while (fgets(line, sizeof(line), list)) {
		line[strcspn(line, "\n")] = '\0';
		/* 40000 entries of 480 KB names: too much output. */
		if (strstr(line, "config-oom"))
			continue;
		one(line, line);
	}
	pclose(list);

	random_values();
	random_names();

	git_libgit2_shutdown();
	return 0;
}
