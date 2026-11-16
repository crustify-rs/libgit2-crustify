/* From the u2-r1 review. */
#include "git2_util.h"
#include "git2/global.h"
#include "wildmatch.h"
static unsigned long seed = 777;
static unsigned rnd(void) { seed = seed * 6364136223846793005UL + 1442695040888963407UL; return (unsigned)(seed >> 33); }
static const char *pt[] = {"*","**","?","[","]","[!","[^","-","\\","/","a","A","b","Z","z","[:alpha:]","[:upper:]","[:bogus:]","[:","]:",":","\xc3","\xa9","\x80","0","9","*/","/**/","**/","/**","x"};
static const char *tt[] = {"a","A","b","B","z","Z","/","-","[","]","\\","\xc3","\xa9","0","9",":","*","?","x","foo","bar/"," "};
int main(int argc, char **argv)
{
	int n = argc > 1 ? atoi(argv[1]) : 300000, i;
	char p[128], t[128];
	for (i = 0; i < n; i++) {
		int k = rnd() % 7, j; size_t pl = 0, tl = 0;
		for (j = 0; j < k; j++) { const char *s = pt[rnd() % (sizeof(pt)/sizeof(*pt))]; size_t l = strlen(s); memcpy(p + pl, s, l); pl += l; }
		p[pl] = 0;
		k = rnd() % 9;
		for (j = 0; j < k; j++) { const char *s = tt[rnd() % (sizeof(tt)/sizeof(*tt))]; size_t l = strlen(s); memcpy(t + tl, s, l); tl += l; }
		t[tl] = 0;
		printf("%d %d %d %d\n", wildmatch(p, t, 0), wildmatch(p, t, WM_CASEFOLD), wildmatch(p, t, WM_PATHNAME), wildmatch(p, t, WM_PATHNAME|WM_CASEFOLD));
	}
	return 0;
}
