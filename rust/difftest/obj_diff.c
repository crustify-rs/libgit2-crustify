/*
 * Differential test of work package l-obj: every line printed must be the
 * same when linked against the C reference and against the Rust port.
 *
 * Covers the parsers and encoders of object.c, tag.c, signature.c and
 * grafts.c on valid and malformed inputs: git_signature__parse (headers,
 * enders, every truncation and random mutations of real and hostile
 * signature lines), git_signature_from_buffer, git_signature_new (trimming,
 * validation), git_signature__writebuf; tag parsing through
 * git_object__from_raw and git_object_rawcontent_is_valid (the fuzzers'
 * corpus, every truncation, byte mutations, field-level variants); grafts
 * parsing; object type names, object ids of buffers, oid headers.
 *
 * difftest-link: libgit2_tests
 *
 * Build and compare: RUST_BUILD=/work/builds/l-obj rust/difftest/run.sh rust/difftest/obj_diff.c
 */

#include "common.h"
#include "git2/global.h"
#include "git2/errors.h"
#include "git2/object.h"
#include "git2/signature.h"
#include "object.h"
#include "signature.h"
#include "tag.h"
#include "grafts.h"
#include "str.h"

static unsigned long seed = 4243;
static unsigned rnd(void)
{
	seed = seed * 6364136223846793005UL + 1442695040888963407UL;
	return (unsigned)(seed >> 33);
}

static unsigned long hash_state;
static void h_bytes(const void *p, size_t n)
{
	const unsigned char *b = p;
	size_t i;
	for (i = 0; i < n; i++)
		hash_state = (hash_state ^ b[i]) * 1099511628211UL;
}
static void h_str(const char *s) { if (s) h_bytes(s, strlen(s) + 1); else h_bytes("\xff", 1); }
static void h_int(long v) { h_bytes(&v, sizeof(v)); }

/* Print the error state and clear it. */
static void err_line(char *out, size_t n)
{
	const git_error *e = git_error_last();
	snprintf(out, n, "class=%d msg=%s", e ? e->klass : -1, e ? e->message : "(none)");
	git_error_clear();
}

static void print_quoted(const char *s, size_t len)
{
	size_t i;
	for (i = 0; i < len; i++) {
		unsigned char c = s[i];
		if (c >= 32 && c < 127 && c != '\\')
			putchar(c);
		else
			printf("\\x%02x", c);
	}
}

/* ------------------------------------------------------------------ */
/* signatures */

static void sig_case(const char *buf, size_t len, const char *header, char ender, int verbose)
{
	git_signature sig;
	const char *p = buf;
	char e[600];
	int rc;

	memset(&sig, 0xAB, sizeof(sig));
	git_error_clear();
	rc = git_signature__parse(&sig, &p, buf + len, header, ender);
	err_line(e, sizeof(e));

	if (verbose) {
		printf("sig hdr=%s ender=%d in=\"", header ? header : "(null)", ender);
		print_quoted(buf, len);
		printf("\" rc=%d", rc);
		if (rc == 0)
			printf(" name=\"%s\" email=\"%s\" time=%" PRId64 " off=%d sign=%d used=%ld",
				sig.name, sig.email, (int64_t)sig.when.time, sig.when.offset,
				sig.when.sign, (long)(p - buf));
		else
			printf(" name=%p email=%p", (void *)sig.name, (void *)sig.email);
		printf(" %s\n", e);
	} else {
		h_int(rc);
		h_str(e);
		if (rc == 0) {
			h_str(sig.name);
			h_str(sig.email);
			h_int(sig.when.time);
			h_int(sig.when.offset);
			h_int(sig.when.sign);
			h_int(p - buf);
		} else {
			h_int(sig.name == NULL);
			h_int(sig.email == NULL);
		}
	}

	if (rc == 0) {
		git__free(sig.name);
		git__free(sig.email);
	}
}

