Fill in as many answers as you want before starting Crustify. It will ask only
for campaign decisions that remain unresolved.

# Mandatory questions

## Campaign

1. **Which repository and revision should this campaign use?** (`id: source`)
   - Answer: `https://github.com/libgit2/libgit2.git`, `0551dfd4a` (the `c/` submodule):
     the revision of the checkout at `/target` the campaign started from

2. **Should this campaign port the C implementation to Rust, or create safe
   Rust wrappers?** (`id: campaign`)
   - Answer: `port`

3. **What should this campaign target: a named subset of modules, or a named
   subset of functions and types, or the whole target repo? You can define them
   now or we can brainstorm them during the live session. You can also let the
   assistant decide.** (`id: scope`)
   - Answer: the whole target, excluding deprecated items, building up on the
     previous wrap campaign.

4. **Which agent CLI and billing should do the translation work?**
   (`id: translate-agent`)
   - Answer: `claude, api, proxy: openrouter, args: --model claude-opus-5-5`

5. **Do you want agentic review after translated work lands? If so, which
   agent CLI and billing should perform each review?** (`id: review-agent`)
   - Answer: `claude, api, proxy: openrouter, args: --model claude-opus-5-5`, after each wave

6. **Should I run fully autonomously end to end?** (`id: autonomy`)
   - Answer: `no`

# Optional questions

Unanswered optional questions use their defaults.

## Campaign execution

7. **Which agent CLI and billing should plan the campaign, and how many planner runs
   should it make? Each run emits what the plan lacks and corrects what the runs
   before it committed.** (`id: plan-agent`)
   - Answer: `claude, api, proxy: openrouter, args: --model claude-opus-5-5, runs: 2`

8. **Should batches and parallelism be capped? Each wave's batches are packed when it
   starts, sized by its items, the agents and the host; a cap bounds every batch, or the
   agents running at once.** (`id: workload`)
   - Answer: `assistant's choice`

9. **Should review batches be capped? They are packed larger than translation batches,
   since a reviewer reads landed code rather than writing it.** (`id: review-workload`)
   - Answer: `assistant's choice`

10. **Which objectives should review agents review for? By default, every one the
    reviewer offers.** (`id: review-objectives`)
    - Answer: `all`
    - Without `fix`, reviewers report and reproduce what they find but patch nothing.

11. **Which sanitizers should the campaign build and test under? By default, every one
    the toolchain supports.** (`id: sanitizers`)
    - Answer: `asan-ubsan, tsan, bsan`, the variants `crustify/artifacts/build.json`
      records; MSan is not built.
    - From 2026-11-15, the user's decision, a departure from the `testing-gates` skill's
      Regressions section: BSan runs over the workspace only at a ship point, in its full
      run. It is no gate of a wave's end, a module's end or switch, or the periodic full
      runs between ship points, which run every other stage. A batch still runs it on the
      tests its own work adds or changes, and a review on its reproducers, each named and
      alone, under a `timeout`. The `bsan` variant is still built with each generation.
      The round of 2026-11-15 spent about 6.5 of its 20.3 agent hours on commands running
      BSan. Before it, from 2026-11-14, BSan ran as the skill has it, at every wave's and
      module's end less the slow list, `crustify/scripts/slow-list.txt`; and from
      2026-11-13 to 2026-11-14 under this same departure.
    - Miri needs no build and is always on.

12. **How should the target be configured: which of its build features should be
    enabled or disabled?** (`id: target-config`)
    - Answer: the configuration `crustify/artifacts/build.json` records: deprecated
      features disabled (`-DDEPRECATE_HARD=ON`), HTTPS through OpenSSL, SSH, Negotiate
      auth and i18n off, threads on.

