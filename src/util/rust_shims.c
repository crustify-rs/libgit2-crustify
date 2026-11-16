/*
 * Copyright (C) the libgit2 contributors. All rights reserved.
 *
 * This file is part of libgit2, distributed under the GNU GPL v2 with
 * a Linking Exception. For full terms see the included COPYING file.
 */

/*
 * The variadic functions of the util layer, which stable Rust cannot
 * define.  When the core is built from the Rust port (USE_RUST), the
 * C sources of these functions leave the build; these thin shims take
 * their place: they format with vsnprintf (or collect the arguments)
 * and hand the result to the Rust port (rust/libgit2-rs/src/util).
 *
 * This file is compiled only with USE_RUST=ON.
 */

#include "git2_util.h"

#include "errors.h"
#include "str.h"

/* Implemented in Rust (src/util/errors.rs). */
extern void git_error__set_formatted(
	int error_class, const char *message, size_t len, int os_errno);

/* Implemented in Rust (src/util/str.rs). */
extern int git_str__vprintf_begin(git_str *buf, size_t format_len);
extern int git_str__vprintf_written(git_str *buf, int len);
extern int git_str__join_n(
	git_str *buf, char separator, int nbuf, const char **segments);

void git_error_set(int error_class, const char *fmt, ...)
{
	va_list ap;

	va_start(ap, fmt);
	git_error_vset(error_class, fmt, ap);
	va_end(ap);
}

void git_error_vset(int error_class, const char *fmt, va_list ap)
{
	/* errno is read first, as the C did, before anything may change it. */
	int os_errno = (error_class == GIT_ERROR_OS) ? errno : 0;
	char stackbuf[512], *msg = stackbuf;
	va_list args;
	int len;

	if (!fmt) {
		git_error__set_formatted(error_class, NULL, 0, os_errno);
		return;
	}

	va_copy(args, ap);
	len = p_vsnprintf(stackbuf, sizeof(stackbuf), fmt, args);
	va_end(args);

	if (len < 0) {
		/*
		 * An encoding error.  The C marked the thread's message buffer
		 * out of memory here (see rust/DEFECTS.md); report that.
		 */
		git_error_set_oom();
		return;
	} else if ((size_t)len >= sizeof(stackbuf)) {
		/*
		 * Longer messages are formatted again in a heap buffer from the
		 * C library's allocator, which only this function uses and frees
		 * (the Rust copies the message into the thread's error state).
		 */
		if ((msg = malloc((size_t)len + 1)) == NULL) {
			git_error_set_oom();
			return;
		}

		va_copy(args, ap);
		len = p_vsnprintf(msg, (size_t)len + 1, fmt, args);
		va_end(args);

		if (len < 0) {
			free(msg);
			git_error_set_oom();
			return;
		}
	}

	git_error__set_formatted(error_class, msg, (size_t)len, os_errno);

	if (msg != stackbuf)
		free(msg);
}

int git_str_vprintf(git_str *buf, const char *format, va_list ap)
{
	int len, r;

	if ((r = git_str__vprintf_begin(buf, strlen(format))) < 0)
		return r;

	while (1) {
		va_list args;
		va_copy(args, ap);

		len = p_vsnprintf(
			buf->ptr + buf->size,
			buf->asize - buf->size,
			format, args
		);

		va_end(args);

		/* 0: done; 1: grown, format again; < 0: failed. */
		if ((r = git_str__vprintf_written(buf, len)) <= 0)
			return r;
	}
}

int git_str_printf(git_str *buf, const char *format, ...)
{
	int r;
	va_list ap;

	va_start(ap, format);
	r = git_str_vprintf(buf, format, ap);
	va_end(ap);

	return r;
}

#define JOIN_N_STACK 16

int git_str_join_n(git_str *buf, char separator, int nbuf, ...)
{
	const char *stack_segments[JOIN_N_STACK], **segments = stack_segments;
	va_list ap;
	int i, r;

	if (nbuf > JOIN_N_STACK &&
	    (segments = git__mallocarray((size_t)nbuf, sizeof(char *))) == NULL)
		return -1;

	va_start(ap, nbuf);
	for (i = 0; i < nbuf; ++i)
		segments[i] = va_arg(ap, const char *);
	va_end(ap);

	r = git_str__join_n(buf, separator, nbuf, segments);

	if (segments != stack_segments)
		git__free((void *)segments);

	return r;
}