static const char *sig_corpus[] = {
	"author Vicent Marti <tanoku@gmail.com> 12345 +0200\n",
	"author Vicent Marti <tanoku@gmail.com> 12345 -0200\n",
	"author Vicent Marti <tanoku@gmail.com> 12345 +0000\n",
	"author Vicent Marti <tanoku@gmail.com> 12345 -0000\n",
	"author Vicent Marti <tanoku@gmail.com> 12345\n",
	"author Vicent Marti <tanoku@gmail.com>\n",
	"author Vicent Marti <tanoku@gmail.com> \n",
	"author Vicent Marti <tanoku@gmail.com>  \n",
	"author Vicent Marti <tanoku@gmail.com> x\n",
	"author Vicent Marti <tanoku@gmail.com> 12345 +2000\n",
	"author Vicent Marti <tanoku@gmail.com> 12345 +1460\n",
	"author Vicent Marti <tanoku@gmail.com> 12345 +1459\n",
	"author Vicent Marti <tanoku@gmail.com> 12345 -1459\n",
	"author Vicent Marti <tanoku@gmail.com> 12345 +-0130\n",
	"author Vicent Marti <tanoku@gmail.com> 12345 --0130\n",
	"author Vicent Marti <tanoku@gmail.com> 12345 -2147483648\n",
	"author Vicent Marti <tanoku@gmail.com> 12345 +2147483647\n",
	"author Vicent Marti <tanoku@gmail.com> 12345 +99999999999\n",
	"author Vicent Marti <tanoku@gmail.com> 12345 -+0130\n",
	"author Vicent Marti <tanoku@gmail.com> 12345 *0100\n",
	"author Vicent Marti <tanoku@gmail.com> 12345 +01\n",
	"author Vicent Marti <tanoku@gmail.com> 12345 +\n",
	"author Vicent Marti <tanoku@gmail.com> 12345 +abc\n",
	"author Vicent Marti <tanoku@gmail.com> 12345  +0100\n",
	"author Vicent Marti <tanoku@gmail.com> 9223372036854775807 +0100\n",
	"author Vicent Marti <tanoku@gmail.com> 9223372036854775808 +0100\n",
	"author Vicent Marti <tanoku@gmail.com> -9223372036854775808 +0100\n",
	"author Vicent Marti <tanoku@gmail.com> -5 +0100\n",
	"author Vicent Marti <tanoku@gmail.com>  12345 +0100\n",
	"author Vicent Marti <tanoku@gmail.com>12345 +0100\n",
	"author Vicent Marti <tanoku@gmail.com> 0x10 +0100\n",
	"author Vicent Marti <tanoku@gmail.com> 012 +0100\n",
	"author Vicent Marti <tanoku@gmail.com> \t12 +0100\n",
	"author   Vicent Marti   <  tanoku@gmail.com  > 12345 +0100\n",
	"author <> 12345 +0100\n",
	"author <a> 12345 +0100\n",
	"author a <> 12345 +0100\n",
	"author a b> 12345 +0100\n",
	"author a <b 12345 +0100\n",
	"author a >b< 12345 +0100\n",
	"author a <b> <c> 12345 +0100\n",
	"author a <b> c> 12345 +0100\n",
	"author a <<b>> 12345 +0100\n",
	"author \"a,b\"; <:c:> 1 +0100\n",
	"author '\\\\a\\\\' <\x01\x02e\x7f> 1 +0100\n",
	"author \xc3\xa9\xc3\xa8 <\xff\xfe> 1 +0100\n",
	"author a <b> 1 +0100",
	"authorx a <b> 1 +0100\n",
	"author\n",
	"author \n",
	"autho\n",
	"\n",
	"",
	"committer a <b> 1 +0100\nauthor c <d> 2 +0200\n",
	"author a <b> 1 +0100 extra stuff\n",
	"author a <b> 1 +0100\0hidden\n",
};

static const char *headers[] = { NULL, "author ", "committer ", "tagger ", "a", "" };
static const char enders[] = { '\n', 0, '>', ' ' };

