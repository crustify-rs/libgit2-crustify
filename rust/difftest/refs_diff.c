/*
 * Differential test of refs.c, refdb_fs.c and reflog.c: every line printed
 * must be the same when linked against the C reference and against the
 * Rust port.
 *
 * - Reference name validation and normalization on a corpus and on random
 *   names over the bytes that matter, with every flag combination.
 * - packed-refs files (sorted and unsorted, with peel lines, comments, CRs,
 *   truncations and random corruption): lookups and full iterations.
 * - Loose reference files with random contents.
 * - Reflog files with random lines: what is read, and what is written back.
 *
 * The repository is a scratch one (./refs_diff.tmp, removed at the end).
 *
 * difftest-link: libgit2_tests
 * Build and compare: RUST_BUILD=/work/builds/l-refs rust/difftest/run.sh refs_diff.c
 */

#include "common.h"
#include "git2.h"
#include "git2/sys/refs.h"
#include "refs.h"
#include "futils.h"
#include <stdio.h>
#include <string.h>

static unsigned long seed = 777;
static unsigned rnd(void)
{
	seed = seed * 6364136223846793005UL + 1442695040888963407UL;
	return (unsigned)(seed >> 33);
}

static void pbytes(const char *p, size_t n)
{
	size_t i;
	putchar('[');
	for (i = 0; i < n; i++) {
		unsigned char c = (unsigned char)p[i];
		if (c >= 32 && c < 127 && c != '[' && c != ']' && c != '\\')
			putchar(c);
		else
			printf("\\x%02x", c);
	}
	putchar(']');
}

static void ps(const char *p)
{
	if (!p)
		printf("(null)");
	else
		pbytes(p, strlen(p));
}

static void perr(int error)
{
	const git_error *e = git_error_last();
	printf(" -> %d", error);
	if (error != 0) {
		printf(" class=%d msg=", e ? e->klass : -1);
		ps(e ? e->message : NULL);
	}
	putchar('\n');
	git_error_clear();
}

static void names_one(const char *name, unsigned flags)
{
	char out[64];
	git_str buf = GIT_STR_INIT;
	int valid = -1, error;

	printf("norm flags=%u ", flags);
	ps(name);
	error = git_reference_normalize_name(out, sizeof(out), name, flags);
	if (!error) {
		printf(" = ");
		ps(out);
	}
	perr(error);

	error = git_reference__normalize_name(&buf, name, flags);
	printf("  str %d size=%zu ", error, buf.size);
	if (!error)
		pbytes(buf.ptr, buf.size);
	putchar('\n');
	git_str_dispose(&buf);
	git_error_clear();

	error = git_reference__name_is_valid(&valid, name, flags);
	printf("  valid %d %d\n", error, valid);
	git_error_clear();
}

static void names(void)
{
	static const char *corpus[] = {
		"", "/", "a", "HEAD", "@", "^", "^HEAD", "refs", "refs/", "refs/heads/master",
		"refs//heads///master", "/refs/heads/x", "refs/heads/x/", "refs/heads/x.",
		"refs/heads/.x", "refs/heads/x..y", "refs/heads/x.lock", "refs/heads/.lock",
		"refs/heads/a@{b", "refs/heads/@", "@/x", "refs/heads/*", "refs/*/x", "*",
		"refs/heads/a*b*", "FETCH_HEAD", "_HEAD", "HEAD_", "MERGE_HEAD", "lower",
		"refs/heads/\x7f", "refs/heads/\x80\xff", "refs/heads/ x", "refs/heads/x~1",
		"refs/heads/x^", "refs/heads/x:y", "refs/heads/x?", "refs/heads/x[", "refs/heads/x\\y",
		"HEAD/foo", "^refs/heads/x", "refs/^heads", "x/y", "X/y", "a/.lock/b",
		"refs/heads/averyveryveryveryverylongnamethatwillnotfitintheshortbuffer",
		NULL
	};
	static const char alphabet[] = "aAZ_/.@{*^~:?[\\ \x7f\x80\xfflockHEAD";
	unsigned flags;
	size_t i;

	for (i = 0; corpus[i]; i++)
		for (flags = 0; flags < 8; flags++)
			names_one(corpus[i], flags);

	for (i = 0; i < 20000; i++) {
		char name[24];
		size_t n = rnd() % 20, j;
		for (j = 0; j < n; j++)
			name[j] = alphabet[rnd() % (sizeof(alphabet) - 1)];
		name[n] = '\0';
		names_one(name, rnd() % 8);
	}
}

#define A "1111111111111111111111111111111111111111"
#define B "2222222222222222222222222222222222222222"
#define C "a65fedf39aefe402d3bb6e24df4d4f5fe4547750"

static void write_file(const char *path, const char *data, size_t len)
{
	git_str p = GIT_STR_INIT;
	git_str_joinpath(&p, "refs_diff.tmp/.git", path);
	git_futils_mkpath2file(p.ptr, 0777);
	git_futils_writebuffer(&(git_str){ (char *)data, 0, len }, p.ptr, 0, 0666);
	git_str_dispose(&p);
}

