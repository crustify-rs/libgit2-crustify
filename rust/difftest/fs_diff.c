/*
 * Differential test of fs_path.c, futils.c, posix.c and filebuf.c: every
 * line printed must be the same when linked against the C reference and
 * against the Rust port.
 *
 * - The pure path functions run on a fixed corpus and on ~200k random
 *   strings over an alphabet of the bytes that matter ('/', '.', ':', '\\',
 *   ' ', '%', hex digits, letters, non-ASCII bytes), with buffers in every
 *   state and arguments that alias the output buffer.
 * - The filesystem functions run in a scratch directory (./fs_diff.tmp),
 *   whose paths are printed relative to it.
 *
 * Build and compare: RUST_BUILD=/work/builds/u3 rust/difftest/run.sh fs_diff.c
 */

#include "git2_util.h"
#include "git2/global.h"
#include "git2/errors.h"
#include "fs_path.h"
#include "futils.h"
#include "filebuf.h"
#include "posix.h"
#include "vector.h"
#include <sys/stat.h>
#include <unistd.h>

static unsigned long seed = 4242;
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

static void pstr(const git_str *s)
{
	printf(" size=%zu asize=%zu oom=%d ", s->size, s->asize, git_str_oom(s));
	if (!git_str_oom(s) && s->ptr)
		pbytes(s->ptr, s->size);
}

static void perr(int rc)
{
	const git_error *e = git_error_last();
	printf(" rc=%d", rc);
	if (rc < 0 || (e && e->klass))
		printf(" class=%d msg=%s", e->klass, e->message);
	git_error_clear();
}

static const char alphabet[] = "//..::\\ %2Fabca/C\x80\xc3";

static void random_path(char *buf, size_t max)
{
	size_t n = rnd() % max, i;
	for (i = 0; i < n; i++)
		buf[i] = alphabet[rnd() % (sizeof(alphabet) - 1)];
	buf[n] = '\0';
}

static const char *corpus[] = {
	"", "/", "//", "///", ".", "..", "./", "../", "a", "a/", "a//", "/a", "//a", "/a/", "a/b",
	"a/b/", "a//b", "a//b//", "/a/b/c", "a/b/c/", "./a", "../a", "a/./b", "a/../b",
	"a/b/../../c", "../../a", "/..", "/../a", "C:", "C:/", "C:/a", "C:a", "c:/a/b",
	"\xc3\xa4:/x", "\x80\x80\x80\x80:", "\x80:", "file:///tmp/a", "file://localhost/x/y",
	"file://", "file:///", "file://localhost/", "file://other/x", "http://a/b/../c",
	"git://host/../x", "a%20b", "%", "%2", "%2F", "%zz", "%41%42%4", "a/./.", ".a", "a.",
	"a ", "a:", "CON", "con.txt", "aux", "AUX:", "COM1", "com0", "LPT9.x", "nul ", "PRN",
	"a/b\\c", "a\\\\b", "dir/.git/x", "/tmp/../tmp/./x", "a/b/c/d/e/f/g/h", NULL
};

