/* icount: run argv under ptrace; count user instructions between the child's
 * SIGSTOP markers (region 1 = between 1st and 2nd SIGSTOP, ...). */
#include <sys/ptrace.h>
#include <sys/wait.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
int main(int argc, char **argv) {
	pid_t pid = fork();
	if (pid == 0) { ptrace(PTRACE_TRACEME, 0, 0, 0); execvp(argv[1], argv + 1); _exit(127); }
	int st; waitpid(pid, &st, 0); /* exec stop */
	int pending = 0; int stepping = 0, region = 0; long long n = 0;
	for (;;) {
		ptrace(stepping ? PTRACE_SINGLESTEP : PTRACE_CONT, pid, 0, pending); pending = 0;
		if (waitpid(pid, &st, 0) < 0 || WIFEXITED(st) || WIFSIGNALED(st)) break;
		int sig = WSTOPSIG(st);
		if (sig == SIGTRAP && stepping) { n++; continue; }
		if (sig == SIGSTOP) {
			if (stepping) { printf("region %d: %lld instructions\n", ++region, n); fflush(stdout); }
			stepping = !stepping; n = 0; continue;
		}
		pending = sig; /* pass other signals on the next resume */
	}
	return 0;
}