static void remove_file(const char *path)
{
	git_str p = GIT_STR_INIT;
	git_str_joinpath(&p, "refs_diff.tmp/.git", path);
	p_unlink(p.ptr);
	git_str_dispose(&p);
}

static void dump_ref(git_reference *ref)
{
	ps(git_reference_name(ref));
	if (git_reference_type(ref) == GIT_REFERENCE_DIRECT) {
		printf(" %s", git_oid_tostr_s(git_reference_target(ref)));
		if (git_reference_target_peel(ref))
			printf(" ^%s", git_oid_tostr_s(git_reference_target_peel(ref)));
	} else {
		printf(" -> ");
		ps(git_reference_symbolic_target(ref));
	}
}

static void lookups(const char *what)
{
	static const char *names[] = {
		"refs/heads/a", "refs/heads/b", "refs/heads/b/c", "refs/tags/t", "refs/tags/u",
		"refs/heads/missing", "refs/heads/aa", "refs/z", NULL
	};
	git_repository *repo;
	git_reference_iterator *it;
	git_reference *ref;
	const char *name;
	size_t i;
	int error;

	printf("== %s\n", what);
	/* A fresh repository each time: the backend caches the file. */
	if ((error = git_repository_open(&repo, "refs_diff.tmp")) < 0) {
		perr(error);
		return;
	}
	for (i = 0; names[i]; i++) {
		printf("lookup %s", names[i]);
		error = git_reference_lookup(&ref, repo, names[i]);
		if (!error) {
			printf(" = ");
			dump_ref(ref);
			git_reference_free(ref);
		}
		perr(error);
	}
	if ((error = git_reference_iterator_new(&it, repo)) < 0) {
		printf("iterator");
		perr(error);
	} else {
		while ((error = git_reference_next(&ref, it)) == 0) {
			printf("  it ");
			dump_ref(ref);
			putchar('\n');
			git_reference_free(ref);
		}
		printf("  end");
		perr(error == GIT_ITEROVER ? 0 : error);
		git_reference_iterator_free(it);
	}
	if ((error = git_reference_iterator_glob_new(&it, repo, "refs/tags/*")) == 0) {
		while ((error = git_reference_next_name(&name, it)) == 0) {
			printf("  glob ");
			ps(name);
			putchar('\n');
		}
		git_reference_iterator_free(it);
	}
	git_error_clear();
	git_repository_free(repo);
}

static void packed(void)
{
	static const char *files[] = {
		"# pack-refs with: peeled fully-peeled sorted \n"
		A " refs/heads/a\n" A " refs/heads/b\n" B " refs/heads/b/c\n" A " refs/tags/t\n^" B "\n" B " refs/tags/u\n",
		"# pack-refs with: peeled \n"
		B " refs/tags/u\n" A " refs/heads/a\n" A " refs/tags/t\n^" B "\n",
		"# pack-refs with: peeled fully-peeled sorted \n# a comment\n"
		A " refs/heads/a\r\n" A " refs/heads/b\n",
		A " refs/heads/a\n" A " refs/heads/b",
		A " refs/heads/a\n^12\n",
		"# pack-refs with: sorted ",
		"1234 refs/heads/a\n",
		A "xrefs/heads/a\n",
		"",
		"# pack-refs with: peeled fully-peeled sorted \n" A " refs/heads/a\n" A " refs/heads/a\n",
		NULL
	};
	char buf[512];
	size_t i, k;

	for (i = 0; files[i]; i++) {
		write_file("packed-refs", files[i], strlen(files[i]));
		snprintf(buf, sizeof(buf), "packed %zu", i);
		lookups(buf);
	}
	/* Random corruptions of the first file. */
	for (k = 0; k < 300; k++) {
		size_t len = strlen(files[0]), n = rnd() % 4, j;
		memcpy(buf, files[0], len);
		for (j = 0; j < n; j++) {
			static const char repl[] = "\n^ #\r\0a1/";
			buf[rnd() % len] = repl[rnd() % (sizeof(repl) - 1)];
		}
		if (rnd() % 3 == 0)
			len = rnd() % len;
		write_file("packed-refs", buf, len);
		snprintf(buf, sizeof(buf), "packed random %zu", k);
		lookups(buf);
	}
	remove_file("packed-refs");
}

static void loose(void)
{
	static const char *contents[] = {
		A "\n", A, A " trailing", A "x", "ref: refs/heads/b\n", "ref: ", "ref:  refs/heads/b  \n",
		"1111", "", "ref: refs/heads/a\0junk", "\n" A, NULL
	};
	git_repository *repo;
	git_reference *ref;
	size_t i;
	int error;

	if (git_repository_open(&repo, "refs_diff.tmp") < 0)
		return;
	for (i = 0; contents[i]; i++) {
		size_t len = strlen(contents[i]);
		if (i == 9)
			len = sizeof("ref: refs/heads/a\0junk") - 1;
		write_file("refs/heads/loose", contents[i], len);
		printf("loose %zu", i);
		error = git_reference_lookup(&ref, repo, "refs/heads/loose");
		if (!error) {
			printf(" = ");
			dump_ref(ref);
			git_reference_free(ref);
		}
		perr(error);
	}
	remove_file("refs/heads/loose");
	git_repository_free(repo);
}

