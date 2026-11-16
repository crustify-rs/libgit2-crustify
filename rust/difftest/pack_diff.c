/*
 * Differential test of src/libgit2/pack.c, mwindow.c and odb_pack.c: the
 * pack index (v1, v2, 64-bit offsets, SHA-1 and SHA-256), object headers,
 * delta chains (OFS and REF bases, chains deeper than the 64-element stack),
 * inflation, streams, iteration, and the odb backend over them, on valid
 * packs and on thousands of corrupted copies of them.
 *
 * usage: pack_diff <corpus dir> [cases per set]   (see pack_corpus.sh)
 *
 * Every case runs in a child process with a time limit: on a corrupted pack
 * the C may crash (it reads out of bounds) or loop forever (a cyclic delta
 * chain), and pack_diff.sh then compares only the cases the C survived. A
 * case prints a summary line with a digest of every result (return codes,
 * error classes and messages, offsets, ids, sizes, types, data hashes);
 * PACKDIFF_VERBOSE=1 prints every result line too, and PACKDIFF_ONLY=<case>
 * runs only that case.
 *
 * difftest-link: libgit2_tests
 */

#include "common.h"
#include "git2.h"
#include "git2/sys/odb_backend.h"
#include "pack.h"
#include "mwindow.h"
#include "odb.h"
#include "oid.h"

#include <signal.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>
#include <fcntl.h>

static unsigned long long seed = 1234567;
static unsigned rnd(void)
{
	seed = seed * 6364136223846793005ULL + 1442695040888963407ULL;
	return (unsigned)(seed >> 33);
}

static int verbose;
static unsigned long long digest;
static unsigned nlines, nerrors;

static void emit(const char *fmt, ...)
{
	char buf[1024];
	va_list ap;
	size_t i, n;

	va_start(ap, fmt);
	n = (size_t)vsnprintf(buf, sizeof(buf), fmt, ap);
	va_end(ap);
	if (n >= sizeof(buf))
		n = sizeof(buf) - 1;
	for (i = 0; i < n; i++) {
		digest ^= (unsigned char)buf[i];
		digest *= 1099511628211ULL;
	}
	nlines++;
	if (verbose)
		printf("  %s\n", buf);
}

static void emit_rc(const char *what, int rc)
{
	const git_error *e = git_error_last();
	if (rc < 0) {
		nerrors++;
		emit("%s rc=%d class=%d msg=%s", what, rc, e ? e->klass : -1, e && e->message ? e->message : "");
	} else {
		emit("%s rc=%d", what, rc);
	}
}

static unsigned long long fnv(const void *p, size_t n)
{
	const unsigned char *b = p;
	unsigned long long h = 1469598103934665603ULL;
	size_t i;
	for (i = 0; i < n; i++) {
		h ^= b[i];
		h *= 1099511628211ULL;
	}
	return h;
}

/* ------------------------------------------------------------------ */

typedef struct {
	git_oid id;
	off64_t offset;
} entry;

static entry *entries;
static size_t nentries, aentries;

static int collect_cb(const git_oid *id, off64_t offset, void *payload)
{
	(void)payload;
	if (nentries == aentries) {
		aentries = aentries ? aentries * 2 : 256;
		entries = realloc(entries, aentries * sizeof(entry));
	}
	git_oid_cpy(&entries[nentries].id, id);
	entries[nentries].offset = offset;
	nentries++;
	return 0;
}

static unsigned long long foreach_hash;
static unsigned foreach_count;

/*
 * The C's git_pack_foreach_entry lists the ids of a SHA-256 pack at a
 * stride of 20 bytes (DEFECTS.md: wrong ids); the Rust lists the right
 * ones. For SHA-256 sets only the count of ids is compared.
 */
static int hash_foreach_ids;

static int foreach_cb(const git_oid *id, void *payload)
{
	(void)payload;
	if (hash_foreach_ids)
		foreach_hash = foreach_hash * 31 + fnv(id->id, git_oid_size(id->type));
	foreach_count++;
	return (payload && foreach_count == 5) ? 42 : 0;
}

