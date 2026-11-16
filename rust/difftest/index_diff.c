/*
 * Differential test of work package l-index: every line printed must be
 * the same when linked against the C reference and against the Rust port.
 *
 * - Every index file under tests/resources (and the loose *.index files),
 *   read: the entries, REUC and NAME records, the tree cache, the
 *   checksum, or the error.
 * - Each, written back in versions 2, 3 and 4 (and read again): the bytes
 *   written, hashed.
 * - Generated indexes (long paths, extended flags, conflicts, REUC, NAME,
 *   unknown extensions, SHA-256), written in each version.
 * - Corrupt files: truncations and single-byte changes of small indexes
 *   (with the checksum fixed up, so that the parser goes past it), and the
 *   error each gives.
 *
 * difftest-link: libgit2_tests
 *
 * RUST_BUILD=/work/builds/l-index rust/difftest/run.sh rust/difftest/index_diff.c
 */

#include "common.h"
#include "git2.h"
#include "git2/sys/index.h"
#include "index.h"
#include "tree-cache.h"
#include "futils.h"
#include "hash.h"
#include "oid.h"

#include <dirent.h>
#include <sys/stat.h>

static char tmpdir[4096];

static uint64_t fnv(uint64_t h, const void *p, size_t n)
{
	const unsigned char *s = p;
	while (n--) { h ^= *s++; h *= 1099511628211ULL; }
	return h;
}

static void show_err(const char *what, int rc)
{
	const git_error *e = git_error_last();
	printf("  %s rc=%d class=%d msg=%s\n", what, rc, e ? e->klass : -1, e ? e->message : "(none)");
	git_error_clear();
}

static void dump_tree(const git_tree_cache *t, int depth)
{
	size_t i;
	char hex[GIT_OID_MAX_HEXSIZE + 1] = {0};
	if (!t) return;
	if (t->entry_count >= 0) git_oid_tostr(hex, sizeof(hex), &t->oid);
	printf("  tree %*s'%s' %d %d %s\n", depth, "", t->name, (int)t->entry_count, (int)t->children_count, hex);
	for (i = 0; i < t->children_count && depth < 6; i++)
		dump_tree(t->children[i], depth + 1);
}

static void dump(git_index *ix)
{
	size_t i, n = git_index_entrycount(ix);
	uint64_t h = 1469598103934665603ULL;
	char hex[GIT_OID_MAX_HEXSIZE + 1];

	printf("  entries=%zu version=%u conflicts=%d reuc=%zu names=%zu caps=%d\n", n,
		git_index_version(ix), git_index_has_conflicts(ix),
		git_index_reuc_entrycount(ix), git_index_name_entrycount(ix), git_index_caps(ix));
	for (i = 0; i < n; i++) {
		const git_index_entry *e = git_index_get_byindex(ix, i);
		h = fnv(h, e, offsetof(git_index_entry, path));
		h = fnv(h, e->path, strlen(e->path) + 1);
		if (i < 8 || i % 97 == 0) {
			git_oid_tostr(hex, sizeof(hex), &e->id);
			printf("  [%zu] %s %o %s f=%x fx=%x sz=%u m=%d.%u\n", i, e->path, e->mode, hex,
				e->flags, e->flags_extended, e->file_size, e->mtime.seconds, e->mtime.nanoseconds);
		}
	}
	printf("  entries-hash %016llx\n", (unsigned long long)h);
	for (i = 0; i < git_index_reuc_entrycount(ix); i++) {
		const git_index_reuc_entry *r = git_index_reuc_get_byindex(ix, i);
		git_oid_tostr(hex, sizeof(hex), &r->oid[1]);
		printf("  reuc %s %o %o %o %s\n", r->path, r->mode[0], r->mode[1], r->mode[2], hex);
	}
	for (i = 0; i < git_index_name_entrycount(ix); i++) {
		const git_index_name_entry *ne = git_index_name_get_byindex(ix, i);
		printf("  name %s %s %s\n", ne->ancestor ? ne->ancestor : "-", ne->ours ? ne->ours : "-",
			ne->theirs ? ne->theirs : "-");
	}
	dump_tree(ix->tree, 0);
	{
		size_t cs = git_oid_size(ix->oid_type);
		printf("  checksum %016llx\n", (unsigned long long)fnv(1469598103934665603ULL, git_index__checksum(ix), cs));
	}
	{
		git_buf b = GIT_BUF_INIT;
		int rc = git_index_extension_lookup(&b, ix, "ZZZZ");
		printf("  ext ZZZZ rc=%d size=%zu\n", rc, b.size);
		git_buf_dispose(&b);
	}
}