static void test_names(const char *p)
{
	git_str b = GIT_STR_INIT;
	int r;
	char *d;

	printf("path "); ps(p); printf("\n");

	r = git_fs_path_dirname_r(&b, p); printf(" dirname_r"); perr(r); pstr(&b); printf("\n");
	r = git_fs_path_dirname_r(NULL, p); printf(" dirname_r(NULL)"); perr(r); printf("\n");
	r = git_fs_path_basename_r(&b, p); printf(" basename_r"); perr(r); pstr(&b); printf("\n");
	r = git_fs_path_basename_r(NULL, p); printf(" basename_r(NULL)"); perr(r); printf("\n");
	d = git_fs_path_dirname(p); printf(" dirname "); ps(d); printf("\n"); git__free(d);
	d = git_fs_path_basename(p); printf(" basename "); ps(d); printf("\n"); git__free(d);
	printf(" root=%d dirlen=%zu nonascii=%d local=%d\n", git_fs_path_root(p),
		git_fs_path_dirlen(p), git_fs_path_has_non_ascii(p, strlen(p)),
		git_fs_path_is_local_file_url(p));

	/* aliasing: the argument is the buffer itself */
	git_str_sets(&b, p);
	r = git_fs_path_dirname_r(&b, b.ptr); printf(" dirname_r(self)"); perr(r); pstr(&b); printf("\n");
	git_str_sets(&b, p);
	r = git_fs_path_basename_r(&b, b.ptr); printf(" basename_r(self)"); perr(r); pstr(&b); printf("\n");
	git_str_sets(&b, p);
	printf(" basename_offset=%zu", git_fs_path_basename_offset(&b));
	r = git_fs_path_to_dir(&b); printf(" to_dir"); perr(r); pstr(&b); printf("\n");

	git_str_sets(&b, p);
	git_fs_path_squash_slashes(&b); printf(" squash"); pstr(&b); printf("\n");

	{
		size_t ceil;
		for (ceil = 0; ceil < 4; ceil++) {
			git_str_sets(&b, p);
			r = git_fs_path_resolve_relative(&b, ceil);
			printf(" resolve(%zu)", ceil); perr(r); pstr(&b); printf("\n");
		}
	}

	git_str_sets(&b, "base/dir");
	r = git_fs_path_apply_relative(&b, p); printf(" apply"); perr(r); pstr(&b); printf("\n");
	git_str_sets(&b, "/base/dir/");
	r = git_fs_path_apply_relative(&b, p); printf(" apply_abs"); perr(r); pstr(&b); printf("\n");

	r = git__percent_decode(&b, p); printf(" pdecode"); perr(r); pstr(&b); printf("\n");
	r = git_fs_path_fromurl(&b, p); printf(" fromurl"); perr(r); pstr(&b); printf("\n");
	git_str_clear(&b);
	r = git_fs_path_from_url_or_path(&b, p); printf(" url_or_path"); perr(r); pstr(&b); printf("\n");

	git_str_sets(&b, "pre\\");
	r = git_fs_path_normalize_slashes(&b, p); printf(" normalize"); perr(r); pstr(&b); printf("\n");

	{
		ssize_t root_at = -7;
		r = git_fs_path_join_unrooted(&b, p, NULL, &root_at);
		printf(" join_unrooted(NULL)"); perr(r); pstr(&b); printf(" root_at=%zd\n", root_at);
		root_at = -7;
		r = git_fs_path_join_unrooted(&b, p, "/base", &root_at);
		printf(" join_unrooted(/base)"); perr(r); pstr(&b); printf(" root_at=%zd\n", root_at);
		root_at = -7;
		r = git_fs_path_join_unrooted(&b, p, "/a/", &root_at);
		printf(" join_unrooted(/a/)"); perr(r); pstr(&b); printf(" root_at=%zd\n", root_at);
	}

	{
		char sbuf[32];
		size_t sz;
		for (sz = 0; sz < 4 && strlen(p) + 2 <= sizeof(sbuf); sz++) {
			strcpy(sbuf, p);
			git_fs_path_string_to_dir(sbuf, strlen(p) + sz);
			printf(" string_to_dir(+%zu) ", sz); ps(sbuf); printf("\n");
		}
	}

	git_str_dispose(&b);
}

static bool char_cb(char c, void *payload)
{
	(*(int *)payload)++;
	return c != 'x';
}

static bool comp_cb(const char *c, size_t len, void *payload)
{
	(*(int *)payload) += 100;
	return !(len == 3 && !memcmp(c, "bad", 3));
}

static void test_valid(const char *p)
{
	unsigned int flags;
	int count;
	printf("valid "); ps(p);
	for (flags = 0; flags < (1u << 10); flags = flags * 2 + 1)
		printf(" %d", git_fs_path_is_valid(p, flags));
	for (flags = 1; flags < (1u << 10); flags <<= 1)
		printf(" %d", git_fs_path_is_valid(p, flags));
	{
		git_str s = GIT_STR_INIT_CONST(p, strlen(p));
		count = 0;
		printf(" ext=%d", git_fs_path_str_is_valid_ext(&s, 0x3ff, char_cb, comp_cb, NULL, &count));
		printf(" count=%d", count);
		count = 0;
		printf(" ext3=%d", git_fs_path_str_is_valid_ext(&s, GIT_FS_PATH_REJECT_TRAVERSAL, char_cb, comp_cb, NULL, &count));
		printf(" count=%d", count);
		if (s.size > 1) {
			s.size--;
			printf(" short=%d", git_fs_path_str_is_valid(&s, GIT_FS_PATH_REJECT_EMPTY_COMPONENT));
		}
	}
	printf("\n");
}