static int foreach_offset_cb(const git_oid *id, off64_t off, void *payload)
{
	(void)payload;
	foreach_hash = foreach_hash * 31 + fnv(id->id, git_oid_size(id->type)) + (unsigned long long)off;
	foreach_count++;
	return 0;
}

static void lookup_one(struct git_pack_file *p, const git_oid *id, size_t len, int deep)
{
	struct git_pack_entry e;
	char hex[GIT_OID_MAX_HEXSIZE + 1];
	int rc;

	memset(&e, 0, sizeof(e));
	git_error_clear();
	rc = git_pack_entry_find(&e, p, id, len);
	git_oid_tostr(hex, sizeof(hex), id);
	if (rc < 0) {
		emit_rc("find", rc);
		emit("  of %.*s", (int)len, hex);
		return;
	}
	git_oid_tostr(hex, sizeof(hex), &e.id);
	emit("find %zu ok off=%lld id=%s", len, (long long)e.offset, hex);
	if (!deep)
		return;

	{
		size_t size = 0;
		git_object_t type = 0;
		git_error_clear();
		rc = git_packfile_resolve_header(&size, &type, p, e.offset);
		emit_rc("resolve_header", rc);
		if (rc == 0)
			emit("  size=%zu type=%d", size, type);
	}
	{
		git_rawobj raw = {0};
		off64_t off = e.offset;
		git_error_clear();
		rc = git_packfile_unpack(&raw, p, &off);
		emit_rc("unpack", rc);
		emit("  off=%lld", (long long)off);
		if (rc == 0) {
			emit("  len=%zu type=%d hash=%016llx nul=%d", raw.len, raw.type,
			     fnv(raw.data, raw.len), ((char *)raw.data)[raw.len]);
			git__free(raw.data);
		}
	}
	{
		/* The stream API over the object's data, read in odd chunks. */
		size_t size = 0;
		git_object_t type = 0;
		off64_t curpos = e.offset;
		git_mwindow *w = NULL;
		git_error_clear();
		rc = git_packfile_unpack_header(&size, &type, p, &w, &curpos);
		git_mwindow_close(&w);
		emit_rc("unpack_header", rc);
		if (rc == 0) {
			emit("  size=%zu type=%d pos=%lld", size, type, (long long)curpos);
			if (type == GIT_PACKFILE_OFS_DELTA || type == GIT_PACKFILE_REF_DELTA) {
				off64_t base = 0;
				git_error_clear();
				rc = get_delta_base(&base, p, &w, &curpos, type, e.offset);
				git_mwindow_close(&w);
				emit_rc("delta_base", rc);
				emit("  base=%lld pos=%lld", (long long)base, (long long)curpos);
			}
			if (rc == 0) {
				git_packfile_stream s;
				unsigned long long h = 0;
				size_t total = 0;
				int loops = 0;
				rc = git_packfile_stream_open(&s, p, curpos);
				emit_rc("stream_open", rc);
				while (rc == 0 && loops++ < 100000) {
					char buf[777];
					ssize_t n = git_packfile_stream_read(&s, buf, 1 + rnd() % sizeof(buf));
					if (n == GIT_EBUFS)
						continue;
					if (n <= 0) {
						if (n < 0)
							emit_rc("stream_read", (int)n);
						break;
					}
					h = h * 31 + fnv(buf, (size_t)n);
					total += (size_t)n;
				}
				emit("  stream total=%zu hash=%016llx curpos=%lld done=%d", total, h, (long long)s.curpos, s.done);
				if (rc == 0)
					git_packfile_stream_dispose(&s);
			}
		}
	}
}

