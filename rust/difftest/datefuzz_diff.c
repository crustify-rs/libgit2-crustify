/*
 * Differential fuzz of git_date_offset_parse (from the u2-r1 review):
 * random token strings under three time zones, "now" pinned.
 */
#include "git2_util.h"
#include "git2/global.h"
#include "git2/errors.h"
#include "date.h"
#include <time.h>

/* "now" pinned for both builds (the C's time() and the Rust's libc::time
 * resolve to this definition). From the u2-r1 review (fixtime.c). */
time_t time(time_t *t) { time_t v = 1700000000; if (t) *t = v; return v; }
static unsigned long seed = 12345;
static unsigned rnd(void) { seed = seed * 6364136223846793005UL + 1442695040888963407UL; return (unsigned)(seed >> 33); }
static const char *toks[] = { "0","1","2","9","12","13","24","31","32","59","60","99","100","1970","2000","2038","2147483647","4294967296","99999999999","-1","+0100","-0530","+1","-","/",".",":",",", " ", " ", "  ","\t",
 "jan","February","mar","sept","dec","mon","Tuesday","sun","am","pm","AM","PM","ago","days","day","weeks","months","years","hours","minutes","seconds","yesterday","today","now","noon","midnight","tea","last","never","UTC","GMT","EST","PST","CEST","Z","T","@","@1234567890","th","st","nd","rd","x","!", "1.2.3", "2005-04-07", "22:13:13", "Thu, 07 Apr 2005 22:13:13 +0200", "10/12/2023", "12.10.2023", "1 week ago", "2 months", "08", "0x10", "1e5" };
int main(int argc, char **argv)
{
	static const char *zones[] = { "UTC", "America/New_York", "Asia/Kolkata" };
	int n = argc > 1 ? atoi(argv[1]) : 100000, i, z;
	char buf[512];
	git_libgit2_init();
	for (z = 0; z < 3; z++) {
	setenv("TZ", zones[z], 1);
	tzset();
	printf("== TZ=%s\n", zones[z]);
	for (i = 0; i < n; i++) {
		int k = 1 + rnd() % 8, j; size_t len = 0;
		buf[0] = 0;
		for (j = 0; j < k; j++) {
			const char *t = toks[rnd() % (sizeof(toks)/sizeof(toks[0]))];
			size_t l = strlen(t);
			if (len + l + 2 >= sizeof(buf)) break;
			memcpy(buf + len, t, l); len += l;
			if (rnd() % 3) buf[len++] = ' ';
			buf[len] = 0;
		}
		{
			git_time_t out = -42; int off = -4242, rc;
			rc = git_date_offset_parse(&out, &off, buf);
			printf("[%s] rc=%d out=%lld off=%d\n", buf, rc, (long long)out, off);
		}
	}
	}
	return 0;
}