static void test_pair(const char *a, const char *b)
{
	git_str s = GIT_STR_INIT;
	int r;

	printf("pair "); ps(a); ps(b);
	printf(" common=%zu", git_fs_path_common_dirlen(a, b));
	printf(" cmp=%d/%d/%d/%d",
		git_fs_path_cmp(a, strlen(a), 0, b, strlen(b), 0, strncmp),
		git_fs_path_cmp(a, strlen(a), 1, b, strlen(b), 0, strncmp),
		git_fs_path_cmp(a, strlen(a), 0, b, strlen(b), 1, git__strncasecmp),
		git_fs_path_cmp(a, strlen(a), 1, b, strlen(b), 1, strncmp));
	git_str_sets(&s, a);
	r = git_fs_path_make_relative(&s, b); printf(" make_relative"); perr(r); pstr(&s);
	{
		ssize_t root_at = -7;
		r = git_fs_path_join_unrooted(&s, a, b, &root_at);
		printf(" join"); perr(r); pstr(&s); printf(" root_at=%zd", root_at);
	}
	printf("\n");
	git_str_dispose(&s);
}

static int walk_cb(void *payload, const char *path)
{
	int *n = payload;
	printf(" ["); ps(path); printf("]");
	return ++(*n) == 7 ? 42 : 0;
}

static void test_walk(const char *p, const char *ceiling)
{
	git_str s = GIT_STR_INIT;
	int n = 0, r;
	git_str_sets(&s, p);
	printf("walk_up "); ps(p); printf(" "); ps(ceiling);
	r = git_fs_path_walk_up(&s, ceiling, walk_cb, &n);
	printf(" ->"); perr(r); pstr(&s); printf("\n");
	git_str_dispose(&s);
}

static void test_set_error(void)
{
	int errs[] = { ENOENT, ENOTDIR, EINVAL, ENAMETOOLONG, EEXIST, EACCES, EPERM, EBUSY, 0 };
	size_t i;
	for (i = 0; i < sizeof(errs) / sizeof(errs[0]); i++) {
		int r;
		errno = errs[i];
		r = git_fs_path_set_error(errs[i], "some/path", "act");
		printf("set_error %zu", i); perr(r); printf(" errno=%d\n", errno);
	}
}

static void test_modes(void)
{
	unsigned m;
	for (m = 0; m < 0200000; m += 0777 / 3 + 1)
		printf("%o:%o ", m, git_futils_canonical_mode(m));
	printf("\n");
}

/* --- the filesystem --- */

static int list_cb(void *payload, git_str *path)
{
	git_vector *v = payload;
	return git_vector_insert(v, git__strdup(path->ptr));
}

static void print_sorted(const char *what, git_vector *v)
{
	size_t i;
	char *s;
	git_vector_sort(v);
	printf("%s %zu:", what, v->length);
	git_vector_foreach(v, i, s) { printf(" "); ps(s); git__free(s); }
	printf("\n");
	git_vector_clear(v);
}

static void touchfile(const char *p, const char *content)
{
	git_futils_mkpath2file(p, 0755);
	git_str b = GIT_STR_INIT_CONST(content, strlen(content));
	int r = git_futils_writebuffer(&b, p, 0, 0644);
	printf("write "); ps(p); perr(r); printf("\n");
}