static void run_pack(const char *idx_path, git_oid_t oid_type)
{
	struct git_pack_file *p = NULL;
	size_t i, hexsize = git_oid_hexsize(oid_type);
	int rc;

	git_error_clear();
	rc = git_packfile_alloc(&p, idx_path, oid_type);
	emit_rc("alloc", rc);
	if (rc < 0)
		return;

	for (i = 0; i < nentries; i++)
		lookup_one(p, &entries[i].id, hexsize, (i % 3) == 0 || nentries < 200);

	/* Prefixes: unique, ambiguous, missing. */
	for (i = 0; i < nentries; i += 1 + nentries / 40) {
		lookup_one(p, &entries[i].id, 7, 0);
		lookup_one(p, &entries[i].id, 4, 0);
	}
	for (i = 0; i < 20; i++) {
		git_oid id;
		unsigned char raw[GIT_OID_MAX_SIZE];
		size_t j;
		for (j = 0; j < sizeof(raw); j++)
			raw[j] = (unsigned char)rnd();
		git_oid_from_raw(&id, raw, oid_type);
		lookup_one(p, &id, hexsize, 0);
		lookup_one(p, &id, 5, 0);
	}

	foreach_hash = 0; foreach_count = 0;
	git_error_clear();
	rc = git_pack_foreach_entry(p, foreach_cb, NULL);
	emit_rc("foreach", rc);
	emit("  count=%u hash=%016llx", foreach_count, foreach_hash);

	foreach_hash = 0; foreach_count = 0;
	git_error_clear();
	rc = git_pack_foreach_entry(p, foreach_cb, (void *)1);
	emit_rc("foreach_stop", rc);
	emit("  count=%u", foreach_count);

	foreach_hash = 0; foreach_count = 0;
	git_error_clear();
	rc = git_pack_foreach_entry_offset(p, foreach_offset_cb, NULL);
	emit_rc("foreach_offset", rc);
	emit("  count=%u hash=%016llx", foreach_count, foreach_hash);

	git_packfile_free(p, 0);
}

static void run_odb(const char *idx_path, git_oid_t oid_type)
{
	git_odb_backend *b = NULL;
	git_odb *odb = NULL;
	git_odb_backend_pack_options bo = GIT_ODB_BACKEND_PACK_OPTIONS_INIT;
	git_odb_options oo = GIT_ODB_OPTIONS_INIT;
	size_t i, hexsize = git_oid_hexsize(oid_type);
	int rc;

	bo.oid_type = oid_type;
	oo.oid_type = oid_type;
	git_error_clear();
	rc = git_odb_backend_one_pack(&b, idx_path, &bo);
	emit_rc("one_pack", rc);
	if (rc < 0)
		return;
	if (git_odb_new_ext(&odb, &oo) < 0 || git_odb_add_backend(odb, b, 1) < 0) {
		emit("odb setup failed");
		return;
	}
	for (i = 0; i < nentries; i += 1 + nentries / 60) {
		git_odb_object *obj = NULL;
		size_t len = 0;
		git_object_t type = 0;
		git_oid out;

		git_error_clear();
		rc = git_odb_read_header(&len, &type, odb, &entries[i].id);
		emit_rc("odb_read_header", rc);
		if (rc == 0)
			emit("  len=%zu type=%d", len, type);
		git_error_clear();
		rc = git_odb_read(&obj, odb, &entries[i].id);
		emit_rc("odb_read", rc);
		if (rc == 0) {
			emit("  len=%zu type=%d hash=%016llx", git_odb_object_size(obj), git_odb_object_type(obj),
			     fnv(git_odb_object_data(obj), git_odb_object_size(obj)));
			git_odb_object_free(obj);
		}
		emit("odb_exists %d", git_odb_exists(odb, &entries[i].id));
		git_error_clear();
		rc = git_odb_exists_prefix(&out, odb, &entries[i].id, 6);
		emit_rc("odb_exists_prefix", rc);
		git_error_clear();
		rc = git_odb_read_prefix(&obj, odb, &entries[i].id, 9);
		emit_rc("odb_read_prefix", rc);
		if (rc == 0)
			git_odb_object_free(obj);
		(void)hexsize;
	}
	foreach_hash = 0; foreach_count = 0;
	git_error_clear();
	rc = git_odb_foreach(odb, foreach_cb, NULL);
	emit_rc("odb_foreach", rc);
	emit("  count=%u hash=%016llx", foreach_count, foreach_hash);
	git_odb_free(odb);
}

/* ------------------------------------------------------------------ */

static unsigned char *slurp(const char *path, size_t *len)
{
	FILE *f = fopen(path, "rb");
	unsigned char *b;
	long n;
	if (!f)
		return NULL;
	fseek(f, 0, SEEK_END);
	n = ftell(f);
	fseek(f, 0, SEEK_SET);
	b = malloc((size_t)n + 64);
	if (fread(b, 1, (size_t)n, f) != (size_t)n)
		abort();
	fclose(f);
	*len = (size_t)n;
	return b;
}

