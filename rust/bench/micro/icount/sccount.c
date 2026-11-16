/* sccount: count syscalls (by name via ausyscall-less table: number) between SIGSTOP markers */
#include <sys/ptrace.h>
#include <sys/user.h>
#include <sys/wait.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
int main(int argc, char **argv) {
	pid_t pid = fork();
	if (pid == 0) { ptrace(PTRACE_TRACEME, 0, 0, 0); execvp(argv[1], argv + 1); _exit(127); }
	int st; waitpid(pid, &st, 0);
	ptrace(PTRACE_SETOPTIONS, pid, 0, PTRACE_O_TRACESYSGOOD);
	int on = 0, region = 0, entering = 1; long cnt[512] = {0};
	for (;;) {
		ptrace(PTRACE_SYSCALL, pid, 0, 0);
		if (waitpid(pid, &st, 0) < 0 || WIFEXITED(st) || WIFSIGNALED(st)) break;
		int sig = WSTOPSIG(st);
		if (sig == (SIGTRAP | 0x80)) {
			if (entering && on) { struct user_regs_struct r; ptrace(PTRACE_GETREGS, pid, 0, &r); if (r.orig_rax < 512) cnt[r.orig_rax]++; }
			entering = !entering; continue;
		}
		if (sig == SIGSTOP) {
			if (on) { region++; printf("region %d:", region); long t = 0; for (int i = 0; i < 512; i++) if (cnt[i]) { printf(" %d:%ld", i, cnt[i]); t += cnt[i]; cnt[i] = 0; } printf("  total %ld\n", t); fflush(stdout); }
			on = !on; continue;
		}
		ptrace(PTRACE_SYSCALL, pid, 0, sig);
		if (waitpid(pid, &st, 0) < 0 || WIFEXITED(st)) break;
		entering = !entering;
	}
	return 0;
}
