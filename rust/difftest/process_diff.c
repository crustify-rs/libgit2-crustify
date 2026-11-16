/*
 * Differential test of src/util/unix/process.c: the same lines must be
 * printed when linked against the C reference and against the Rust port.
 * Covers argument and environment set-up (shell wrapping, NAME= removal,
 * exclude_env, the prefix match of git_strlist_contains_key), pipes, exit
 * codes and signals, the failure reports of the child (chdir, execve, not
 * found), result messages and the argument checks.
 *
 * stdout is flushed before every start: the C child calls exit(), which
 * flushes the parent's stdio buffers a second time (rust/DEFECTS.md).
 *
 * Build and compare: RUST_BUILD=/work/builds/u4 rust/difftest/run.sh rust/difftest/process_diff.c
 */

#include "git2_util.h"
#include "git2/global.h"
#include "git2/errors.h"
#include "process.h"
#include "str.h"
#include "vector.h"

#include <signal.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>

static void show_err(const char *what, long rc)
{
	const git_error *e = git_error_last();
	if (rc < 0)
		printf("%s rc=%ld class=%d msg=%s\n", what, rc, e ? e->klass : -1, e ? e->message : "-");
	else
		printf("%s rc=%ld\n", what, rc);
	git_error_clear();
}

static int cmp_str(const void *a, const void *b)
{
	return strcmp(*(const char **)a, *(const char **)b);
}

/* Run, capture stdout (sorted lines when `sort`), and report the result. */
static void run(const char *name, const char **args, size_t args_len,
	const char **env, size_t env_len, unsigned use_shell, unsigned exclude_env,
	const char *cwd, const char *input, int sort, int cmdline)
{
	git_process *p = NULL;
	git_process_options opts = GIT_PROCESS_OPTIONS_INIT;
	git_process_result result = GIT_PROCESS_RESULT_INIT;
	git_str out = GIT_STR_INIT, msg = GIT_STR_INIT;
	char buf[100];
	ssize_t n;
	int rc;

	opts.use_shell = use_shell;
	opts.exclude_env = exclude_env;
	opts.capture_out = 1;
	opts.capture_in = input != NULL;
	opts.cwd = (char *)cwd;

	printf("== %s\n", name);
	rc = cmdline ? git_process_new_from_cmdline(&p, args[0], env, env_len, &opts)
	             : git_process_new(&p, args, args_len, env, env_len, &opts);
	show_err("new", rc);
	if (rc < 0)
		return;

	fflush(stdout);
	rc = git_process_start(p);
	show_err("start", rc);
	if (rc < 0) {
		printf("close_out rc=%d\n", git_process_close_out(p));
		git_process_free(p);
		return;
	}

	if (input) {
		n = git_process_write(p, input, strlen(input));
		show_err("write", n);
		show_err("close_in", git_process_close_in(p));
	}

	while ((n = git_process_read(p, buf, sizeof(buf))) > 0)
		git_str_put(&out, buf, (size_t)n);
	show_err("read", n);

	if (sort) {
		git_vector lines = GIT_VECTOR_INIT;
		char *tok;
		size_t i;
		for (tok = strtok(out.ptr, "\n"); tok; tok = strtok(NULL, "\n"))
			if (strncmp(tok, "TESTDIFF_", 9) == 0 || !exclude_env)
				git_vector_insert(&lines, tok);
		qsort(lines.contents, lines.length, sizeof(void *), cmp_str);
		for (i = 0; i < lines.length; i++)
			if (strncmp(lines.contents[i], "TESTDIFF_", 9) == 0 || exclude_env)
				printf("  %s\n", (char *)lines.contents[i]);
		printf("  (%zu lines)\n", lines.length);
		git_vector_dispose(&lines);
	} else {
		printf("  out=[%s]\n", out.ptr ? out.ptr : "");
	}

	show_err("wait", git_process_wait(&result, p));
	printf("result status=%d exit=%d signal=%d\n", result.status, result.exitcode, result.signal);
	git_process_result_msg(&msg, &result);
	printf("msg=%s\n", msg.ptr);
	show_err("wait again", git_process_wait(&result, p));
	printf("result status=%d exit=%d signal=%d\n", result.status, result.exitcode, result.signal);

	git_process_close(p);
	git_process_free(p);
	git_str_dispose(&out);
	git_str_dispose(&msg);
}

