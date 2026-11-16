/* difftest-link: libgit2_tests
 * usage: tagapi_diff <path to a scratch copy of testrepo.git> */
#include "common.h"
#include "git2.h"
#include "git2/sys/repository.h"
#include "tag.h"

static git_repository *repo;
static char saved[1024];
static void S(void) { const git_error *e = git_error_last(); snprintf(saved, sizeof(saved), "class=%d msg=%s", e ? e->klass : -1, e ? e->message : "(none)"); }
static void E(const char *what, int rc)
{
	const git_error *e = git_error_last();
	if (saved[0]) { printf("%s: rc=%d %s\n", what, rc, saved); saved[0] = 0; }
	else printf("%s: rc=%d class=%d msg=%s\n", what, rc, e ? e->klass : -1, e ? e->message : "(none)");
	git_error_clear();
}
static void show_ref(const char *name)
{
	git_reference *r = NULL;
	int rc = git_reference_lookup(&r, repo, name);
	if (rc) { E(name, rc); return; }
	printf("  ref %s -> %s\n", name, git_oid_tostr_s(git_reference_target(r)));
	git_reference_free(r);
}
static void show_obj(const git_oid *id)
{
	git_odb *odb; git_odb_object *o;
	if (git_repository_odb(&odb, repo)) { E("odb", -1); return; }
	if (git_odb_read(&o, odb, id) == 0) {
		printf("  obj %s type=%d len=%zu\n<<", git_oid_tostr_s(id), git_odb_object_type(o), git_odb_object_size(o));
		fwrite(git_odb_object_data(o), 1, git_odb_object_size(o), stdout);
		printf(">>\n");
		git_odb_object_free(o);
	} else E("  obj read", -1);
	git_odb_free(odb);
}
static void list(const char *pat)
{
	git_strarray a = { (char **)0x1, 7 };
	size_t i;
	int rc = pat ? git_tag_list_match(&a, pat, repo) : git_tag_list(&a, repo);
	printf("list(%s) rc=%d count=%zu strings=%s:", pat ? pat : "ALL", rc, a.count, a.strings ? "ptr" : "NULL");
	for (i = 0; i < a.count; i++) printf(" %s", a.strings[i]);
	printf("\n");
	git_error_clear();
	git_strarray_dispose(&a);
}

static int mode, calls;
static int fcb(const char *name, git_oid *oid, void *p)
{
	int rc;
	(void)p;
	calls++;
	printf("  cb %s %s\n", name, git_oid_tostr_s(oid));
	switch (mode) {
	case 0: return 0;
	case 1: return calls == 2 ? 7 : 0;
	case 2: return calls == 2 ? -7 : 0;
	case 3: if (calls == 2) { git_error_set_str(GIT_ERROR_NET, "user err"); return -12; } return 0;
	case 4: /* delete every tag on the first call */
		if (calls == 1) {
			git_strarray a; size_t i;
			git_tag_list(&a, repo);
			for (i = 0; i < a.count; i++) { rc = git_tag_delete(repo, a.strings[i]); printf("   del %s rc=%d\n", a.strings[i], rc); git_error_clear(); }
			git_strarray_dispose(&a);
		}
		return 0;
	case 5: /* create a tag during iteration */
		if (calls == 1) {
			git_oid o; git_object *t;
			git_revparse_single(&t, repo, "HEAD");
			git_error_clear(); rc = git_tag_create_lightweight(&o, repo, "zz-new", t, 0); S();
			printf("   create rc=%d\n", rc);
			git_object_free(t);
		}
		return 0;
	case 6: return GIT_EUSER;
	}
	return 0;
}
static void foreach(int m)
{
	int rc;
	mode = m; calls = 0;
	rc = git_tag_foreach(repo, fcb, NULL);
	printf("foreach mode=%d calls=%d ", m, calls);
	E("rc", rc);
}

