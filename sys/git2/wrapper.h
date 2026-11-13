/*
 * bindgen input of the `git2` link unit, the CLI executable (src/cli): its headers, as its
 * TUs see them, on the include path CMake gives `git2_cli` (src/util, src/cli, include, the
 * build's gen_headers and the dependencies' includes), which campaign mode reads from the
 * variant's build.ninja and the bundled build from its compile_commands.json. The types of
 * libgit2 and src/util they name are `sys::libgit2`'s, as external.txt lists them.
 */
#include "common.h"     /* src/cli: git2_util.h, error.h, opt.h, opt_usage.h */
#include "cmd.h"        /* src/cli */
#include "progress.h"   /* src/cli */
#include "sighandler.h" /* src/cli */

#include "shims.h"