static void signatures(void)
{
	size_t i, h, k, len, n;
	char buf[512];

	for (i = 0; i < ARRAY_SIZE(sig_corpus); i++) {
		const char *s = sig_corpus[i];
		size_t full = strlen(s);
		if (i == ARRAY_SIZE(sig_corpus) - 1)
			full = sizeof("author a <b> 1 +0100\0hidden\n") - 1; /* through the NUL */
		for (h = 0; h < ARRAY_SIZE(headers); h++)
			for (k = 0; k < ARRAY_SIZE(enders); k++)
				sig_case(s, full, headers[h], enders[k], 1);
		/* every truncation, the C's usual header and ender */
		for (len = 0; len <= full; len++) {
			sig_case(s, len, "author ", '\n', 0);
			sig_case(s, len, NULL, 0, 0);
		}
	}
	printf("sig truncations hash=%lx\n", hash_state);

	/* random mutations and random strings */
	{
		static const char alphabet[] = "<> +-0123456789abcAZ\n\t,;:\"'\\\x01\xff.";
		for (n = 0; n < 200000; n++) {
			size_t l;
			if (n % 2 == 0) {
				const char *s = sig_corpus[rnd() % ARRAY_SIZE(sig_corpus)];
				l = strlen(s);
				memcpy(buf, s, l);
				for (k = rnd() % 4; k > 0 && l > 0; k--)
					buf[rnd() % l] = alphabet[rnd() % (sizeof(alphabet) - 1)];
			} else {
				l = rnd() % 48;
				for (k = 0; k < l; k++)
					buf[k] = alphabet[rnd() % (sizeof(alphabet) - 1)];
			}
			sig_case(buf, l, (n % 3) ? "author " : NULL, (n % 5) ? '\n' : 0, 0);
		}
	}
	printf("sig fuzz hash=%lx\n", hash_state);

	/* git_signature_from_buffer */
	for (i = 0; i < ARRAY_SIZE(sig_corpus); i++) {
		git_signature *sig = (git_signature *)0x1;
		char e[600];
		int rc;
		const char *s = sig_corpus[i];
		if (!strncmp(s, "author ", 7))
			s += 7;
		rc = git_signature_from_buffer(&sig, s);
		err_line(e, sizeof(e));
		printf("from_buffer %zu rc=%d out_null=%d", i, rc, sig == NULL);
		if (!rc)
			printf(" name=\"%s\" email=\"%s\" time=%" PRId64 " off=%d sign=%d",
				sig->name, sig->email, (int64_t)sig->when.time, sig->when.offset, sig->when.sign);
		printf(" %s\n", e);
		git_signature_free(sig == (git_signature *)0x1 ? NULL : sig);
	}

	/* git_signature_new and git_signature__writebuf */
	{
		static const char *names[] = { "a", " a ", "<a", "a>", "", " ", ",;:", "\"x\"", "x\\", "\x01y\x7f", "\xc3\xa9", "na me", "'q'" };
		static const int offsets[] = { 0, 60, -60, 90, -90, 1, -1, 840, 900, -900, 6000, INT_MAX, INT_MIN, INT_MIN + 1 };
		static const int64_t times[] = { 0, 1, -1, 1234567890, 4294967296LL, -4294967297LL, INT64_MAX, INT64_MIN };
		size_t a, b, t, o;
		for (a = 0; a < ARRAY_SIZE(names); a++)
			for (b = 0; b < ARRAY_SIZE(names); b++) {
				git_signature *sig = NULL;
				char e[600];
				int rc = git_signature_new(&sig, names[a], names[b], 1, -30);
				err_line(e, sizeof(e));
				printf("new \"%s\" \"%s\" rc=%d", names[a], names[b], rc);
				if (!rc)
					printf(" -> \"%s\" \"%s\" sign=%c", sig->name, sig->email, sig->when.sign);
				printf(" %s\n", e);
				git_signature_free(sig);
			}
		for (t = 0; t < ARRAY_SIZE(times); t++)
			for (o = 0; o < ARRAY_SIZE(offsets); o++) {
				int s;
				for (s = 0; s < 3; s++) {
					git_signature sig;
					git_str out = GIT_STR_INIT;
					sig.name = "N";
					sig.email = "e@x";
					sig.when.time = times[t];
					sig.when.offset = offsets[o];
					sig.when.sign = s == 0 ? '+' : s == 1 ? '-' : 0;
					git_signature__writebuf(&out, (o & 1) ? "tagger " : NULL, &sig);
					printf("writebuf size=%zu asize=%zu \"", out.size, out.asize);
					print_quoted(out.ptr, out.size);
					printf("\"\n");
					git_str_dispose(&out);
				}
			}
		{
			git_signature x = { "a", "b", { 1, -0, '-' } }, y = { "a", "b", { 1, 0, '+' } };
			printf("equal %d %d\n", git_signature__equal(&x, &y), git_signature__equal(&x, &x));
		}
	}
}