static void test_fs(void)
{
	git_str b = GIT_STR_INIT;
	git_vector v = GIT_VECTOR_INIT;
	struct stat st;
	int r, fd;

	v._cmp = git__strcmp_cb;

	r = git_futils_mkdir("d1/d2/d3", 0755, GIT_MKDIR_PATH); printf("mkdir -p"); perr(r); printf("\n");
	r = git_futils_mkdir("d1/d2/d3", 0755, GIT_MKDIR_PATH | GIT_MKDIR_EXCL); printf("mkdir excl"); perr(r); printf("\n");
	r = git_futils_mkdir("x1/x2", 0755, 0); printf("mkdir nopath"); perr(r); printf("\n");
	r = git_futils_mkdir("x1/x2/", 0755, GIT_MKDIR_SKIP_LAST); printf("mkdir skip_last"); perr(r); printf(" isdir=%d\n", git_fs_path_isdir("x1"));
	r = git_futils_mkdir("y1/y2/y3", 0700, GIT_MKDIR_PATH | GIT_MKDIR_SKIP_LAST2 | GIT_MKDIR_CHMOD_PATH); printf("mkdir skip_last2"); perr(r);
	printf(" y1=%d y1/y2=%d\n", git_fs_path_isdir("y1"), git_fs_path_isdir("y1/y2"));
	r = git_futils_mkdir("", 0755, GIT_MKDIR_PATH); printf("mkdir empty"); perr(r); printf("\n");
	touchfile("d1/file", "hello\n");
	r = git_futils_mkdir("d1/file/sub", 0755, GIT_MKDIR_PATH); printf("mkdir over file"); perr(r); printf("\n");
	r = git_futils_mkdir("d1/file", 0755, GIT_MKDIR_PATH | GIT_MKDIR_VERIFY_DIR); printf("mkdir verify file"); perr(r); printf("\n");
	r = git_futils_mkdir("d1/file", 0755, GIT_MKDIR_PATH | GIT_MKDIR_REMOVE_FILES); printf("mkdir remove_files"); perr(r); printf(" isdir=%d\n", git_fs_path_isdir("d1/file"));
	r = git_futils_mkdir_relative("r1/r2", "d1", 0755, GIT_MKDIR_PATH, NULL); printf("mkdir_relative"); perr(r); printf(" %d\n", git_fs_path_isdir("d1/r1/r2"));
	{
		struct git_futils_mkdir_options opts = {0};
		r = git_futils_mkdir_relative("p1/p2/p3", "d1", 0755, GIT_MKDIR_PATH | GIT_MKDIR_CHMOD, &opts);
		printf("mkdir_relative perf"); perr(r); printf(" stat=%zu mkdir=%zu chmod=%zu\n",
			opts.perfdata.stat_calls, opts.perfdata.mkdir_calls, opts.perfdata.chmod_calls);
	}
	r = git_futils_mkpath2file("m1/m2/file", 0755); printf("mkpath2file"); perr(r); printf(" %d\n", git_fs_path_isdir("m1/m2"));

	touchfile("d1/d2/a", "a");
	touchfile("d1/d2/.hidden", "h");
	touchfile("d1/d2/d3/b", "bb");
	r = p_symlink("a", "d1/d2/link"); printf("symlink %d\n", r);

	r = git_str_sets(&b, "d1");
	r = git_fs_path_direach(&b, 0, list_cb, &v); printf("direach"); perr(r); pstr(&b); printf("\n");
	print_sorted("  entries", &v);
	r = git_fs_path_dirload(&v, "d1/d2", 3, 0); printf("dirload"); perr(r); printf("\n");
	print_sorted("  entries", &v);
	r = git_fs_path_dirload(&v, "nonexistent", 0, 0); printf("dirload missing"); perr(r); printf("\n");
	r = git_fs_path_dirload(&v, "d1/d2/", 0, GIT_FS_PATH_DIR_INCLUDE_DOT_AND_DOTDOT); printf("dirload dots"); perr(r); printf("\n");
	print_sorted("  entries", &v);

	{
		git_fs_path_diriter it = GIT_FS_PATH_DIRITER_INIT;
		const char *name, *full;
		size_t nlen, flen;
		r = git_fs_path_diriter_init(&it, "d1/d2///", 0); printf("diriter_init"); perr(r); printf("\n");
		while ((r = git_fs_path_diriter_next(&it)) == 0) {
			git_fs_path_diriter_filename(&name, &nlen, &it);
			git_fs_path_diriter_fullpath(&full, &flen, &it);
			git_vector_insert(&v, git__strndup(full, flen));
			r = git_fs_path_diriter_stat(&st, &it);
			if (r < 0) { printf("stat"); perr(r); printf("\n"); }
		}
		printf("diriter end"); perr(r); printf("\n");
		print_sorted("  entries", &v);
		git_fs_path_diriter_free(&it);
		/* (C dereferences NULL for an empty path: rust/DEFECTS.md) */
		r = git_fs_path_diriter_init(&it, "nope", 0); printf("diriter_init missing"); perr(r); printf("\n");
		git_fs_path_diriter_free(&it);
	}

	printf("preds exists=%d%d isdir=%d%d isfile=%d%d islink=%d%d exec=%d empty=%d%d\n",
		git_fs_path_exists("d1"), git_fs_path_exists("zz"),
		git_fs_path_isdir("d1"), git_fs_path_isdir("d1/d2/a"),
		git_fs_path_isfile("d1/d2/a"), git_fs_path_isfile("d1"),
		git_fs_path_islink("d1/d2/link"), git_fs_path_islink("d1/d2/a"),
		git_fs_path_isexecutable("d1/d2/a"),
		git_fs_path_is_empty_dir("d1/d2/d3"), git_fs_path_is_empty_dir("x1/x2"));
	git_str_sets(&b, "d1/d2");
	printf("contains %d %d %d %d", git_fs_path_contains(&b, "a"), git_fs_path_contains_dir(&b, "d3"),
		git_fs_path_contains_file(&b, "d3"), git_fs_path_contains_file(&b, "a"));
	pstr(&b); printf("\n");

	r = git_fs_path_lstat("d1/nope", &st); printf("lstat"); perr(r); printf("\n");

	r = git_futils_readbuffer(&b, "d1/d2/d3/b"); printf("readbuffer"); perr(r); pstr(&b); printf("\n");
	r = git_futils_readbuffer(&b, "d1/d2"); printf("readbuffer dir"); perr(r); printf("\n");
	r = git_futils_readbuffer(&b, "d1/none"); printf("readbuffer missing"); perr(r); printf("\n");
	{
		unsigned char sum[32] = {0};
		int updated = -1;
		r = git_futils_readbuffer_updated(&b, "d1/d2/d3/b", sum, &updated); printf("updated"); perr(r); printf(" %d", updated);
		r = git_futils_readbuffer_updated(&b, "d1/d2/d3/b", sum, &updated); printf(" again"); perr(r); printf(" %d\n", updated);
	}
	fd = git_futils_open_ro("d1/d2/d3/b");
	r = git_futils_readbuffer_fd(&b, fd, 5); printf("readbuffer_fd short"); perr(r); printf("\n");
	p_close(fd);
	fd = git_futils_open_ro("d1/none"); printf("open_ro missing"); perr(fd); printf("\n");

	{
		git_str big = GIT_STR_INIT;
		size_t i;
		for (i = 0; i < 30000; i++)
			git_str_putc(&big, (char)('a' + i % 23));
		r = git_futils_writebuffer(&big, "big", O_CREAT | O_TRUNC | O_WRONLY | O_FSYNC, 0600);
		printf("write big"); perr(r); printf("\n");
		fd = git_futils_open_ro("big");
		r = git_futils_readbuffer_fd(&b, fd, 30000); printf("read big"); perr(r);
		printf(" eq=%d\n", b.size == big.size && !memcmp(b.ptr, big.ptr, big.size));
		p_close(fd);
		git_str_dispose(&big);
	}

	r = git_futils_cp("d1/d2/a", "copied", 0644); printf("cp"); perr(r); printf("\n");
	r = git_futils_cp("d1/d2/a", "copied", 0644); printf("cp exists"); perr(r); printf("\n");
	r = git_futils_cp_r("d1", "tree", GIT_CPDIR_COPY_SYMLINKS | GIT_CPDIR_CREATE_EMPTY_DIRS, 0755); printf("cp_r"); perr(r); printf("\n");
	git_str_sets(&b, "tree");
	r = git_fs_path_direach(&b, 0, list_cb, &v); print_sorted("  tree", &v);
	git_str_sets(&b, "tree/d2");
	r = git_fs_path_direach(&b, 0, list_cb, &v); print_sorted("  tree/d2", &v);
	r = git_futils_cp_r("d1", "tree2", GIT_CPDIR_COPY_DOTFILES | GIT_CPDIR_CHMOD_DIRS | GIT_CPDIR_SIMPLE_TO_MODE, 0700); printf("cp_r2"); perr(r); printf("\n");
	git_str_sets(&b, "tree2/d2");
	r = git_fs_path_direach(&b, 0, list_cb, &v); print_sorted("  tree2/d2", &v);

	r = git_futils_mv_withpath("copied", "moved/deep/file", 0755); printf("mv"); perr(r); printf(" %d\n", git_fs_path_isfile("moved/deep/file"));
	r = git_futils_mv_withpath("nothere", "moved/x", 0755); printf("mv missing"); perr(r); printf("\n");
	r = git_futils_fake_symlink("target/path", "fake/link"); printf("fake_symlink"); perr(r); printf("\n");
	r = git_futils_readbuffer(&b, "fake/link"); printf("  "); pstr(&b); printf("\n");
	{
		time_t t = 1234567890;
		r = git_futils_touch("moved/deep/file", &t); printf("touch"); perr(r);
		p_stat("moved/deep/file", &st); printf(" mtime=%ld\n", (long)st.st_mtime);
		r = git_futils_touch("moved/none", &t); printf("touch missing"); perr(r); printf("\n");
	}
	r = git_futils_truncate("trunc", 0644); printf("truncate"); perr(r); printf("\n");
	r = git_futils_creat_locked("trunc", 0644); printf("creat_locked exists"); perr(r); printf("\n");
	r = git_futils_creat_locked("nodir/x", 0644); printf("creat_locked nodir"); perr(r); printf("\n");
	fd = git_futils_creat_locked_withpath("lk/a/b", 0755, 0644); printf("creat_locked_withpath %d\n", fd >= 0); if (fd >= 0) p_close(fd);
	fd = git_futils_creat_withpath("cw/a/b", 0755, 0644); printf("creat_withpath %d\n", fd >= 0); if (fd >= 0) p_close(fd);
	{
		uint64_t sz = 0;
		fd = git_futils_open_ro("big");
		r = git_futils_filesize(&sz, fd); printf("filesize"); perr(r); printf(" %llu\n", (unsigned long long)sz);
		p_close(fd);
		r = git_futils_filesize(&sz, -1); printf("filesize bad"); perr(r); printf("\n");
	}
	{
		git_futils_filestamp stamp, s2;
		memset(&stamp, 0, sizeof(stamp));
		printf("stamp %d", git_futils_filestamp_check(&stamp, "big"));
		printf(" %d", git_futils_filestamp_check(&stamp, "big"));
		printf(" %d", git_futils_filestamp_check(NULL, "big"));
		printf(" %d", git_futils_filestamp_check(&stamp, "none"));
		git_futils_filestamp_set(&s2, &stamp);
		printf(" %d", git_futils_filestamp_check(&s2, "big"));
		git_futils_filestamp_set(&s2, NULL);
		printf(" %d", git_futils_filestamp_check(&s2, "big"));
		p_stat("big", &st);
		git_futils_filestamp_set_from_stat(&s2, &st);
		printf(" %d\n", git_futils_filestamp_check(&s2, "big"));
	}
	{
		git_map map;
		r = git_futils_mmap_ro_file(&map, "big"); printf("mmap"); perr(r);
		if (!r) { printf(" len=%zu eq=%d", map.len, !memcmp(map.data, "abc", 3)); git_futils_mmap_free(&map); }
		printf("\n");
		r = git_futils_mmap_ro_file(&map, "trunc"); printf("mmap empty"); perr(r); printf("\n");
		r = git_futils_mmap_ro_file(&map, "none"); printf("mmap missing"); perr(r); printf("\n");
	}

	r = git_futils_fsync_dir("d1"); printf("fsync_dir"); perr(r); printf("\n");
	r = git_futils_fsync_dir("none"); printf("fsync_dir missing"); perr(r); printf("\n");
	r = git_futils_fsync_parent("d1/file"); printf("fsync_parent"); perr(r); printf("\n");

	git_str_clear(&b);
	r = git_fs_path_prettify(&b, "d1/../d1/d2", NULL); printf("prettify"); perr(r); printf(" rel=%d\n", r ? -1 : (int)git__suffixcmp(b.ptr, "fs_diff.tmp/d1/d2"));
	r = git_fs_path_prettify_dir(&b, "d2", "d1"); printf("prettify_dir"); perr(r); printf(" rel=%d\n", r ? -1 : (int)git__suffixcmp(b.ptr, "fs_diff.tmp/d1/d2/"));
	git_str_sets(&b, "d1/./d2");
	r = git_fs_path_prettify(&b, b.ptr, NULL); printf("prettify self"); perr(r); printf(" rel=%d\n", r ? -1 : (int)git__suffixcmp(b.ptr, "fs_diff.tmp/d1/d2"));
	r = git_fs_path_prettify(&b, "nope/x", NULL); printf("prettify missing"); perr(r); pstr(&b); printf("\n");
	r = git_fs_path_prettify(&b, "d1/d2/a/x", NULL); printf("prettify notdir"); perr(r); printf("\n");
	git_str_sets(&b, "d1/d2/a");
	r = git_fs_path_find_dir(&b); printf("find_dir"); perr(r); printf(" rel=%d\n", (int)git__suffixcmp(b.ptr, "fs_diff.tmp/d1/d2/"));
	git_str_sets(&b, "nope/x");
	r = git_fs_path_find_dir(&b); printf("find_dir missing"); perr(r); pstr(&b); printf("\n");

	{
		bool is = false;
		r = git_fs_path_owner_is_current_user(&is, "d1"); printf("owner"); perr(r); printf(" %d", is);
		r = git_fs_path_owner_is_system(&is, "d1"); printf(" sys"); perr(r); printf(" %d", is);
		r = git_fs_path_owner_is(&is, "none", GIT_FS_PATH_OWNER_CURRENT_USER); printf(" missing"); perr(r);
		git_fs_path__set_owner(GIT_FS_PATH_OWNER_ADMINISTRATOR);
		r = git_fs_path_owner_is_system(&is, "none"); printf(" mock"); perr(r); printf(" %d", is);
		r = git_fs_path_owner_is_current_user(&is, "none"); printf(" %d\n", is);
		git_fs_path__set_owner(GIT_FS_PATH_OWNER_NONE);
	}
	printf("symlinks %d decompose %d\n", git_fs_path_supports_symlinks("."), git_fs_path_does_decompose_unicode("."));
	r = git_fs_path_find_executable(&b, "sh"); printf("find sh"); perr(r); printf("\n");
	r = git_fs_path_find_executable(&b, "no-such-exe-xyz"); printf("find none"); perr(r); pstr(&b); printf("\n");
	r = git_fs_path_find_executable(&b, "d1/d2/a"); printf("find path"); perr(r); printf("\n");

	fd = git_futils_mktmp(&b, "tmpfile", 0600); printf("mktmp %d prefix=%d\n", fd >= 0, !git__prefixcmp(b.ptr, "tmpfile_git2_") && b.size == 29);
	if (fd >= 0) { p_close(fd); p_unlink(b.ptr); }

	/* rmdir */
	/* (one entry: readdir's order is not stable on every filesystem) */
	touchfile("e1/e2/only", "x");
	r = git_futils_rmdir_r("e1", NULL, GIT_RMDIR_EMPTY_HIERARCHY); printf("rmdir empty_hier"); perr(r); printf("\n");
	r = git_futils_rmdir_r("d1/d2", NULL, GIT_RMDIR_SKIP_NONEMPTY); printf("rmdir skip_nonempty"); perr(r); printf(" %d\n", git_fs_path_isdir("d1/d2"));
	r = git_futils_rmdir_r("tree", NULL, GIT_RMDIR_REMOVE_FILES | GIT_RMDIR_SKIP_ROOT); printf("rmdir skip_root"); perr(r); printf(" %d %d\n", git_fs_path_isdir("tree"), git_fs_path_is_empty_dir("tree"));
	r = git_futils_rmdir_r("d1/r1/r2", "d1", GIT_RMDIR_EMPTY_PARENTS); printf("rmdir parents"); perr(r); printf(" %d %d\n", git_fs_path_isdir("d1/r1"), git_fs_path_isdir("d1"));
	r = git_futils_rmdir_r("big/x/y", NULL, GIT_RMDIR_REMOVE_BLOCKERS); printf("rmdir blockers"); perr(r); printf(" %d\n", git_fs_path_exists("big"));
	r = git_futils_rmdir_r("trunc/x", NULL, 0); printf("rmdir notdir"); perr(r); printf("\n");
	r = git_futils_rmdir_r("none", NULL, 0); printf("rmdir missing"); perr(r); printf("\n");

	/* filebuf */
	{
		git_filebuf fb = GIT_FILEBUF_INIT;
		unsigned char hash[32];
		size_t i;
		void *res;

		r = git_filebuf_open(&fb, "fbuf/target", GIT_FILEBUF_HASH_SHA1 | GIT_FILEBUF_CREATE_LEADING_DIRS, 0644);
		printf("filebuf open"); perr(r); printf(" lock="); ps(fb.path_lock); printf("\n");
		{
			git_filebuf fb2 = GIT_FILEBUF_INIT;
			r = git_filebuf_open(&fb2, "fbuf/target", 0, 0644); printf("filebuf locked"); perr(r); printf("\n");
			git_filebuf_cleanup(&fb2);
		}
		for (i = 0; i < 3000; i++)
			git_filebuf_printf(&fb, "line %zu %s\n", i, "abcdefghij");
		git_filebuf_write(&fb, "tail", 4);
		r = git_filebuf_reserve(&fb, &res, 3); memcpy(res, "xyz", 3);
		r = git_filebuf_reserve(&fb, &res, 100000); printf("reserve big"); perr(r); printf("\n");
		r = git_filebuf_hash(hash, &fb); printf("filebuf hash"); perr(r); printf("\n");
		r = git_filebuf_commit(&fb); printf("filebuf commit"); perr(r); printf("\n");

		r = git_filebuf_open(&fb, "fbuf/target2", GIT_FILEBUF_HASH_SHA1, 0644); printf("filebuf2"); perr(r); printf("\n");
		for (i = 0; i < 3000; i++)
			git_filebuf_printf(&fb, "line %zu %s\n", i, "abcdefghij");
		git_filebuf_write(&fb, "tail", 4);
		r = git_filebuf_hash(hash, &fb); printf("filebuf2 hash"); perr(r);
		for (i = 0; i < 20; i++) printf("%02x", hash[i]);
		printf("\n");
		r = git_filebuf_commit(&fb); printf("filebuf2 commit"); perr(r); printf("\n");
		r = git_futils_readbuffer(&b, "fbuf/target2"); printf("  size=%zu\n", b.size);

		r = git_filebuf_open(&fb, "fbuf/target2", GIT_FILEBUF_APPEND | GIT_FILEBUF_HASH_SHA256 | (9 << GIT_FILEBUF_DEFLATE_SHIFT), 0644);
		printf("filebuf deflate"); perr(r); printf("\n");
		for (i = 0; i < 1000; i++)
			git_filebuf_write(&fb, "compress me please ", 19);
		r = git_filebuf_hash(hash, &fb); printf("deflate hash"); perr(r);
		for (i = 0; i < 32; i++) printf("%02x", hash[i]);
		printf("\n");
		r = git_filebuf_commit_at(&fb, "fbuf/target3"); printf("deflate commit_at"); perr(r); printf("\n");
		r = git_futils_readbuffer(&b, "fbuf/target3"); printf("  size=%zu\n", b.size);

		r = git_filebuf_open(&fb, "fbuf/temp", GIT_FILEBUF_TEMPORARY | GIT_FILEBUF_DO_NOT_BUFFER, 0644);
		printf("filebuf temp"); perr(r); printf(" orig="); ps(fb.path_original); printf("\n");
		git_filebuf_write(&fb, "abc", 3);
		{
			time_t mt; size_t sz;
			r = git_filebuf_stats(&mt, &sz, &fb); printf("stats"); perr(r); printf(" %zu\n", sz);
		}
		r = git_filebuf_commit(&fb); printf("temp commit"); perr(r); printf("\n");
		git_filebuf_cleanup(&fb);

		r = git_filebuf_open(&fb, "fbuf", 0, 0644); printf("filebuf dir"); perr(r); printf("\n");
		r = git_filebuf_open(&fb, "nodir/x", 0, 0644); printf("filebuf nodir"); perr(r); printf("\n");
		p_symlink("target2", "fbuf/sl");
		r = git_filebuf_open(&fb, "fbuf/sl", 0, 0644); printf("filebuf symlink"); perr(r); printf(" lock="); ps(fb.path_lock); printf("\n");
		git_filebuf_cleanup(&fb);
		printf("  after cleanup lock_exists=%d fd=%d\n", git_fs_path_exists("fbuf/target2.lock"), fb.fd);
		r = git_filebuf_open(&fb, "fbuf/x", 99 << GIT_FILEBUF_DEFLATE_SHIFT, 0644); printf("filebuf bad level"); perr(r); printf("\n");
	}

	r = git_futils_rmdir_r("fbuf", NULL, GIT_RMDIR_REMOVE_FILES); printf("rm fbuf"); perr(r); printf("\n");
	git_str_dispose(&b);
	git_vector_dispose(&v);
}

