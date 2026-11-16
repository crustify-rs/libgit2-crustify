/* difftest-link: libgit2_tests
 * Tag parsing with the buffer's end against a PROT_NONE guard page, every
 * truncation of a corpus, SHA1 and SHA256, from_raw and rawcontent_is_valid. */
#include "common.h"
#include "git2.h"
#include "object.h"
#include "tag.h"
#include <sys/mman.h>

static char *page;
static size_t psz;

static void pq(const char *s, size_t n) { size_t i; for (i = 0; i < n; i++) { unsigned char c = s[i]; if (c >= 32 && c < 127 && c != '\\') putchar(c); else printf("\\x%02x", c); } }

static void one(const char *buf, size_t len, git_oid_t ot)
{
	char *p = page + psz - len;
	git_object *o = NULL;
	const git_error *e;
	int rc, valid = -1, rc2 = 0;
	memcpy(p, buf, len);
	git_error_clear();
	rc = git_object__from_raw(&o, p, len, GIT_OBJECT_TAG, ot);
	e = git_error_last();
	printf("ot=%d len=%zu rc=%d class=%d msg=%s", ot, len, rc, e ? e->klass : -1, e ? e->message : "-");
	if (rc == 0) {
		git_tag *t = (git_tag *)o;
		printf(" type=%d target=%s name=", t->type, git_oid_tostr_s(&t->target));
		pq(t->tag_name, strlen(t->tag_name));
		if (t->tagger) printf(" tagger=%s|%s|%lld|%d|%c", t->tagger->name, t->tagger->email, (long long)t->tagger->when.time, t->tagger->when.offset, t->tagger->when.sign);
		else printf(" tagger=NULL");
		if (t->message) { printf(" msg="); pq(t->message, strlen(t->message)); } else printf(" msg=NULL");
		git_object_free(o);
	}
	{
		git_error_clear();
		rc2 = git_object_rawcontent_is_valid(&valid, p, len, GIT_OBJECT_TAG, ot);
		e = git_error_last();
		printf(" | valid rc=%d v=%d msg=%s", rc2, valid, e ? e->message : "-");
	}
	printf("\n");
}

int main(void)
{
	static const char *corpus[] = {
		"object %s\ntype commit\ntag v1.0\ntagger Joe <joe@x> 1288977662 +0100\n\nmessage\n",
		"object %s\ntype tag\ntag v\ntagger Joe <joe@x> 1288977662 -0130\nextra\nmore\n\nm\n\nm2",
		"object %s\ntype blob\ntag v\n\n",
		"object %s\ntype tree\ntag v\n\n\n\n",
		"object %s\ntype tree\ntag v\ntagger <> 0 +0000\n\n",
		"object %s\ntype tree\ntag v\ntagger J <j> \n\n",
		"object %s\ntype tree\ntag v\ntagger J <j>\n\n",
		"object %s\ntype tree\ntag v\ntagger J <j> 99999999999999999999 +9999\n\n",
		"object %s\ntype tree\ntag v\ntagger J <j\n\n",
		"object %s\ntype tree\ntag v\ntagger\n\n",
		"object %s\ntype tree\ntag v\ntagger \n\n",
		"object %s\ntype tree\ntag v\r\ntagger J <j> 1 +0000\r\n\r\nm\r\n",
		"object %s\ntype commit\ntag v\ntagger J <j> 1 +0000\n",
		"object %s\ntype commit\ntag v\ntagger J <j> 1 +0000",
		NULL };
	static const char *sha1 = "0123456789012345678901234567890123456789";
	static const char *sha256 = "0123456789012345678901234567890123456789012345678901234567890123";
	char buf[1024];
	size_t i, l;
	int t;
	git_libgit2_init();
	psz = sysconf(_SC_PAGESIZE);
	page = mmap(NULL, 2 * psz, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	mprotect(page + psz, psz, PROT_NONE);
	for (t = 0; t < 4; t++) {
		/* t=0: sha1 ids, sha1 type; 1: sha256 ids, sha256 type; 2: sha1 ids, sha256 type; 3: sha256 ids, sha1 type */
		const char *id = (t == 1 || t == 3) ? sha256 : sha1;
		git_oid_t ot = (t == 1 || t == 2) ? GIT_OID_SHA256 : GIT_OID_SHA1;
		for (i = 0; corpus[i]; i++) {
			snprintf(buf, sizeof(buf), corpus[i], id);
			for (l = 0; l <= strlen(buf); l++)
				one(buf, l, ot);
		}
	}
	/* embedded NULs */
	{
		const char nb[] = "object 0123456789012345678901234567890123456789\ntype commit\ntag v\0x\ntagger J\0 <j> 1 +0000\n\nm\0sg";
		for (l = 0; l <= sizeof(nb) - 1; l++) one(nb, l, GIT_OID_SHA1);
	}
	/* invalid oid types */
	one("object 0123456789012345678901234567890123456789\ntype commit\ntag v\n\nm", 60, 0);
	one("object 0123456789012345678901234567890123456789\ntype commit\ntag v\n\nm", 60, 3);
	git_libgit2_shutdown();
	return 0;
}
