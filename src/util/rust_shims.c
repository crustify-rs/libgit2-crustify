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
#include "posix.h"
#include "filebuf.h"

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

/* Implemented in Rust (src/util/posix.rs). */
extern int git__p_open(const char *path, int flags, mode_t mode);

int p_open(const char *path, volatile int flags, ...)
{
	mode_t mode = 0;

	if (flags & O_CREAT) {
		va_list arg_list;

		va_start(arg_list, flags);
		mode = (mode_t)va_arg(arg_list, int);
		va_end(arg_list);
	}

	return git__p_open(path, flags, mode);
}

/* Implemented in Rust (src/util/filebuf.rs). */
extern int git_filebuf__printf_formatted(
	git_filebuf *file, const char *data, int len);

#define FILEBUF_PRINTF_STACK 8192

int git_filebuf_printf(git_filebuf *file, const char *format, ...)
{
	char stackbuf[FILEBUF_PRINTF_STACK], *msg = stackbuf;
	va_list ap;
	int len, r;

	va_start(ap, format);
	len = p_vsnprintf(stackbuf, sizeof(stackbuf), format, ap);
	va_end(ap);

	if (len >= 0 && (size_t)len >= sizeof(stackbuf)) {
		/* < 0 tells the Rust that formatting (here: memory) failed. */
		if ((msg = git__malloc((size_t)len + 1)) == NULL)
			return git_filebuf__printf_formatted(file, NULL, -1);

		va_start(ap, format);
		len = p_vsnprintf(msg, (size_t)len + 1, format, ap);
		va_end(ap);
	}

	r = git_filebuf__printf_formatted(file, msg, len);

	if (msg != stackbuf)
		git__free(msg);

	return r;
}
/*
 * git_libgit2_opts (src/libgit2/settings.c): the shim reads the arguments
 * of each option from the va_list, as the C did, and hands them to the
 * Rust (rust/libgit2-rs/src/settings.rs), which does the rest.
 */
#include "git2/common.h"
#include "git2/config.h"

struct git_libgit2__opt_args {
	int i;
	size_t sz;
	ssize_t ssz;
	void *p1;
	void *p2;
};

/* Implemented in Rust (src/settings.rs). */
extern int git_libgit2__opts(int key, const struct git_libgit2__opt_args *args);