/* ------------------------------------------------------------------ */
/* tags */

static void tag_case(const char *buf, size_t len, int verbose)
{
	git_object *obj = (git_object *)0x1;
	git_tag *tag;
	char e[600];
	int rc, valid = -1, rc2;

	git_error_clear();
	rc = git_object__from_raw(&obj, buf, len, GIT_OBJECT_TAG, GIT_OID_SHA1);
	err_line(e, sizeof(e));
	rc2 = git_object_rawcontent_is_valid(&valid, buf, len, GIT_OBJECT_TAG, GIT_OID_SHA1);
	git_error_clear();
	tag = (git_tag *)obj;

	if (verbose) {
		printf("tag in=\"");
		print_quoted(buf, len);
		printf("\" rc=%d valid=%d/%d out_null=%d", rc, rc2, valid, obj == NULL);
		if (rc == 0) {
			char a[GIT_OID_MAX_HEXSIZE + 1], b[GIT_OID_MAX_HEXSIZE + 1];
			printf(" id=%s type=%d target=%s name=\"%s\"",
				git_oid_tostr(a, sizeof(a), git_object_id(obj)), tag->type,
				git_oid_tostr(b, sizeof(b), &tag->target), tag->tag_name);
			if (tag->tagger)
				printf(" tagger=\"%s\" <%s> %" PRId64 " %d %d", tag->tagger->name,
					tag->tagger->email, (int64_t)tag->tagger->when.time,
					tag->tagger->when.offset, tag->tagger->when.sign);
			else
				printf(" tagger=NULL");
			if (tag->message) {
				printf(" msg=\"");
				print_quoted(tag->message, strlen(tag->message));
				printf("\"");
			} else
				printf(" msg=NULL");
			printf(" accessors: %s %s %d", git_tag_name(tag), git_oid_tostr_s(git_tag_target_id(tag)), git_tag_target_type(tag));
		}
		printf(" %s\n", e);
	} else {
		h_int(rc);
		h_int(rc2);
		h_int(valid);
		h_str(e);
		if (rc == 0) {
			h_bytes(git_object_id(obj), sizeof(git_oid));
			h_int(tag->type);
			h_bytes(&tag->target, sizeof(git_oid));
			h_str(tag->tag_name);
			h_int(tag->tagger != NULL);
			if (tag->tagger) {
				h_str(tag->tagger->name);
				h_str(tag->tagger->email);
				h_int(tag->tagger->when.time);
				h_int(tag->tagger->when.offset);
				h_int(tag->tagger->when.sign);
			}
			h_str(tag->message);
		}
	}
	if (rc == 0)
		git_object_free(obj);
}