static void test_misc(void)
{
	const char *args[] = { "/bin/cat" };
	git_process *p = NULL;
	git_process_options opts = GIT_PROCESS_OPTIONS_INIT;
	git_process_result r = { 7, 7, 7 };
	git_str msg = GIT_STR_INIT;
	pid_t pid;
	char c;

	show_err("new NULL out", git_process_new(NULL, args, 1, NULL, 0, NULL));
	show_err("new 0 args", git_process_new(&p, args, 0, NULL, 0, NULL));
	printf("p=%p\n", (void *)p);
	show_err("new NULL args", git_process_new(&p, NULL, 1, NULL, 0, NULL));

	show_err("new", git_process_new(&p, args, 1, NULL, 0, NULL));
	show_err("id before start", git_process_id(&pid, p));
	show_err("id NULL", git_process_id(NULL, p));
	show_err("wait before start", git_process_wait(&r, p));
	printf("r=%d %d %d\n", r.status, r.exitcode, r.signal);
	show_err("close_in", git_process_close_in(p));
	show_err("close_out", git_process_close_out(p));
	show_err("close_err", git_process_close_err(p));
	show_err("close", git_process_close(p));
	show_err("read", git_process_read(p, &c, 1));
	show_err("read_err", git_process_read_err(p, &c, 1));
	show_err("write", git_process_write(p, &c, 1));
	show_err("read NULL", git_process_read(NULL, &c, 1));
	show_err("write NULL", git_process_write(NULL, &c, 1));
	git_process_free(p);
	git_process_free(NULL);

	opts.capture_in = 1;
	opts.capture_out = 1;
	opts.capture_err = 1;
	show_err("new", git_process_new(&p, args, 1, NULL, 0, &opts));
	fflush(stdout);
	show_err("start", git_process_start(p));
	show_err("id", git_process_id(&pid, p) < 0 ? -1 : (pid > 0));
	show_err("write", git_process_write(p, "x", 1));
	show_err("close_in", git_process_close_in(p));
	show_err("close_in again", git_process_close_in(p));
	show_err("read", git_process_read(p, &c, 1));
	show_err("read_err (eof)", git_process_read_err(p, &c, 1));
	show_err("wait", git_process_wait(NULL, p));
	/* write to a closed pipe: EBADF */
	show_err("write closed", git_process_write(p, "x", 1));
	git_process_free(p);

	/* SIGPIPE is blocked around writes: a dead reader gives EPIPE */
	{
		const char *t[] = { "/bin/true" };
		int i;
		opts.capture_err = 0;
		show_err("new", git_process_new(&p, t, 1, NULL, 0, &opts));
		fflush(stdout);
		show_err("start", git_process_start(p));
		show_err("wait", git_process_wait(NULL, p));
		for (i = 0; i < 3; i++)
			show_err("write to dead child", git_process_write(p, "hello", 5));
		git_process_free(p);
	}

	for (r.status = 0; r.status < 4; r.status++) {
		int sig;
		for (sig = 0; sig < 2; sig++) {
			r.exitcode = 42; r.signal = sig * 9;
			git_str_clear(&msg);
			printf("result_msg(%d,%d) rc=%d %s\n", r.status, r.signal,
			       git_process_result_msg(&msg, &r), msg.ptr);
		}
	}
	git_str_dispose(&msg);
}

