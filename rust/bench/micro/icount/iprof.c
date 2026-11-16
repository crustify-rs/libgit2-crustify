/* iprof: like icount, but also records rip of every step; at region end
 * writes prof.<region>.txt ("addr count" lines) and maps.<region>.txt */
#include <sys/ptrace.h>
#include <sys/user.h>
#include <sys/wait.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#define HS (1<<22)
static unsigned long long keys[HS]; static long long vals[HS];
static void add(unsigned long long k){unsigned long long h=(k*0x9E3779B97F4A7C15ull)>>42; while(keys[h]&&keys[h]!=k)h=(h+1)&(HS-1); keys[h]=k; vals[h]++;}
int main(int argc, char **argv) {
	pid_t pid = fork();
	if (pid == 0) { ptrace(PTRACE_TRACEME, 0, 0, 0); execvp(argv[1], argv + 1); _exit(127); }
	int st; waitpid(pid, &st, 0);
	int pending = 0; int stepping = 0, region = 0; long long n = 0;
	int want = getenv("REGION") ? atoi(getenv("REGION")) : 0;
	for (;;) {
		ptrace(stepping ? PTRACE_SINGLESTEP : PTRACE_CONT, pid, 0, pending); pending = 0;
		if (waitpid(pid, &st, 0) < 0 || WIFEXITED(st) || WIFSIGNALED(st)) break;
		int sig = WSTOPSIG(st);
		if (sig == SIGTRAP && stepping) { n++; if (!want || want == region + 1) { struct user_regs_struct r; ptrace(PTRACE_GETREGS, pid, 0, &r); add(r.rip);} continue; }
		if (sig == SIGSTOP) {
			if (stepping) {
				region++;
				printf("region %d: %lld instructions\n", region, n); fflush(stdout);
				if (!want || want == region) {
					char f[64]; sprintf(f, "prof.%d.txt", region); FILE *o = fopen(f, "w");
					for (int i = 0; i < HS; i++) if (keys[i]) { fprintf(o, "%llx %lld\n", keys[i], vals[i]); }
					fclose(o); memset(keys, 0, sizeof keys); memset(vals, 0, sizeof vals);
					char cmd[128]; sprintf(cmd, "cat /proc/%d/maps > maps.%d.txt", pid, region); system(cmd);
				}
			}
			stepping = !stepping; n = 0; continue;
		}
		pending = sig;
	}
	return 0;
}