int main(void)
{
	char a[24], b[24];
	int i;
	const char **c, **d;

	git_libgit2_init();

	for (c = corpus; *c; c++) {
		test_names(*c);
		test_valid(*c);
	}
	for (c = corpus; *c; c += 3)
		for (d = corpus; *d; d += 2)
			test_pair(*c, *d);
	for (i = 0; i < 30000; i++) {
		random_path(a, sizeof(a) - 1);
		test_names(a);
		test_valid(a);
		random_path(b, sizeof(b) - 1);
		test_pair(a, b);
	}
	for (i = 0; i < 150000; i++) {
		random_path(a, sizeof(a) - 1);
		test_valid(a);
	}

	test_walk("a/b/c", NULL);
	test_walk("/a/b/c/", NULL);
	test_walk("/a/b/c", "/a");
	test_walk("/a/b/c", "/x");
	test_walk("", NULL);
	test_walk("a", "");
	test_walk("a/b/c/d/e/f/g/h/i", NULL);
	test_walk("//a//b//", "//");
	for (i = 0; i < 2000; i++) {
		random_path(a, sizeof(a) - 1);
		random_path(b, 6);
		test_walk(a, (i & 1) ? b : NULL);
	}

	test_set_error();
	test_modes();

	{
		git_str b2 = GIT_STR_INIT;
		char cwd[4096];
		const char *dir = "fs_diff.tmp";

		git_futils_rmdir_r(dir, NULL, GIT_RMDIR_REMOVE_FILES);
		git_futils_mkdir(dir, 0755, 0);
		if (!getcwd(cwd, sizeof(cwd)) || chdir(dir) < 0)
			return 1;
		umask(022);
		test_fs();
		if (chdir(cwd) < 0)
			return 1;
		i = git_futils_rmdir_r(dir, NULL, GIT_RMDIR_REMOVE_FILES);
		printf("cleanup %d\n", i);
		git_str_dispose(&b2);
	}

	git_libgit2_shutdown();
	return 0;
}