int main(void)
{
	const char *nonexec = "/tmp/u4_process_diff_nonexec";
	const char *badexec = "/tmp/u4_process_diff_badexec";
	int fd = open(nonexec, O_CREAT | O_TRUNC | O_WRONLY, 0644);
	const char *a_false[] = { "/bin/false" };
	const char *a_true_path[] = { "true" };
	const char *a_nf[] = { "/a/b/z/y/not_found" };
	const char *a_nf_path[] = { "no-such-program-for-difftest" };
	const char *a_nonexec[] = { nonexec };
	const char *a_badexec[] = { badexec };
	const char *a_echo[] = { "/bin/echo", "a", "b c", "-n" };
	const char *a_sh[] = { "echo \"$0|$1|$2\"", "x y", "z" };
	const char *a_cmd[] = { "echo hello; exit 3" };
	const char *a_kill[] = { "kill -9 $$" };
	const char *a_pwd[] = { "/bin/pwd" };
	const char *a_cat[] = { "/bin/cat" };
	const char *a_env[] = { "/usr/bin/env" };
	const char *e_add[] = { "TESTDIFF_A=1", "TESTDIFF_B=two=2", "TESTDIFF_DEL=", "TESTDIFF_PREFIXLONG=x" };
	const char *e_only[] = { "TESTDIFF_ONLY=1", "NOEQUALS", "TESTDIFF_EMPTY=" };
	const char *e_del[] = { "TESTDIFF_KEEP=", "HOME=" };

	if (fd >= 0) {
		if (write(fd, "#!/bin/sh\n", 10) < 0) {}
		close(fd);
	}
	/* executable, but not a format execve knows: ENOEXEC in the child */
	fd = open(badexec, O_CREAT | O_TRUNC | O_WRONLY, 0755);
	if (fd >= 0) {
		if (write(fd, "\x7f\x01garbage", 9) < 0) {}
		close(fd);
	}

	git_libgit2_init();
	setenv("TESTDIFF_INHERITED", "yes", 1);
	setenv("TESTDIFF_KEEP", "keep", 1);
	setenv("TESTDIFF_PREFIX", "shadowed by TESTDIFF_PREFIXLONG", 1);
	setenv("TESTDIFF_A", "overridden", 1);

	run("false", a_false, 1, NULL, 0, 0, 0, NULL, NULL, 0, 0);
	run("true in PATH", a_true_path, 1, NULL, 0, 0, 0, NULL, NULL, 0, 0);
	run("not found", a_nf, 1, NULL, 0, 0, 0, NULL, NULL, 0, 0);
	run("not in PATH", a_nf_path, 1, NULL, 0, 0, 0, NULL, NULL, 0, 0);
	run("not executable", a_nonexec, 1, NULL, 0, 0, 0, NULL, NULL, 0, 0);
	run("bad exec format", a_badexec, 1, NULL, 0, 0, 0, NULL, NULL, 0, 0);
	run("echo", a_echo, 4, NULL, 0, 0, 0, NULL, NULL, 0, 0);
	run("shell args", a_sh, 3, NULL, 0, 1, 0, NULL, NULL, 0, 0);
	run("cmdline", a_cmd, 1, NULL, 0, 0, 0, NULL, NULL, 0, 1);
	run("killed", a_kill, 1, NULL, 0, 0, 0, NULL, NULL, 0, 1);
	run("cwd", a_pwd, 1, NULL, 0, 0, 0, "/tmp", NULL, 0, 0);
	run("bad cwd", a_pwd, 1, NULL, 0, 0, 0, "/no/such/dir", NULL, 0, 0);
	run("cat", a_cat, 1, NULL, 0, 0, 0, NULL, "line 1\nline 2\n", 0, 0);
	run("shell not found", a_nf_path, 1, NULL, 0, 1, 0, NULL, NULL, 0, 0);
	run("env add", a_env, 1, e_add, 4, 0, 0, NULL, NULL, 1, 0);
	run("env exclude", a_env, 1, e_only, 3, 0, 1, NULL, NULL, 1, 0);
	run("env exclude empty", a_env, 1, NULL, 0, 0, 1, NULL, NULL, 1, 0);
	run("env delete", a_env, 1, e_del, 2, 0, 0, NULL, NULL, 1, 0);
	test_misc();

	unlink(nonexec);
	unlink(badexec);
	git_libgit2_shutdown();
	return 0;
}