static const char *tag_corpus[] = {
	"object 0123456789012345678901234567890123456789\ntype commit\ntag v1.0\ntagger Joe <joe@x> 1288977662 +0100\n\nmessage\n",
	"object 0123456789012345678901234567890123456789\ntype tree\ntag v1.0\ntagger Joe <joe@x> 1288977662 +0100\n\n",
	"object 0123456789012345678901234567890123456789\ntype blob\ntag v1.0\n\nno tagger\n",
	"object 0123456789012345678901234567890123456789\ntype tag\ntag v1.0\n",
	"object 0123456789012345678901234567890123456789\ntype tag\ntag v1.0\n\n",
	"object 0123456789012345678901234567890123456789\ntype commit\ntag \ntagger Joe <joe@x> 1 +0100\n\nm",
	"object 0123456789012345678901234567890123456789\ntype commit\ntag v\ntagger Joe <joe@x> 1 +0100\nextra header\n\nmsg\n",
	"object 0123456789012345678901234567890123456789\ntype commit\ntag v\ntagger Joe <joe@x> 1 +0100\nextra header\n",
	"object 0123456789012345678901234567890123456789\ntype commit\ntag v\ntagger Joe joe@x 1 +0100\n\nmsg\n",
	"object 0123456789012345678901234567890123456789\ntype commit\ntag v\nfoo Joe <joe@x> 1 +0100\n\nmsg\n",
	"object 0123456789012345678901234567890123456789\ntype commi\ntag v\n\nmsg\n",
	"object 0123456789012345678901234567890123456789\ntype commitx\ntag v\n\nmsg\n",
	"object 0123456789012345678901234567890123456789\ntype  commit\ntag v\n\nmsg\n",
	"object 0123456789012345678901234567890123456789\ntype commit\ntag  v\n\nmsg\n",
	"object 0123456789012345678901234567890123456789\ntype commit\ntag v1\0x\n\nmsg\0more\n",
	"object 0123456789012345678901234567890123456789\ntype commit\ntagv\n\nmsg\n",
	"object 0123456789012345678901234567890123456789\ntype commit\nTag v\n\nmsg\n",
	"object 012345678901234567890123456789012345678\ntype commit\ntag v\n\nmsg\n",
	"object 0123456789012345678901234567890123456789x\ntype commit\ntag v\n\nmsg\n",
	"object 012345678901234567890123456789012345678g\ntype commit\ntag v\n\nmsg\n",
	"object  0123456789012345678901234567890123456789\ntype commit\ntag v\n\nmsg\n",
	"objec 0123456789012345678901234567890123456789\ntype commit\ntag v\n\nmsg\n",
	"object 0123456789012345678901234567890123456789\ntyp commit\ntag v\n\nmsg\n",
	"object 0123456789012345678901234567890123456789\n",
	"object 0123456789012345678901234567890123456789\ntype",
	"object 0123456789012345678901234567890123456789\ntype ",
	"object 0123456789012345678901234567890123456789\ntype tree\n",
	"object 0123456789012345678901234567890123456789\ntype tree\ntag",
	"object 0123456789012345678901234567890123456789\ntype tree\ntag ",
	"object 0123456789012345678901234567890123456789\ntype tree\ntag v",
	"",
};

static char *read_file(const char *path, size_t *len)
{
	FILE *f = fopen(path, "rb");
	char *buf;
	long n;
	if (!f)
		return NULL;
	fseek(f, 0, SEEK_END);
	n = ftell(f);
	fseek(f, 0, SEEK_SET);
	buf = malloc(n + 1);
	if (fread(buf, 1, n, f) != (size_t)n) {
		fclose(f);
		free(buf);
		return NULL;
	}
	fclose(f);
	buf[n] = 0;
	*len = (size_t)n;
	return buf;
}