int main(int argc, char **argv)
{
	git_oid oid, oid2;
	git_object *head, *tree, *blob, *other_obj = NULL, *tagobj;
	git_signature *sig;
	git_repository *other = NULL;
	git_tag *tag;
	int rc, v;
	const char *names[] = { "v1", "-x", "HEAD", "", "a..b", "a/b", "foo.lock", "x y", "refs/tags/z", "@", "a@{b", "ok/../x", "HEADx", "nul\x01", NULL };
	size_t i;
	char buf[4096];

	git_libgit2_init();
	if (git_repository_open(&repo, argv[1])) { E("open", -1); return 1; }
	git_signature_new(&sig, "Tagger Name", "tagger@ex.com", 1234567890, 60);
	git_revparse_single(&head, repo, "HEAD");
	git_revparse_single(&tree, repo, "HEAD^{tree}");
	git_revparse_single(&blob, repo, "HEAD:README");
	if (argc > 2 && git_repository_open(&other, argv[2]) == 0)
		git_revparse_single(&other_obj, other, "HEAD");

	list(NULL); list("e*"); list("*"); list("["); list("v0.*"); list("**/*"); list("e90810b");

	for (i = 0; names[i]; i++) {
		rc = git_tag_name_is_valid(&v, names[i]);
		printf("name_is_valid(%s) rc=%d v=%d\n", names[i], rc, v);
		git_error_clear();
	}
	rc = git_tag_name_is_valid(&v, NULL); printf("name_is_valid(NULL) rc=%d v=%d\n", rc, v);

	/* annotated */
	for (i = 0; names[i]; i++) {
		memset(&oid, 0xab, sizeof(oid)); oid.type = GIT_OID_SHA1;
		git_error_clear(); rc = git_tag_create(&oid, repo, names[i], head, sig, "a message\n", 0); S();
		snprintf(buf, sizeof(buf), "create(%s) oid=%s", names[i], git_oid_tostr_s(&oid));
		E(buf, rc);
	}
	show_ref("refs/tags/v1");
	memset(&oid, 0xab, sizeof(oid)); oid.type = GIT_OID_SHA1;
	git_error_clear(); rc = git_tag_create(&oid, repo, "e90810b", head, sig, "m", 0); S();
	snprintf(buf, sizeof(buf), "create existing oid=%s", git_oid_tostr_s(&oid)); E(buf, rc);
	git_error_clear(); rc = git_tag_create(&oid, repo, "e90810b", tree, sig, "msg no nl", 1); S();
	snprintf(buf, sizeof(buf), "create force oid=%s", git_oid_tostr_s(&oid)); E(buf, rc);
	show_ref("refs/tags/e90810b"); show_obj(&oid);
	git_error_clear(); rc = git_tag_create(&oid, repo, "v1", blob, sig, "", 1); S(); E("create blob empty msg", rc); show_obj(&oid);
	git_error_clear(); rc = git_tag_create(&oid, repo, "v2", head, sig, NULL, 0); S(); E("create NULL msg", rc);
	git_error_clear(); rc = git_tag_create(&oid, repo, "v2", head, NULL, "m", 0); S(); E("create NULL tagger", rc);
	git_error_clear(); rc = git_tag_create(&oid, repo, NULL, head, sig, "m", 0); S(); E("create NULL name", rc);
	git_error_clear(); rc = git_tag_create(&oid, repo, "v2", NULL, sig, "m", 0); S(); E("create NULL target", rc);
	if (other_obj) { git_error_clear(); rc = git_tag_create(&oid, repo, "v3", other_obj, sig, "m", 0); S(); E("create foreign", rc); }
	/* tag of a tag */
	git_object_lookup(&tagobj, repo, &oid, GIT_OBJECT_TAG);
	git_error_clear(); rc = git_tag_create(&oid2, repo, "tagoftag", tagobj, sig, "m\n", 0); S(); E("create tag of tag", rc); show_obj(&oid2);
	{
		git_tag *t2; git_object *tgt, *peeled;
		git_tag_lookup(&t2, repo, &oid2);
		rc = git_tag_target(&tgt, t2); printf("target rc=%d type=%d\n", rc, rc ? -1 : git_object_type(tgt)); if (!rc) git_object_free(tgt);
		rc = git_tag_peel(&peeled, t2); printf("peel rc=%d type=%d %s\n", rc, rc ? -1 : git_object_type(peeled), rc ? "" : git_oid_tostr_s(git_object_id(peeled))); if (!rc) git_object_free(peeled);
		printf("name=%s msg=%s tagger=%s\n", git_tag_name(t2), git_tag_message(t2), git_tag_tagger(t2)->name);
		git_tag_free(t2);
	}
	git_object_free(tagobj);
	/* weird signature contents */
	{
		git_signature s2 = { "<na\nme>", "em<a>il", { 0, -90, '-' } };
		git_error_clear(); rc = git_tag_create(&oid, repo, "weird", head, &s2, "x\0y", 0); S(); E("create weird sig", rc); if (!rc) show_obj(&oid);
		s2.when.offset = 0; s2.when.sign = '-';
		rc = git_tag_annotation_create(&oid, repo, "anno", head, &s2, "anno msg"); E("annotation_create", rc); if (!rc) show_obj(&oid);
		show_ref("refs/tags/anno");
		rc = git_tag_annotation_create(&oid, repo, "-bad", head, &s2, ""); E("annotation_create -bad", rc);
		rc = git_tag_annotation_create(&oid, repo, "HEAD", head, &s2, ""); E("annotation_create HEAD", rc);
		rc = git_tag_annotation_create(NULL, repo, "a", head, &s2, ""); E("annotation_create NULL oid", rc);
		if (other_obj) { rc = git_tag_annotation_create(&oid, repo, "anno2", other_obj, &s2, ""); E("annotation_create foreign", rc); if (!rc) show_obj(&oid); }
	}
	/* lightweight */
	for (i = 0; names[i]; i++) {
		memset(&oid, 0xab, sizeof(oid)); oid.type = GIT_OID_SHA1;
		git_error_clear(); rc = git_tag_create_lightweight(&oid, repo, names[i], tree, 0); S();
		snprintf(buf, sizeof(buf), "lightweight(%s) oid=%s", names[i], git_oid_tostr_s(&oid));
		E(buf, rc);
	}
	git_error_clear(); rc = git_tag_create_lightweight(&oid, repo, "v1", blob, 1); S(); E("lw force", rc); show_ref("refs/tags/v1");
	git_error_clear(); rc = git_tag_create_lightweight(&oid, repo, "v1/sub", blob, 1); S(); E("lw d/f conflict", rc);
	git_error_clear(); rc = git_tag_create_lightweight(&oid, repo, "e90810b/sub", blob, 0); S(); E("lw d/f conflict 2", rc);

	/* from buffer */
	{
		const char *hs = git_oid_tostr_s(git_object_id(head));
		char hex[64]; strcpy(hex, hs);
		const char *bufs[] = {
			"object %s\ntype commit\ntag fb1\ntagger A <a@b> 1 +0000\n\nmsg\n",
			"object %s\ntype tree\ntag fb2\ntagger A <a@b> 1 +0000\n\nmsg\n",
			"object %s\ntype commit\ntag fb3\n",
			"object %s\ntype commit\ntag e90810b\n\nm",
			"object 0000000000000000000000000000000000000001\ntype commit\ntag fb4\n\nm",
			"object %s\ntype commit\ntag -dash\n\nm",
			"object %s\ntype commit\ntag a..b\n\nm",
			"object %s\ntype commit\ntag HEAD\n\nm",
			"object %s\ntype commit\ntag \n\nm",
			"object %s\ntype commit\ntag fb5\ntagger bad\n\nm",
			"object %s\ntype commit\ntag fb6\ntagger A <a@b> 1 +0000\nno blank",
			"garbage",
			"",
			NULL };
		for (i = 0; bufs[i]; i++) {
			snprintf(buf, sizeof(buf), bufs[i], hex);
			memset(&oid, 0xab, sizeof(oid)); oid.type = GIT_OID_SHA1;
			git_error_clear(); rc = git_tag_create_from_buffer(&oid, repo, buf, 0); S();
			printf("from_buffer[%zu] oid=%s ", i, git_oid_tostr_s(&oid)); E("", rc);
		}
		snprintf(buf, sizeof(buf), bufs[3], hex);
		git_error_clear(); rc = git_tag_create_from_buffer(&oid, repo, buf, 1); S(); printf("from_buffer force oid=%s ", git_oid_tostr_s(&oid)); E("", rc);
		show_ref("refs/tags/e90810b");

	}

	list(NULL);
	/* delete */
	{
		const char *d[] = { "fb1", "fb1", "nope", "a..b", "", "e90810b/", "HEAD", NULL };
		for (i = 0; d[i]; i++) { rc = git_tag_delete(repo, d[i]); snprintf(buf, sizeof(buf), "delete(%s)", d[i]); E(buf, rc); }
	}
	list(NULL);
	/* lookup type mismatch */
	rc = git_tag_lookup(&tag, repo, git_object_id(head)); E("tag_lookup commit", rc);

	foreach(0); foreach(1); foreach(2); foreach(3); foreach(6);
	foreach(5); list(NULL);
	foreach(4); list(NULL);
	rc = git_tag_foreach(repo, NULL, NULL); E("foreach NULL cb", rc);

	git_object_free(head); git_object_free(tree); git_object_free(blob);
	git_object_free(other_obj); git_repository_free(other);
	git_signature_free(sig);
	git_repository_free(repo);
	git_libgit2_shutdown();
	return 0;
}