static void spit(const char *path, const unsigned char *b, size_t len)
{
	FILE *f;
	unlink(path);
	f = fopen(path, "wb");
	if (!f || fwrite(b, 1, len, f) != len)
		abort();
	fclose(f);
}

#define WORK "/tmp/packdiff-work"

static void run_case(const char *label, const unsigned char *idx, size_t idx_len,
		     const unsigned char *pack, size_t pack_len, git_oid_t oid_type)
{
	pid_t pid;
	int status;
	const char *only = getenv("PACKDIFF_ONLY");

	if (only && strcmp(only, label) != 0)
		return;
	spit(WORK "/pack-x.idx", idx, idx_len);
	spit(WORK "/pack-x.pack", pack, pack_len);
	fflush(stdout);
	pid = fork();
	if (pid == 0) {
		alarm(20);
		digest = 1469598103934665603ULL;
		nlines = nerrors = 0;
		if (verbose)
			printf("case %s\n", label);
		run_pack(WORK "/pack-x.idx", oid_type);
		run_odb(WORK "/pack-x.idx", oid_type);
		printf("case %s: lines=%u errors=%u digest=%016llx\n", label, nlines, nerrors, digest);
		fflush(stdout);
		_exit(0);
	}
	waitpid(pid, &status, 0);
	if (WIFSIGNALED(status))
		printf("case %s: SIGNAL %d\n", label, WTERMSIG(status));
	else if (WEXITSTATUS(status) != 0)
		printf("case %s: EXIT %d\n", label, WEXITSTATUS(status));
}

static void be32put(unsigned char *p, uint32_t v)
{
	p[0] = v >> 24; p[1] = v >> 16; p[2] = v >> 8; p[3] = (unsigned char)v;
}

