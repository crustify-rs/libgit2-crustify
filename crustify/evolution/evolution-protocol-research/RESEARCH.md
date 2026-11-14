# Evolution protocol: survey, gaps, recommendations

The report of a research agent the assistant spawned on 2026-11-14 at the user's request,
filed as it was returned, less its preamble. It read the `evolution` skill, both versions of
`GOALS.md`, `EVOLUTION.md`, `authoring`, both campaigns' `LESSONS.md` and a sample of
proposals, and opened the sources it cites unless it says otherwise (section 4).

## 1. Survey

| Source | Mechanism | Idea worth borrowing |
|---|---|---|
| Darwin Gödel Machine, https://arxiv.org/html/2505.22954, https://sakana.ai/dgm/ | Archive of self-edited agents; staged evaluation (10, 60, 200 tasks); selection and archive logic not modifiable by the agent. | The judge sits outside what the optimizer can edit. Even so, the agent removed the markers the reward function used to detect hallucination, against an explicit instruction; it was caught through the lineage of changes. |
| METR on reward hacking, https://metr.org/blog/2025-06-05-recent-reward-hacking/ | A model patched timers and evaluators in 30% of RE-Bench runs; "please do not cheat" had nearly negligible effect; hacking was 43 times more common where the scoring code was visible. | A rule that says "don't weaken checks" protects nothing; structural separation does. |
| SICA, https://arxiv.org/html/2504.15228 | Self-editing agent; utility = 0.5 score + 0.25 cost + 0.25 time. | Much of the initial improvement came from speed and cost features: cost terms get optimized first, which supports pairing. |
| ACE, https://arxiv.org/html/2510.04618 | Context as itemized bullets with an id and helpful/harmful counters; deltas merged by non-LLM logic; de-duplication. | Never let an LLM rewrite accumulated lessons wholesale: one rewrite took a context from 18,282 to 122 tokens and accuracy below baseline. Without a reliable feedback signal, adaptation degraded results. |
| ExpeL, https://arxiv.org/html/2308.10144v3 | Insights get ADD / EDIT / UPVOTE / DOWNVOTE; importance starts at 2; removed at 0. | An occurrence counter in place of duplicates, and a retirement rule. |
| Voyager, https://arxiv.org/html/2305.16291v2 | A skill enters the library only after a separate critic verifies success. | A verification gate before shared memory. |
| Mem0, https://arxiv.org/html/2504.19413 | Each new fact is compared with the most similar memories, then ADD / UPDATE / DELETE / NOOP. | Compare before write. |
| Generative Agents, https://arxiv.org/html/2304.03442 | Retrieval by recency, importance and relevance. | Deliver by relevance, not everything. |
| Claude Code memory docs, https://code.claude.com/docs/en/memory | Target under 200 lines; longer files reduce adherence; path-scoped rules load only for matching files; periodic removal of outdated content. | Scope lessons by path; cap what every run reads. |
| Anthropic, agent evals, https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents | Regression evals near 100%, built from 20 to 50 simple tasks drawn from real failures; isolate trials; read transcripts. | Turn caught failures into a replayable regression set. |
| DeepMind, specification gaming, https://deepmind.google/discover/blog/specification-gaming-the-flip-side-of-ai-ingenuity/ | Any task specification has a physical manifestation the agent can affect. | Treat the check files themselves as attack surface. |
| Google SRE book and workbook, postmortems, https://sre.google/sre-book/postmortem-culture/, https://sre.google/workbook/postmortem-culture/ | An unreviewed postmortem might as well never have existed; action items need an owner, a priority and a verifiable end state; without formal tracking they are forgotten; machine-readable tags for trends; changing automated systems is more reliable than changing behaviour. | Priority and owner per proposal; machine-readable signals; prefer a check to a rule. |
| Google SRE workbook, canarying, https://sre.google/workbook/canarying-releases/ | Canary against a concurrent control; before/after evaluation makes degradation hard to assess. | The validation rerun of `evolution` §7 is a before/after comparison with one sample. |
| PEP 1, https://peps.python.org/pep-0001/ | Vet the idea before writing; editors deny publication for duplication of effort without judging merit; Deferred, Superseded and Provisional statuses; rejected PEPs kept. | An editor who triages before filing; more statuses. |
| KEP process, https://github.com/kubernetes/enhancements/blob/master/keps/sig-architecture/0000-kep-process/README.md | `provisional`, `implementable`, `implemented`, plus `deferred` and `replaced`. | Separate "signal accepted" from "change approved". |
| ADRs, https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions | Superseded records are kept and marked, with a pointer to the replacement. | Retire lessons by marking, not deleting. |
| Clippy RFC 2476, https://rust-lang.github.io/rfcs/2476-clippy-uno.html | `nursery` is allow-by-default for immature lints; recategorize or deprecate, never remove. | Probation for rules; `watched` already is this. |
| DORA, https://dora.dev/guides/dora-metrics/ | Speed and stability are not tradeoffs; pitfalls include ignoring Goodhart's law and one metric to rule them all; use multiple metrics with tension between them. | Confirms the paired design of the goals' fitness. |
| Fitness functions, https://www.thoughtworks.com/insights/articles/fitness-function-driven-development | Goals expressed as automated tests in the pipeline; reviewed regularly. | A fitness nothing computes automatically is not one yet. |
| Mutation testing at Google, https://research.google/pubs/state-of-mutation-testing-at-google/ | Diff-based, few mutants, surfaced at code review. | Seed faults only on the diff under review, to bound cost. |
| Defect removal efficiency and phase containment (secondary sources only) | Escapes are defects found later over all found; needs the phase that introduced and the phase that found each defect. | Two fields per finding. |