13. **At which points should the hybrid C-Rust program ship? Shipping advances `main` to
    what the campaign has integrated, with your approval each time: the modules shipped
    are translated, and the C still left is reached only through the wrappers on its
    boundary.** (`id: ship`)
    - Answer: `after:` `src/libgit2/repository.c` and every module its `schedule.json`
      record reaches through `depends_on.scheduled`, 42 modules of about 15.8K lines of C,
      their `wrap` and `port` waves planned through wave 781: `src/libgit2/buf.c`,
      `src/libgit2/common.h`, `src/libgit2/config.c`, `src/libgit2/grafts.c`,
      `src/libgit2/object.c`, `src/libgit2/odb.c`, `src/libgit2/oid.c`,
      `src/libgit2/oidarray.c`, `src/libgit2/parse.c`, `src/libgit2/refs.c`,
      `src/libgit2/repository.c`, `src/libgit2/strarray.c`, `src/libgit2/sysdir.c`,
      `src/util/alloc.c`, `src/util/allocators/failalloc.c`, `src/util/array.h`,
      `src/util/cc-compat.h`, `src/util/ctype_compat.h`, `src/util/errors.c`,
      `src/util/filebuf.c`, `src/util/fs_path.c`, `src/util/futils.c`, `src/util/hash.c`,
      `src/util/hash/collisiondetect.c`, `src/util/hash/openssl.c`,
      `src/util/hash/sha1dc/sha1.c`, `src/util/hash/sha1dc/ubc_check.c`,
      `src/util/hashmap.h`, `src/util/hashmap_str.h`, `src/util/integer.h`,
      `src/util/pool.c`, `src/util/posix.c`, `src/util/rand.c`, `src/util/regexp.c`,
      `src/util/runtime.c`, `src/util/str.c`, `src/util/thread.c`, `src/util/tsort.c`,
      `src/util/unix/map.c`, `src/util/unix/realpath.c`, `src/util/util.c`,
      `src/util/vector.c`: the first ship point, chosen by the user on 2026-11-15 in place
      of the list by size (about 15K lines of integrated leaves, through wave 338). A
      re-plan that changes the closure changes this list with it. `src/util/alloc.c` and
      `src/util/allocators/failalloc.c` are wrapped, not ported, and the dependencies the
      order cuts stay C behind wrappers at this point. Later points are core modules
      with their closures too, each confirmed with the user; `src/libgit2/commit.c`, nine
      modules further, is the candidate for the second.
    - `decide after planning`: once the plan is complete, I propose ship points from it,
      with the boundary each would leave, and record your choice here.
    - By default: after every integrated module.

## Evolution

14. **How far may the campaign evolve Crustify and its tools from what it exposes? Agents
    file proposals and record the target's lessons; fixes and improvements land, once
    another agent has reviewed them, on a working branch of each component that is the
    campaign's own, and reach the component's `devel` only with your approval.**
    (`id: evolution`)
    - Answer: `evolve`, on the campaign's working branches (`libgit2-crustify/devel` of each
      component, checked out since 2026-11-14 in a worktree of its own beside the
      component's checkout, `/crustify-rs/<component>.libgit2-crustify`, the component's
      own checkout being on `devel`; a session of this campaign loads Crustify from that
      worktree, `--plugin-dir /crustify-rs/crustify.libgit2-crustify`): a change lands
      there once its review approves it, and each component's `devel` takes the working
      branch only with the user's approval, as crustify `c2f21a0`'s model has it. The
      former `unattended: the whole campaign` is retired with it.
    - `propose`: proposals and lessons only. `repair`: also fixes of defects. `evolve`: also
      improvements and new tools. You are asked to take the working branches into `devel`
      at each module's end and the campaign's end.

## Commits

15. **Which git identity should the campaign's commits carry: an author, and any
    co-authors? By default, Crustify's own, `crustify <crustify@berkeley.edu>` (the
    GitHub account `crustify-ai`), as the author, with no co-author.** (`id: identity`)
    - Answer: `author: Marius Momeu <marius.momeu@berkeley.edu>, co-authors: crustify
      <crustify@berkeley.edu>` (the GitHub account `crustify-ai`)

## Autonomy (if question 6 is answered `no`)

Answer these approval-gate questions only if question 6 is answered `no`.

16. **Should I wait for your approval before starting the setup phase?** (`id: gates.setup`)
    - Answer: `no`
17. **Should I wait for your approval before starting the translation phase?**
    (`id: gates.translation`)
    - Answer: `no`
18. **Should I wait for your approval before integrating each module into the working
    branch?** (`id: gates.module`)
    - Answer: `no`
19. **Should I wait for your approval before starting review passes?** (`id: gates.review`)
    - Answer: `no`

# Benchmark recording questions

20. **Where and in what format should the dashboard be recorded? By default, at
    `crustify/artifacts/dashboard.md`, from the standard template.** (`id: dashboard`)
    - Answer: `<repo-checkout>/crustify/artifacts/dashboard.md`, `standard` template

# Additional instructions

Unless otherwise stated, preserve the format of the dashboard exactly.
