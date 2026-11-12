# Consistency fixes to the spec, made in another workspace

Status: `landed` on Crustify's `devel`, cherry-picked by the assistant on 2026-11-12 from the
`review/evolution/*` branches another workspace made. The user reported them, and
they fall under the settings' `evolution` answer (`evolve, unattended: the whole campaign`).
Each one was made by reading the spec for consistency, not from a campaign's run.

| change | commit | what it settles |
|---|---|---|
| `landing-lock-release` | `af5dc69` | A lock holder is bound to its CLI. An agent that shares its CLI with others, as this campaign's subagents do, leaves its holder running when it is ended. The assistant then ends the holder by the pid its record names, after removing the record. |
| `tool-skills` | `d82a46b` | An agent reads a tool's skill before the first step its description names, not every tool skill before its first step. |
| `blocking-defect` | `37683f4` | A run that a defect blocks makes its fix, or under `propose` files it, and ends. The fix lands at once under `unattended`, or else the user is asked at once. The run is then run again. The templates' default `evolution` answer becomes `repair, unattended`. |
| `plan-approval` | `8b7fae2` | The complete plan is put to the user before the Rust tree is scaffolded, and waits where `gates.translation` asks for it. The brief lists the scope as the settings give it. The preflight compares against the plan as presented. |
| `port-switch` | `a6fb91b` | A ported module's one-time change from C to Rust is its *switch*. *Integration* keeps only its meaning of merging a module's run into the working branch. No rule changes. |
| `revisions` | `ac9c126` | Every run's `task.json` records `revisions` at launch, and the field is now required. A change takes effect as it lands, not from the next wave. A run under way may have followed either rule, which its revisions and landing date tell. |
| `inconsistencies` | `2821aac` | `schedule.json` has no wave of no module. The assistant commits to the working branch while nothing is promoted or integrated to it. Under `autonomy: no`, the assistant waits at each `gates` point answered `yes`. The audit template gets question ids and a `source` question. Smaller fixes in bindgen, scheduling, testing and the README. |
| `descriptive-skills` | `4d0a81c` | Skills name no role and no tool as the subject of a step, as `authoring` asks. The tools' own skills carry what they add. |
| `deferred-todos` | `8ef2675` | TODOs for six gaps the user deferred: the agent memory, when branches are pushed, ffibox's unpinned path dependency, the unpinned libclang, the end gates in a campaign without review, and a C ABI over a ported public API. |

## For this campaign

- **Landing locks:** this campaign's stage agents run as subagents of the assistant's CLI.
  So an ended agent's lock holder outlives it, and the assistant releases the lock by its
  pid.
- **Planner run 12** launched under `66e2ae8` and reads the live skills, so these changes
  reach it as they land (`revisions`).
- **Settings:** this campaign answers `autonomy: no` with every `gates` answer `no`, so
  `inconsistencies` adds no wait. It has no wave of no module.
- **`plan-approval`:** its re-plan is reported to the user before the next waves launch,
  as `plan-approval` asks of a complete plan.

## Validation

- Read against the skills they touch. All apply cleanly in sequence on `66e2ae8`.
- `providers/claude/compose.py build` reproduces the committed provider agents unchanged.
- The source branches stay checked out in the other workspace's worktrees, so they were
  not deleted.