static void tags(void)
{
	size_t i, len, n, k;
	char buf[1024];

	for (i = 0; i < ARRAY_SIZE(tag_corpus); i++) {
		size_t full = strlen(tag_corpus[i]);
		if (i == 14)
			full = sizeof("object 0123456789012345678901234567890123456789\ntype commit\ntag v1\0x\n\nmsg\0more\n") - 1;
		tag_case(tag_corpus[i], full, 1);
		for (len = 0; len < full; len++)
			tag_case(tag_corpus[i], len, 0);
	}
	printf("tag truncations hash=%lx\n", hash_state);

	{
		size_t flen = 0;
		char *f = read_file("fuzzers/corpora/objects/tag", &flen);
		if (f) {
			tag_case(f, flen, 1);
			for (len = 0; len < flen; len++)
				tag_case(f, len, 0);
			for (n = 0; n < 20000; n++) {
				char *m = malloc(flen ? flen : 1);
				memcpy(m, f, flen);
				for (k = 1 + rnd() % 3; k > 0 && flen; k--)
					m[rnd() % flen] = "\n <>+-0aZ\0t"[rnd() % 11];
				tag_case(m, flen, 0);
				free(m);
			}
			free(f);
		} else
			printf("no fuzzers corpus\n");
	}
	printf("tag corpus hash=%lx\n", hash_state);

	for (n = 0; n < 100000; n++) {
		const char *s = tag_corpus[rnd() % (ARRAY_SIZE(tag_corpus) - 1)];
		size_t l = strlen(s);
		memcpy(buf, s, l);
		for (k = 1 + rnd() % 3; k > 0; k--) {
			size_t at = rnd() % l;
			switch (rnd() % 3) {
			case 0: buf[at] = "\n <>+-0aZ\0tg"[rnd() % 12]; break;
			case 1: if (l < sizeof(buf) - 1) { memmove(buf + at + 1, buf + at, l - at); buf[at] = "\n <>"[rnd() % 4]; l++; } break;
			default: memmove(buf + at, buf + at + 1, l - at - 1); l--; break;
			}
			if (!l)
				break;
		}
		tag_case(buf, l, 0);
	}
	printf("tag fuzz hash=%lx\n", hash_state);
}

/* ------------------------------------------------------------------ */
/* grafts */

static void graft_case(const char *buf, size_t len)
{
	git_grafts *g = NULL;
	git_oid *oids = NULL;
	size_t n = 0, i;
	char e[600];
	int rc;

	git_grafts_new(&g, GIT_OID_SHA1);
	rc = git_grafts_parse(g, buf, len);
	err_line(e, sizeof(e));
	printf("grafts in=\"");
	print_quoted(buf, len);
	printf("\" rc=%d size=%zu %s", rc, git_grafts_size(g), e);
	git_grafts_oids(&oids, &n, g);
	for (i = 0; i < n; i++) {
		git_commit_graft *c = NULL;
		size_t p;
		git_grafts_get(&c, g, &oids[i]);
		printf(" [%s", git_oid_tostr_s(&oids[i]));
		for (p = 0; p < git_array_size(c->parents); p++)
			printf(" %.7s", git_oid_tostr_s(git_array_get(c->parents, p)));
		printf("]");
	}
	printf("\n");
	git__free(oids);
	git_grafts_free(g);
}

static void grafts(void)
{
	static const char *A = "d7224d49d6d5aff6ade596ed74f4bcd4f77b29e2";
	static const char *B = "8cda5e8e1d9d2e10e1e10c3d9a2a3a4d5e6f7081";
	static const char *C = "0123456789abcdef0123456789abcdef01234567";
	char buf[1024];
	size_t i, l;
	const char *forms[] = {
		"%s\n", "%s", "%s %s\n", "%s %s %s\n", "%s\n%s\n%s\n", "%s  %s\n", "%s %s \n",
		"%s\n\n", "\n", "", "%sx\n", "%s %sx\n", "%s\t%s\n", " %s\n", "%s %s\n%s %s\n",
	};
	for (i = 0; i < ARRAY_SIZE(forms); i++) {
		snprintf(buf, sizeof(buf), forms[i], A, B, C, A);
		graft_case(buf, strlen(buf));
		for (l = 0; l < strlen(buf); l += 7)
			graft_case(buf, l);
	}
	/* many grafts: the khash order of git_grafts_oids */
	{
		git_str s = GIT_STR_INIT;
		for (i = 0; i < 300; i++) {
			git_oid o;
			unsigned char raw[20];
			size_t j;
			for (j = 0; j < 20; j++)
				raw[j] = rnd();
			git_oid_from_raw(&o, raw, GIT_OID_SHA1);
			git_str_printf(&s, "%s%s%s\n", git_oid_tostr_s(&o), i % 2 ? " " : "", i % 2 ? A : "");
		}
		graft_case(s.ptr, s.size);
		git_str_dispose(&s);
	}
}

/* ------------------------------------------------------------------ */
/* object types, ids, headers */

