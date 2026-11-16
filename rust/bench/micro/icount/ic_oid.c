#include "ic.h"
#include "tm.h"
int main(int argc, char **argv) {
	int n = argc > 1 ? atoi(argv[1]) : 100;
	int timing = argc > 2;
	git_oid ids[256]; char buf[64]; static char hex[12000][41];
	git_libgit2_init();
	for (int i = 0; i < 256; i++) { unsigned char raw[20]; for (int j = 0; j < 20; j++) raw[j] = (unsigned char)(i * 37 + j * 101 + (i*j)); git_oid_from_raw(&ids[i], raw, GIT_OID_SHA1); }
	for (int i = 0; i < 12000; i++) { git_oid o; unsigned char raw[20]; unsigned x = i * 2654435761u; for (int j = 0; j < 20; j++) { x = x * 1103515245u + 12345u; raw[j] = x >> 24; } git_oid_from_raw(&o, raw, GIT_OID_SHA1); git_oid_tostr(hex[i], 41, &o); }
	if (timing) {
		TIME_MIN("pathfmt", 15, 1000000, { git_oid_pathfmt(buf, &ids[_i & 255]); __asm__ volatile("" ::: "memory"); });
		TIME_MIN("fmt", 15, 1000000, { git_oid_fmt(buf, &ids[_i & 255]); __asm__ volatile("" ::: "memory"); });
		TIME_MIN("shorten 12000", 7, 1, { git_oid_shorten *os = git_oid_shorten_new(4); for (int k = 0; k < 12000; k++) CK(git_oid_shorten_add(os, hex[k])); git_oid_shorten_free(os); });
		return 0;
	}
	MARK(); for (int i = 0; i < n; i++) git_oid_pathfmt(buf, &ids[i & 255]); MARK();
	MARK(); for (int i = 0; i < n; i++) git_oid_fmt(buf, &ids[i & 255]); MARK();
	git_oid_shorten *os = git_oid_shorten_new(4);
	for (int k = 0; k < 3000; k++) CK(git_oid_shorten_add(os, hex[k]));
	MARK(); for (int k = 3000; k < 3000 + n; k++) CK(git_oid_shorten_add(os, hex[k])); MARK();
	git_oid_shorten_free(os);
	return 0;
}