int git_libgit2_opts(int key, ...)
{
	struct git_libgit2__opt_args a = { 0 };
	va_list ap;

	va_start(ap, key);

	switch (key) {
	/* (size_t) */
	case GIT_OPT_SET_MWINDOW_SIZE:
	case GIT_OPT_SET_MWINDOW_MAPPED_LIMIT:
	case GIT_OPT_SET_MWINDOW_FILE_LIMIT:
	case GIT_OPT_SET_PACK_MAX_OBJECTS:
	case GIT_OPT_SET_PACK_MAX_OBJECT_SIZE:
		a.sz = va_arg(ap, size_t);
		break;

	/* (ssize_t) */
	case GIT_OPT_SET_CACHE_MAX_SIZE:
		a.ssz = va_arg(ap, ssize_t);
		break;

	/* (int) */
	case GIT_OPT_ENABLE_CACHING:
	case GIT_OPT_ENABLE_STRICT_OBJECT_CREATION:
	case GIT_OPT_ENABLE_STRICT_SYMBOLIC_REF_CREATION:
	case GIT_OPT_ENABLE_OFS_DELTA:
	case GIT_OPT_ENABLE_FSYNC_GITDIR:
	case GIT_OPT_ENABLE_STRICT_HASH_VERIFICATION:
	case GIT_OPT_ENABLE_UNSAVED_INDEX_SAFETY:
	case GIT_OPT_DISABLE_PACK_KEEP_FILE_CHECKS:
	case GIT_OPT_ENABLE_HTTP_EXPECT_CONTINUE:
	case GIT_OPT_SET_ODB_PACKED_PRIORITY:
	case GIT_OPT_SET_ODB_LOOSE_PRIORITY:
	case GIT_OPT_SET_OWNER_VALIDATION:
	case GIT_OPT_SET_SERVER_CONNECT_TIMEOUT:
	case GIT_OPT_SET_SERVER_TIMEOUT:
		a.i = va_arg(ap, int);
		break;

	/* (one pointer) */
	case GIT_OPT_GET_MWINDOW_SIZE:
	case GIT_OPT_GET_MWINDOW_MAPPED_LIMIT:
	case GIT_OPT_GET_MWINDOW_FILE_LIMIT:
	case GIT_OPT_GET_TEMPLATE_PATH:
	case GIT_OPT_SET_TEMPLATE_PATH:
	case GIT_OPT_ADD_SSL_X509_CERT:
	case GIT_OPT_SET_USER_AGENT:
	case GIT_OPT_GET_USER_AGENT:
	case GIT_OPT_SET_USER_AGENT_PRODUCT:
	case GIT_OPT_GET_USER_AGENT_PRODUCT:
	case GIT_OPT_SET_SSL_CIPHERS:
	case GIT_OPT_SET_ALLOCATOR:
	case GIT_OPT_GET_PACK_MAX_OBJECTS:
	case GIT_OPT_GET_PACK_MAX_OBJECT_SIZE:
	case GIT_OPT_GET_EXTENSIONS:
	case GIT_OPT_GET_OWNER_VALIDATION:
	case GIT_OPT_GET_HOMEDIR:
	case GIT_OPT_SET_HOMEDIR:
	case GIT_OPT_GET_SERVER_CONNECT_TIMEOUT:
	case GIT_OPT_GET_SERVER_TIMEOUT:
		a.p1 = va_arg(ap, void *);
		break;

	/* (two pointers) */
	case GIT_OPT_GET_CACHED_MEMORY:
	case GIT_OPT_SET_SSL_CERT_LOCATIONS:
		a.p1 = va_arg(ap, void *);
		a.p2 = va_arg(ap, void *);
		break;

	/* (int level, git_buf *) */
	case GIT_OPT_GET_SEARCH_PATH:
		a.i = va_arg(ap, int);
		a.p1 = va_arg(ap, void *);
		break;

	/* (int level, const char *): the path is read only for a valid
	 * level, as the C did. */
	case GIT_OPT_SET_SEARCH_PATH:
		a.i = va_arg(ap, int);
		if (a.i == GIT_CONFIG_LEVEL_SYSTEM ||
		    a.i == GIT_CONFIG_LEVEL_XDG ||
		    a.i == GIT_CONFIG_LEVEL_GLOBAL)
			a.p1 = va_arg(ap, void *);
		break;

	/* (git_object_t, size_t) */
	case GIT_OPT_SET_CACHE_OBJECT_LIMIT:
		a.i = va_arg(ap, int);
		a.sz = va_arg(ap, size_t);
		break;

	/* (const char **, size_t) */
	case GIT_OPT_SET_EXTENSIONS:
		a.p1 = va_arg(ap, void *);
		a.sz = va_arg(ap, size_t);
		break;

	/* Windows only: nothing is read elsewhere (as the C). */
	case GIT_OPT_GET_WINDOWS_SHAREMODE:
	case GIT_OPT_SET_WINDOWS_SHAREMODE:
	default:
		break;
	}

	va_end(ap);

	return git_libgit2__opts(key, &a);

/*
 * git_commit_create_v (src/libgit2/commit.c, ported): the variadic parents
 * are collected into an array for the Rust (src/commit.rs), which treats a
 * NULL parent as the end of the list, as the C's va_arg reader did.
 */
#include "git2/commit.h"

extern int git_commit__create_v(
	git_oid *id,
	git_repository *repo,
	const char *update_ref,
	const git_signature *author,
	const git_signature *committer,
	const char *message_encoding,
	const char *message,
	const git_tree *tree,
	size_t parent_count,
	const git_commit **parents);

int git_commit_create_v(
	git_oid *id,
	git_repository *repo,
	const char *update_ref,
	const git_signature *author,
	const git_signature *committer,
	const char *message_encoding,
	const char *message,
	const git_tree *tree,
	size_t parent_count,
	...)
{
	const git_commit *stackbuf[8], **parents = stackbuf;
	va_list ap;
	size_t i;
	int error;

	if (parent_count > sizeof(stackbuf) / sizeof(stackbuf[0])) {
		parents = git__mallocarray(parent_count, sizeof(*parents));
		GIT_ERROR_CHECK_ALLOC(parents);
	}

	va_start(ap, parent_count);
	for (i = 0; i < parent_count; i++)
		parents[i] = va_arg(ap, const git_commit *);
	va_end(ap);

	error = git_commit__create_v(id, repo, update_ref, author, committer,
		message_encoding, message, tree, parent_count, parents);

	if (parents != stackbuf)
		git__free((void *)parents);

	return error;
}