static void objects(void)
{
	static const char *names[] = { "commit", "tree", "blob", "tag", "commitx", "comm", "", "Blob", "tagged", "t", "ofs-delta", "any" };
	size_t i, l;
	int t;
	for (t = -3; t <= 8; t++)
		printf("type2string %d \"%s\" valid=%d\n", t, git_object_type2string(t), git_object_type_is_valid(t));
	for (i = 0; i < ARRAY_SIZE(names); i++) {
		printf("string2type \"%s\" %d", names[i], git_object_string2type(names[i]));
		for (l = 0; l <= strlen(names[i]) + 1; l++)
			printf(" %d", git_object_stringn2type(names[i], l));
		printf("\n");
	}
	printf("string2type NULL %d\n", git_object_string2type(NULL));

	for (t = -3; t <= 8; t++) {
		int o;
		for (o = 0; o <= 3; o++) {
			git_object_id_options opts = GIT_OBJECT_ID_OPTIONS_INIT;
			git_oid id;
			char e[600];
			int rc;
			opts.object_type = t;
			opts.oid_type = o;
			memset(&id, 0x5a, sizeof(id));
			rc = git_object_id_from_buffer(&id, "hello\n", 6, &opts);
			err_line(e, sizeof(e));
			printf("id type=%d oid=%d rc=%d %s %s\n", t, o, rc, rc ? "-" : git_oid_tostr_s(&id), e);
		}
	}
	{
		git_oid id;
		char e[600];
		int rc = git_object_id_from_buffer(&id, NULL, 3, NULL);
		err_line(e, sizeof(e));
		printf("id NULL data rc=%d %s\n", rc, e);
		rc = git_object_id_from_buffer(&id, NULL, 0, NULL);
		printf("id NULL empty rc=%d %s\n", rc, git_oid_tostr_s(&id));
	}

	/* oid headers */
	{
		static const char *in[] = {
			"tree 0123456789012345678901234567890123456789\n",
			"tree 0123456789012345678901234567890123456789",
			"tree 012345678901234567890123456789012345678\n",
			"tree 0123456789012345678901234567890123456789x",
			"tree 012345678901234567890123456789012345678g\n",
			"tre 0123456789012345678901234567890123456789\n",
			"tree \n",
			"",
		};
		for (i = 0; i < ARRAY_SIZE(in); i++) {
			int o;
			for (o = 0; o <= 3; o++) {
				git_oid id;
				const char *p = in[i];
				char e[600];
				int rc;
				git_error_clear();
				rc = git_object__parse_oid_header(&id, &p, in[i] + strlen(in[i]), "tree ", o);
				err_line(e, sizeof(e));
				printf("oid_header %zu oid=%d rc=%d used=%ld %s %s\n", i, o, rc, (long)(p - in[i]),
					rc ? "-" : git_oid_tostr_s(&id), e);
			}
		}
		for (i = 0; i < 3; i++) {
			git_oid id;
			git_str out = GIT_STR_INIT;
			char e[600];
			int rc;
			memset(&id, 0x11, sizeof(id));
			id.type = (unsigned char)i;
			rc = git_object__write_oid_header(&out, "parent ", &id);
			err_line(e, sizeof(e));
			printf("write_oid_header type=%zu rc=%d \"", i, rc);
			print_quoted(out.ptr, out.size);
			printf("\" %s\n", e);
			git_str_dispose(&out);
		}
	}

	/* raw validity of other types */
	{
		int valid = -1, rc;
		rc = git_object_rawcontent_is_valid(&valid, "x", 1, GIT_OBJECT_BLOB, GIT_OID_SHA1);
		printf("valid blob rc=%d valid=%d\n", rc, valid);
		rc = git_object_rawcontent_is_valid(&valid, "x", 1, GIT_OBJECT_ANY, GIT_OID_SHA1);
		printf("valid any rc=%d valid=%d\n", rc, valid);
		git_error_clear();
	}
}

int main(void)
{
	git_libgit2_init();
	signatures();
	tags();
	grafts();
	objects();
	printf("final hash=%lx\n", hash_state);
	git_libgit2_shutdown();
	return 0;
}