int main(int argc, char **argv)
{
	char path[4096], name[256], label[512];
	FILE *sets;
	int ncases = argc > 2 ? atoi(argv[2]) : 300;

	verbose = getenv("PACKDIFF_VERBOSE") != NULL;
	git_libgit2_init();
	mkdir(WORK, 0755);
	snprintf(path, sizeof(path), "%s/sets.txt", argv[1]);
	sets = fopen(path, "r");
	if (!sets) {
		fprintf(stderr, "no corpus at %s\n", argv[1]);
		return 1;
	}
	while (fscanf(sets, "%255s", name) == 1) {
		unsigned char *idx, *pack, *m;
		size_t idx_len, pack_len, m_len, oid_size, nr, oids_at, offs_at, i;
		git_oid_t oid_type;
		int k, v2;
		FILE *tf;
		struct git_pack_file *p = NULL;

		snprintf(path, sizeof(path), "%s/%s.type", argv[1], name);
		tf = fopen(path, "r");
		if (!tf || fscanf(tf, "%d", (int *)&oid_type) != 1)
			return 1;
		fclose(tf);
		oid_size = git_oid_size(oid_type);
		hash_foreach_ids = oid_type == GIT_OID_SHA1;
		snprintf(path, sizeof(path), "%s/%s.idx", argv[1], name);
		idx = slurp(path, &idx_len);
		snprintf(path, sizeof(path), "%s/%s.pack", argv[1], name);
		pack = slurp(path, &pack_len);

		/* The entries, from the valid index. */
		spit(WORK "/pack-x.idx", idx, idx_len);
		spit(WORK "/pack-x.pack", pack, pack_len);
		nentries = 0;
		if (git_packfile_alloc(&p, WORK "/pack-x.idx", oid_type) < 0 ||
		    git_pack_foreach_entry_offset(p, collect_cb, NULL) < 0) {
			fprintf(stderr, "cannot read %s\n", name);
			return 1;
		}
		git_packfile_free(p, 0);
		printf("set %s: %zu objects\n", name, nentries);

		v2 = idx[0] == 0xff && idx[1] == 't';
		nr = nentries;
		oids_at = v2 ? 8 + 1024 : 1024 + 4;
		offs_at = v2 ? 8 + 1024 + nr * (oid_size + 4) : 1024;

		snprintf(label, sizeof(label), "%s/valid", name);
		run_case(label, idx, idx_len, pack, pack_len, oid_type);

		m = malloc((idx_len > pack_len ? idx_len : pack_len) + 4096);
		for (k = 0; k < ncases; k++) {
			unsigned what = rnd() % 16;
			const char *desc = "";
			memcpy(m, idx, idx_len);
			m_len = idx_len;
			if (what < 8) {
				/* The index. */
				switch (what) {
				case 0: { size_t at = rnd() % (v2 ? 1032 : 1024); m[at] ^= 1u << (rnd() % 8); desc = "idx-header-bit"; break; }
				case 1: { size_t at = (v2 ? 8 : 0) + 4 * (rnd() % 256); be32put(m + at, rnd() % (unsigned)(nr * 2 + 2)); desc = "idx-fanout"; break; }
				case 2: { size_t e = rnd() % nr; size_t at = offs_at + (v2 ? 4 * e : (oid_size + 4) * e); be32put(m + at, rnd()); desc = "idx-offset"; break; }
				case 3: { size_t e = rnd() % nr; size_t at = offs_at + (v2 ? 4 * e : (oid_size + 4) * e); be32put(m + at, 0x80000000u | (rnd() % (unsigned)(nr + 4))); desc = "idx-large-offset"; break; }
				case 4: { size_t e = rnd() % nr; size_t at = oids_at + (v2 ? oid_size : oid_size + 4) * e + rnd() % oid_size; m[at] ^= 1u << (rnd() % 8); desc = "idx-oid-bit"; break; }
				case 5: { m_len = rnd() % idx_len; desc = "idx-truncate"; break; }
				case 6: { size_t n = 1 + rnd() % 64, j; for (j = 0; j < n; j++) m[idx_len + j] = (unsigned char)rnd(); m_len = idx_len + n; desc = "idx-extend"; break; }
				default: { size_t at = rnd() % idx_len; m[at] = (unsigned char)rnd(); desc = "idx-byte"; break; }
				}
				snprintf(label, sizeof(label), "%s/%d/%s", name, k, desc);
				run_case(label, m, m_len, pack, pack_len, oid_type);
			} else {
				/* The pack (its header, an object's header, base reference or data). */
				size_t e = rnd() % nr;
				size_t at = (size_t)entries[e].offset;
				memcpy(m, pack, pack_len);
				m_len = pack_len;
				switch (what) {
				case 8: m[at] ^= 1u << (rnd() % 8); desc = "obj-header-bit"; break;
				case 9: m[at + 1 + rnd() % 3] = (unsigned char)rnd(); desc = "obj-header-cont"; break;
				case 10: { size_t j; for (j = 1; j < 1 + rnd() % 12; j++) m[at + j] = 0xff; desc = "obj-ofs-ff"; break; }
				case 11: m[at + 2 + rnd() % 30] ^= 1u << (rnd() % 8); desc = "obj-base-bit"; break;
				case 12: m[at + 8 + rnd() % 40] = (unsigned char)rnd(); desc = "obj-data-byte"; break;
				case 13: m[rnd() % 12] ^= 1u << (rnd() % 8); desc = "pack-header-bit"; break;
				case 14: {
					/* A REF_DELTA whose base is another object of the pack (maybe itself: a cycle). */
					size_t b = rnd() % nr;
					m[at] = (m[at] & 0x8f) | (GIT_PACKFILE_REF_DELTA << 4);
					{ size_t j = at; while (m[j] & 0x80) j++; memcpy(m + j + 1, entries[(rnd() % 4) ? b : e].id.id, oid_size); }
					desc = "obj-ref-to";
					break;
				}
				default: m_len = rnd() % pack_len; desc = "pack-truncate"; break;
				}
				snprintf(label, sizeof(label), "%s/%d/%s", name, k, desc);
				run_case(label, idx, idx_len, m, m_len, oid_type);
			}
		}
		free(m);
		free(idx);
		free(pack);
	}
	fclose(sets);
	git_libgit2_shutdown();
	return 0;
}
