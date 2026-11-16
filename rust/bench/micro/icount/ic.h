#include <git2.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#define CK(x) do { int _e = (x); if (_e < 0) { const git_error *er = git_error_last(); fprintf(stderr, "%s:%d %d %s\n", __FILE__, __LINE__, _e, er ? er->message : ""); exit(1);} } while (0)
static int ic_marks = 1; /* 0: timing runs, no tracer to resume a stop */
#define MARK() do { if (ic_marks) raise(SIGSTOP); } while (0)