static void file_hash(const char *path)
{
	git_str buf = GIT_STR_INIT;
	if (git_futils_readbuffer(&buf, path) < 0) {
		show_err("readfile", -1);
		return;
	}
	printf("  file size=%zu hash=%016llx\n", buf.size,
		(unsigned long long)fnv(1469598103934665603ULL, buf.ptr, buf.size));
	git_str_dispose(&buf);
}

static void write_versions(git_index *ix, git_oid_t t)
{
	unsigned v;
	char out[4200];
	git_index_options opts = GIT_INDEX_OPTIONS_INIT;
	opts.oid_type = t;
	for (v = 2; v <= 4; v++) {
		git_index *w, *r;
		int rc;
		snprintf(out, sizeof(out), "%s/out-%u", tmpdir, v);
		unlink(out);
		if ((rc = git_index_open_ext(&w, out, &opts)) < 0) { show_err("open-w", rc); continue; }
		if ((rc = git_index_read_index(w, ix)) < 0) show_err("read_index", rc);
		{
			size_t i;
			for (i = 0; i < git_index_reuc_entrycount(ix); i++) {
				const git_index_reuc_entry *e = git_index_reuc_get_byindex(ix, i);
				git_index_reuc_add(w, e->path, e->mode[0], &e->oid[0], e->mode[1], &e->oid[1], e->mode[2], &e->oid[2]);
			}
			for (i = 0; i < git_index_name_entrycount(ix); i++) {
				const git_index_name_entry *e = git_index_name_get_byindex(ix, i);
				git_index_name_add(w, e->ancestor, e->ours, e->theirs);
			}
		}
		git_index_set_version(w, v);
		rc = git_index_write(w);
		printf(" write v%u rc=%d\n", v, rc);
		if (rc < 0) show_err("write", rc);
		file_hash(out);
		if ((rc = git_index_open_ext(&r, out, &opts)) < 0) {
			show_err("reopen", rc);
		} else {
			printf("  reread entries=%zu version=%u\n", git_index_entrycount(r), git_index_version(r));
			git_index_free(r);
		}
		git_index_free(w);
	}
}

static void one_file(const char *path, git_oid_t t)
{
	git_index *ix;
	git_index_options opts = GIT_INDEX_OPTIONS_INIT;
	int rc;
	opts.oid_type = t;
	printf("== %s (%d)\n", path, t);
	if ((rc = git_index_open_ext(&ix, path, &opts)) < 0) {
		show_err("open", rc);
		return;
	}
	dump(ix);
	write_versions(ix, t);
	git_index_free(ix);
}

static void walk(const char *dir, int depth)
{
	DIR *d = opendir(dir);
	struct dirent *de;
	char p[4096];
	char *names[512];
	int n = 0, i;
	if (!d) return;
	while ((de = readdir(d)) && n < 512) {
		if (de->d_name[0] == '.' && strcmp(de->d_name, ".gitted")) continue;
		names[n++] = strdup(de->d_name);
	}
	closedir(d);
	/* stable order */
	for (i = 0; i < n; i++) {
		int j;
		for (j = i + 1; j < n; j++)
			if (strcmp(names[j], names[i]) < 0) { char *x = names[i]; names[i] = names[j]; names[j] = x; }
	}
	for (i = 0; i < n; i++) {
		struct stat st;
		snprintf(p, sizeof(p), "%s/%s", dir, names[i]);
		if (stat(p, &st) == 0) {
			if (S_ISDIR(st.st_mode) && depth < 2)
				walk(p, depth + 1);
			else if (S_ISREG(st.st_mode) &&
				 (!strcmp(names[i], "index") || strstr(names[i], ".index")))
				one_file(p, strstr(p, "256") ? GIT_OID_SHA256 : GIT_OID_SHA1);
		}
		free(names[i]);
	}
}

/* fix up the trailing checksum of buf */
static void fix_checksum(unsigned char *buf, size_t len, git_oid_t t)
{
	size_t cs = git_oid_size(t);
	unsigned char h[GIT_HASH_MAX_SIZE];
	git_hash_buf(h, buf, len - cs, t == GIT_OID_SHA256 ? GIT_HASH_ALGORITHM_SHA256 : GIT_HASH_ALGORITHM_SHA1);
	memcpy(buf + len - cs, h, cs);
}