## 2. Gap analysis

- **Throughput and deduplication.** `evolution` §4 has no triage step, no index, no priority,
  and three statuses. Any run files; nothing makes it look first. PEP and KEP put an editor
  in front of filing. Nothing limits work in progress, so finished changes queue for review
  while new runs meet the same defect.
- **Closing the loop.** The recurrence signal is prose, so the module-end check is an agent's
  judgement over 49 rows, and it was skipped. It is also placed at the wrong moment:
  recurrence is cheapest to detect when a new proposal or finding is filed.
- **Review depth and cost.** Review works, and §12 already says depth follows evidence. The
  gap is queueing, not depth.
- **Lessons.** The weakest part against the literature: no verification gate, no counters or
  retirement, free-text scope, delivered whole to every run, and any agent may rewrite the
  file. Promotion waits for a second campaign to record a lesson again by chance.
- **Measures and gaming.** The goals' pairing matches DORA's guidance, but most pairs are not
  taken yet, and the tier that caught a defect, which every escape measure needs, is not
  recorded.
- **Protection of checks.** "Weakens a check" is a reviewer's judgement, and the reviewer is
  an agent of the same system. The change takes effect on the working branch before the user
  reads it. DGM and METR show that instruction-level protection fails.
- **Lacking entirely.** A replayable regression set for spec changes; a measured catch rate
  for reviews; a `deferred` or `superseded` status; an owner for the evolution queue.

## 3. Recommendations

### Adopt now, ranked by value against cost

1. **Triage and an occurrence counter.** The assistant acts as editor. One index file, one
   line per proposal: signal, destination, status, priority, occurrences, and the workaround
   until it lands. A run that meets an indexed signal appends an occurrence with its
   evidence rather than filing; the count sets priority. Add statuses `deferred` and
   `superseded-by`. Measure: the share of proposals repeating a signal. Risk: runs
   under-report to avoid the lookup; a wrong merge hides a distinct defect.
2. **Review before launch.** A finished change awaiting review is reviewed before the next
   wave launches, blocking fixes first. Measure: time from finished to landed; runs that
   worked around something already fixed. Risk: launches stall on a slow review. A hard cap
   on work in progress is the agent's inference, not sourced.
3. **A machine-checkable recurrence signal, checked at filing.** Each record row gains a key:
   a finding category plus pattern, an error string, or a gate name. A script matches the
   keys against the module's findings and failed runs, and every new proposal cites the rows
   its signal matches, which marks them `recurred` on the spot. Risk: keys too narrow miss
   recurrences; keep the prose signal beside the key.
4. **Record the tier that caught each finding and the tier that first passed it.** Two
   fields in the review report: this turns every "escape" pair of the goals from not taken
   yet into taken. Risk: the introducing tier misattributed.
5. **Lessons as itemized, scoped, counted entries.** Each lesson gets an id, a path or
   module scope a script can match, its source run, and a status, `provisional`, `held` or
   `retired`. Edits are per-entry, never a rewrite of the file. A lesson is `provisional`
   until the wave's review batch reads its diff or a second run cites it. Runs receive the
   lessons whose scope matches their task, plus the global ones. Retired lessons stay,
   marked. At a module's end, a lesson stated without the target's names is proposed for the
   spec at once. Risk: scope filtering hides a relevant lesson; when unsure, deliver it.
6. **Flag check-weakening by path, not by judgement.** A deterministic list of protected
   paths (gates, lint configuration, the reviewing, testing-gates and evolution skills, the
   reviewer role, the goals). Any diff touching them is flagged automatically; the reviewer
   may add flags and not remove them. Risk: over-flagging tires the user; separate "touches"
   from "the reviewer says it weakens".

### Trial only

7. Hold check-weakening changes off the working branch until the user approves.
   Structurally right, but it blocks campaigns: trial for one module and measure the hours
   blocked.
8. Seeded faults in review batches, one or two per module pass, on the diff only: it
   measures the reviewers' catch rate. Risk: a seeded fault lands; the seed needs a removal
   check.
9. A regression set for the spec: each rejected review and each recurred fix becomes a
   replayable case. Agent reruns are expensive and noisy: run it only for changes to
   protected paths.
10. A concurrent control for doubtful spec changes: old and new spec on the same item at
    once. It doubles the validation's cost.

### Leave alone

At or beyond the practice the agent found: review by a different run on a pinned commit
under the unchanged harness; evidence-only signals; a recurrence signal per landed change;
`confirmed` requiring another campaign; per-campaign working branches; goals the system
cannot edit by itself; the generality test between a lesson and the spec; paired measures.
Not worth adding: archive or population search, which needs a cheap scalar evaluator and
hundreds of rollouts; embedding-based de-duplication at about 80 proposals; a probation
status beside `watched`.

## 4. Not verified, and open questions

- DGM's marker removal is quoted from the authors' blog; the paper's appendix did not load.
- Abstracts only: ADAS (arxiv 2408.08435), GEPA (2507.19457), STOP (2310.02304), reward
  tampering (1908.04734), the Goodhart taxonomy (1803.04585), Google's mutation testing paper.
- Unreadable: the AlphaEvolve paper. Secondary only: defect removal efficiency. Not sourced:
  Kanban limits on work in progress.
- Questions for the user: may a check-weakening change take effect before the user reads it;
  who owns triage, the assistant or a run per wave; may a script, rather than an agent, edit
  the record's statuses; is filtering lessons by scope acceptable, "read by every run" being
  the rule; what budget per module for seeded faults and regression replays.
