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
   - Answer: `claude, subscription, args: --model claude-opus-5-5`

5. **Do you want agentic review after translated work lands? If so, which
   agent CLI and billing should perform each review?** (`id: review-agent`)
   - Answer: `claude, subscription, args: --model claude-opus-5-5`, after each wave

6. **Should I run fully autonomously end to end?** (`id: autonomy`)
   - Answer: `no`

# Optional questions

Unanswered optional questions use their defaults.

## Campaign execution

7. **Which agent CLI and billing should plan the campaign, and how many planner runs
   should it make? Each run emits what the plan lacks and corrects what the runs
   before it committed.** (`id: plan-agent`)
   - Answer: `claude, subscription, args: --model claude-opus-5-5, runs: 2`

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
    - Answer: `decide after planning`
    - `decide after planning`: once the plan is complete, I propose ship points from it,
      with the boundary each would leave, and record your choice here.
    - By default: after every integrated module.

## Evolution

14. **How far may the campaign evolve Crustify and its tools from what it exposes, and for
    how long without asking you? Agents file proposals and record the target's lessons;
    fixes and improvements land on each component's `devel` branch, never on its `main`.**
    (`id: evolution`)
    - Answer: `propose`, the default
    - `propose`: proposals and lessons only. `repair`: also fixes of defects. `evolve`: also
      improvements and new tools. Without `unattended`, you approve each change at the next
      module's end. Merging `devel` into `main`, and a new tool upstream, always need your
      approval.

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