static void try_buffer(const unsigned char *buf, size_t len, const char *label, git_oid_t t)
{
	char p[4200];
	git_index *ix;
	git_index_options opts = GIT_INDEX_OPTIONS_INIT;
	int rc;
	opts.oid_type = t;
	snprintf(p, sizeof(p), "%s/corrupt", tmpdir);
	if (git_futils_writebuffer(&(git_str){ (char *)buf, 0, len }, p, 0, 0644) < 0) {
		show_err("writebuffer", -1);
		return;
	}
	rc = git_index_open_ext(&ix, p, &opts);
	printf("%s rc=%d", label, rc);
	if (rc < 0) {
		const git_error *e = git_error_last();
		printf(" %s\n", e ? e->message : "(none)");
		git_error_clear();
	} else {
		printf(" entries=%zu reuc=%zu names=%zu tree=%d\n", git_index_entrycount(ix),
			git_index_reuc_entrycount(ix), git_index_name_entrycount(ix), ix->tree != NULL);
		git_index_free(ix);
	}
}

static void corrupt(const char *path, git_oid_t t)
{
	git_str buf = GIT_STR_INIT;
	size_t i, step;
	char label[128];
	printf("== corrupt %s\n", strncmp(path, tmpdir, strlen(tmpdir)) ? path : strrchr(path, '/') + 1);
	if (git_futils_readbuffer(&buf, path) < 0) { show_err("read", -1); return; }
	step = buf.size / 300 + 1;
	for (i = 13; i < buf.size; i += step) {
		unsigned char *c = git__malloc(buf.size);
		memcpy(c, buf.ptr, buf.size);
		c[i] ^= (i & 1) ? 0x80 : 0x07;
		fix_checksum(c, buf.size, t);
		snprintf(label, sizeof(label), " flip@%zu", i);
		try_buffer(c, buf.size, label, t);
		/* truncate (checksum recomputed at the new end) */
		if (i + git_oid_size(t) < buf.size) {
			fix_checksum(c, i + git_oid_size(t), t);
			memcpy(c, buf.ptr, i);
			fix_checksum(c, i + git_oid_size(t), t);
			snprintf(label, sizeof(label), " cut@%zu", i);
			try_buffer(c, i + git_oid_size(t), label, t);
		}
		git__free(c);
	}
	git_str_dispose(&buf);
}

static void generated(git_oid_t t)
{
	git_index *ix;
	git_index_options opts = GIT_INDEX_OPTIONS_INIT;
	git_index_entry e;
	char path[5000];
	git_oid id;
	int i, rc;
	opts.oid_type = t;
	printf("== generated (%d)\n", t);
	if ((rc = git_index_new_ext(&ix, &opts)) < 0) { show_err("new", rc); return; }
	memset(&id, 0, sizeof(id));
	git_oid_from_string(&id, t == GIT_OID_SHA256 ?
		"c0ffeec0ffeec0ffeec0ffeec0ffeec0ffeec0ffeec0ffeec0ffeec0ffeec0ff" :
		"c0ffeec0ffeec0ffeec0ffeec0ffeec0ffeec0ff", t);
	for (i = 0; i < 300; i++) {
		memset(&e, 0, sizeof(e));
		e.mode = (i % 7) ? GIT_FILEMODE_BLOB : GIT_FILEMODE_BLOB_EXECUTABLE;
		e.id = id;
		e.mtime.seconds = 1000 + i;
		e.mtime.nanoseconds = 7 * i;
		e.file_size = i * 13;
		if (i % 11 == 0)
			e.flags_extended = GIT_INDEX_ENTRY_INTENT_TO_ADD;
		if (i % 13 == 0)
			e.flags_extended |= GIT_INDEX_ENTRY_SKIP_WORKTREE;
		if (i == 5) {
			memset(path, 'L', 4000);
			path[4000] = 0;
			path[100] = '/';
		} else {
			snprintf(path, sizeof(path), "dir%d/sub%d/file-%03d.txt", i % 5, i % 3, i);
		}
		e.path = path;
		if ((rc = git_index_add(ix, &e)) < 0) show_err("add", rc);
	}
	{
		git_index_entry a = e, o = e, th = e;
		a.path = o.path = th.path = "conflicted/file";
		if ((rc = git_index_conflict_add(ix, &a, &o, &th)) < 0) show_err("conflict_add", rc);
	}
	git_index_reuc_add(ix, "resolved", 0100644, &id, 0, NULL, 0100755, &id);
	git_index_name_add(ix, "anc", "ours", NULL);
	git_index_extension_add(ix, "ZZZZ", "hello", 5);
	git_index_extension_add(ix, "YYY", "short", 5);
	dump(ix);
	{
		unsigned v;
		char out[4200];
		for (v = 2; v <= 4; v++) {
			git_index *r;
			snprintf(out, sizeof(out), "%s/gen-%u", tmpdir, v);
			unlink(out);
			/* write through a file-backed index of the same entries */
			{
				git_index *w;
				git_index_open_ext(&w, out, &opts);
				git_index_read_index(w, ix);
				git_index_reuc_add(w, "resolved", 0100644, &id, 0, NULL, 0100755, &id);
				git_index_name_add(w, "anc", "ours", NULL);
				git_index_extension_add(w, "ZZZZ", "hello", 5);
				git_index_set_version(w, v);
				rc = git_index_write(w);
				printf(" gen write v%u rc=%d\n", v, rc);
				if (rc < 0) show_err("write", rc);
				git_index_free(w);
			}
			file_hash(out);
			if ((rc = git_index_open_ext(&r, out, &opts)) < 0) { show_err("reopen", rc); continue; }
			dump(r);
			git_index_free(r);
			if (v == 4 || v == 2) corrupt(out, t);
		}
	}
	git_index_free(ix);
}