static void dump_reflog(git_repository *repo, const char *name)
{
	git_reflog *log;
	size_t i, n;
	int error;

	if ((error = git_reflog_read(&log, repo, name)) < 0) {
		printf("reflog read");
		perr(error);
		return;
	}
	n = git_reflog_entrycount(log);
	printf("reflog %zu entries\n", n);
	for (i = 0; i < n; i++) {
		const git_reflog_entry *e = git_reflog_entry_byindex(log, i);
		const git_signature *s = git_reflog_entry_committer(e);
		printf("  %s", git_oid_tostr_s(git_reflog_entry_id_old(e)));
		printf(" %s ", git_oid_tostr_s(git_reflog_entry_id_new(e)));
		ps(s->name);
		ps(s->email);
		printf(" %lld %d %c ", (long long)s->when.time, s->when.offset, s->when.sign);
		ps(git_reflog_entry_message(e));
		putchar('\n');
	}
	git_reflog_free(log);
}

static void reflogs(void)
{
	static const char *lines[] = {
		A " " B " A U Thor <a@example.com> 1234567890 +0100\tmessage\n",
		A " " B " A U Thor <a@example.com> 1234567890 -0130\n",
		A " " B " A U Thor <a@example.com> 1234567890 +0000\t\n",
		A " " B " No Email 1234567890 +0000\tx\n",
		A B " missing space <a@b> 1 +0000\tx\n",
		A " 22 short <a@b> 1 +0000\tx\n",
		"garbage\n",
		"\n",
		A " " B " A <a@b> 1 +0000\ttabs\tinside\n",
		A " " B " A <a@b> 1 +0000\tno newline",
		NULL
	};
	git_repository *repo;
	git_reflog *log;
	git_signature *sig;
	git_oid id;
	git_str file = GIT_STR_INIT, content = GIT_STR_INIT;
	size_t i, k;
	int error;

	if (git_repository_open(&repo, "refs_diff.tmp") < 0)
		return;
	for (k = 0; k < 200; k++) {
		size_t n = rnd() % 5, j;
		git_str_clear(&content);
		for (j = 0; j < n; j++)
			git_str_puts(&content, lines[rnd() % 10]);
		write_file("logs/refs/heads/x", content.ptr ? content.ptr : "", content.size);
		printf("== reflog %zu\n", k);
		dump_reflog(repo, "refs/heads/x");
	}
	git_oid_from_string(&id, C, GIT_OID_SHA1);
	git_signature_new(&sig, "Wr Iter", "w@example.com", 1500000000, -300);
	for (i = 0; i < 3; i++) {
		static const char *msgs[] = { "plain", "multi\nline\nmessage\n", NULL };
		if ((error = git_reflog_read(&log, repo, "refs/heads/x")) < 0) {
			perr(error);
			continue;
		}
		error = git_reflog_append(log, &id, sig, msgs[i]);
		printf("append");
		perr(error);
		if (i == 1) {
			printf("drop");
			perr(git_reflog_drop(log, 0, 1));
			printf("drop missing");
			perr(git_reflog_drop(log, 99, 1));
		}
		printf("write");
		perr(git_reflog_write(log));
		git_reflog_free(log);
		git_str_joinpath(&file, "refs_diff.tmp/.git", "logs/refs/heads/x");
		git_futils_readbuffer(&content, file.ptr);
		printf("file ");
		pbytes(content.ptr, content.size);
		putchar('\n');
	}
	git_signature_free(sig);
	git_str_dispose(&file);
	git_str_dispose(&content);
	git_repository_free(repo);
}

static int stop_at_3(const char *name, void *payload)
{
	int *n = payload;
	(void)name;
	return ++*n == 3 ? 42 : 0;
}

static void foreach_stop(void)
{
	git_repository *repo;
	int n = 0, error;

	if (git_repository_open(&repo, "tests/resources/testrepo.git") < 0)
		return;
	git_error_clear();
	error = git_reference_foreach_name(repo, stop_at_3, &n);
	printf("foreach_name n=%d", n);
	perr(error);
	n = 0;
	error = git_reference_foreach_glob(repo, "refs/heads/*", stop_at_3, &n);
	printf("foreach_glob n=%d", n);
	perr(error);
	git_repository_free(repo);
}

int main(void)
{
	git_repository *repo;

	git_libgit2_init();
	names();
	foreach_stop();

	git_futils_rmdir_r("refs_diff.tmp", NULL, GIT_RMDIR_REMOVE_FILES);
	if (git_repository_init(&repo, "refs_diff.tmp", 0) < 0) {
		printf("init failed\n");
		return 1;
	}
	git_repository_free(repo);
	packed();
	loose();
	reflogs();
	git_futils_rmdir_r("refs_diff.tmp", NULL, GIT_RMDIR_REMOVE_FILES);
	git_libgit2_shutdown();
	return 0;
}
