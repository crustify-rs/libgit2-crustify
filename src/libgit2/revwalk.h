/*
 * Copyright (C) the libgit2 contributors. All rights reserved.
 *
 * This file is part of libgit2, distributed under the GNU GPL v2 with
 * a Linking Exception. For full terms see the included COPYING file.
 */
#ifndef INCLUDE_revwalk_h__
#define INCLUDE_revwalk_h__

#include "common.h"

#include "git2/revwalk.h"
#include "commit_list.h"
#include "pqueue.h"
#include "pool.h"
#include "vector.h"
#include "hashmap_oid.h"

GIT_HASHMAP_OID_STRUCT(git_revwalk_oidmap, git_commit_list_node *);

/*
 * The walker is defined in Rust (rust/libgit2-rs/src/revwalk). C sees only
 * this prefix of it, which the Rust allocates and lays out at offset 0;
 * the walk's own state (iterators, sorting, flags, the hide callback, the
 * pathspec) follows it and is private to the Rust. Never allocate, embed
 * or size a git_revwalk in C.
 *
 * `commits` is only touched by the Rust (git_revwalk__commit_lookup), but
 * stays here so that `commit_pool` keeps its offset.
 */
struct git_revwalk {
	git_repository *repo;
	git_odb *odb;

	git_revwalk_oidmap commits;
	git_pool commit_pool;

	/* the pushes and hides */
	git_commit_list *user_input;
};

git_commit_list_node *git_revwalk__commit_lookup(git_revwalk *walk, const git_oid *oid);

typedef struct {
	int uninteresting;
	int from_glob;
	int insert_by_date;
} git_revwalk__push_options;

#define GIT_REVWALK__PUSH_OPTIONS_INIT { 0 }

int git_revwalk__push_commit(git_revwalk *walk,
	const git_oid *oid,
	const git_revwalk__push_options *opts);

int git_revwalk__push_ref(git_revwalk *walk,
	const char *refname,
	const git_revwalk__push_options *opts);

int git_revwalk__push_glob(git_revwalk *walk,
	const char *glob,
	const git_revwalk__push_options *given_opts);

#endif