/* TREE names with high-bit bytes, and REUC modes that are negative ints */
static void bytes_and_modes(void)
{
	static const unsigned char tree[] = "\0-1 2\n\xff\xfe\0-1 0\nlatin\xe9\0-1 0\n";
	unsigned char buf[256];
	size_t n = 0, tl = sizeof(tree) - 1;
	char p[4200];
	git_index *ix;
	git_oid id;
	int rc;

	printf("== bytes and modes\n");
	memcpy(buf, "DIRC\0\0\0\2\0\0\0\0", 12); n = 12;
	memcpy(buf + n, "TREE", 4); n += 4;
	buf[n++] = 0; buf[n++] = 0; buf[n++] = 0; buf[n++] = (unsigned char)tl;
	memcpy(buf + n, tree, tl); n += tl;
	n += 20;
	fix_checksum(buf, n, GIT_OID_SHA1);
	snprintf(p, sizeof(p), "%s/treenames", tmpdir);
	git_futils_writebuffer(&(git_str){ (char *)buf, 0, n }, p, 0, 0644);
	if ((rc = git_index_open(&ix, p)) < 0) { show_err("open", rc); return; }
	dump_tree(ix->tree, 0);
	rc = git_index_write(ix);
	printf(" write rc=%d\n", rc);
	file_hash(p);

	git_oid_from_string(&id, "1111111111111111111111111111111111111111", GIT_OID_SHA1);
	rc = git_index_reuc_add(ix, "neg", -1, &id, 0, NULL, 0, NULL);
	printf(" reuc -1 with id rc=%d\n", rc);
	if (rc < 0) show_err("reuc", rc);
	rc = git_index_reuc_add(ix, "negnull", -1, NULL, 0, NULL, 0, NULL);
	printf(" reuc -1 without id rc=%d\n", rc);
	if (rc < 0) show_err("reuc", rc);
	rc = git_index_write(ix);
	printf(" write rc=%d\n", rc);
	file_hash(p);
	git_index_free(ix);
}

int main(void)
{
	git_libgit2_init();
	snprintf(tmpdir, sizeof(tmpdir), "/tmp/index_diff.%d", (int)getpid());
	mkdir(tmpdir, 0755);
	walk("tests/resources", 0);
	bytes_and_modes();
	generated(GIT_OID_SHA1);
	generated(GIT_OID_SHA256);
	corrupt("tests/resources/indexv4/.gitted/index", GIT_OID_SHA1);
	corrupt("tests/resources/status/.gitted/index", GIT_OID_SHA1);
	corrupt("tests/resources/mergedrepo/.gitted/index", GIT_OID_SHA1);
	{
		char cmd[4200];
		snprintf(cmd, sizeof(cmd), "rm -rf %s", tmpdir);
		if (system(cmd)) {}
	}
	git_libgit2_shutdown();
	return 0;
}
