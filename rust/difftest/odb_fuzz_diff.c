/*
 * 6000 randomly corrupted and truncated loose objects (from review
 * l-odb-r1, loose/ld3.c) through the loose backend's read, read_header and
 * read stream: codes, classes and zlib messages.
 *
 * difftest-link: libgit2_tests
 */
#include <unistd.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <zlib.h>
#include <git2.h>
#include <git2/sys/odb_backend.h>
#include <git2/sys/errors.h>
static unsigned long fnv(const unsigned char *p, size_t n){unsigned long h=1469598103934665603UL;while(n--){h^=*p++;h*=1099511628211UL;}return h;}
static void e(const char *w, int r){ const git_error *x = git_error_last(); printf(" %s %d", w, r); if (r<0) printf(" [%d] %s", x?x->klass:-1, x?x->message:"-"); printf("\n"); git_error_clear(); }
int main(void){
	git_odb_backend *be; git_oid id; char path[256]; unsigned char body[20000], z[30000], m[30000]; size_t bl, i; int it;
	unsigned int s = 1;
	git_libgit2_init();
	char dir[128], cmd[300]; snprintf(dir, sizeof(dir), "/tmp/odb_fuzz_diff.%d", (int)getuid()); snprintf(cmd, sizeof(cmd), "rm -rf %s; mkdir -p %s/01", dir, dir); if (system(cmd)) return 1;
	git_odb_backend_loose(&be, dir, NULL);
	git_oid_from_string(&id, "0123456789012345678901234567890123456789", GIT_OID_SHA1);
	snprintf(path, sizeof(path), "%s/01/23456789012345678901234567890123456789", dir);
	for (it = 0; it < 6000; it++) {
		uLongf zl = sizeof(z); size_t n, k, nflips; FILE *f; void *buf; size_t len; git_object_t t; int r;
		size_t sz = (it * 37) % 5000;
		bl = sprintf((char *)body, "%s %zu", (it % 4 == 0) ? "blob" : (it % 4 == 1) ? "tree" : (it % 4 == 2) ? "commit" : "tag", sz) + 1;
		for (i = 0; i < sz; i++) { s = s * 1103515245 + 12345; body[bl + i] = (it & 8) ? (s >> 16) : 'a' + (s >> 16) % 4; }
		compress2(z, &zl, body, bl + sz, it % 10);
		n = zl; memcpy(m, z, n);
		nflips = 1 + (it % 3);
		for (k = 0; k < nflips; k++) { s = s * 1103515245 + 12345; m[(s >> 8) % n] ^= (unsigned char)(1u << ((s >> 4) % 8)); }
		if (it % 7 == 0) { s = s * 1103515245 + 12345; n = (s >> 8) % n; }
		f = fopen(path, "wb"); fwrite(m, 1, n, f); fclose(f);
		printf("%d", it);
		r = be->read(&buf, &len, &t, be, &id); e("r", r); if (!r) { printf("  %zu %d %016lx\n", len, t, fnv(buf, len)); free(buf); }
		r = be->read_header(&len, &t, be, &id); e("h", r); if (!r) printf("  %zu %d\n", len, t);
		{ git_odb_stream *st; r = be->readstream(&st, &len, &t, be, &id); e("s", r);
		  if (!r) { char b[1000]; int q; unsigned long h = 0; size_t tot = 0; while ((q = st->read(st, b, sizeof b)) > 0) { h = h * 31 + fnv((unsigned char *)b, q); tot += q; } printf("  %zu %d -> %zu %d %lx\n", len, t, tot, q, h); if (q < 0) e("sr", q); free(st->hash_ctx); st->free(st); } }
	}
	be->free(be);
	snprintf(cmd, sizeof(cmd), "rm -rf %s", dir);
	return system(cmd) ? 1 : 0;
}
