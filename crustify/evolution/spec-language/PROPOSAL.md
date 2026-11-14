# The spec's language; the goals rewritten under it

Status: `open`: the change is one commit on Crustify's
`libgit2-crustify/evolution/spec-language`, `f20edc9`, at the user's direction on 2026-11-14,
and awaits review-4. Four earlier commits (`73ca306`, `01830f8`, `437cc1d`, `44186d8`), which
three reviews rejected, are replaced by it. It rewrites `GOALS.md`, a protected path, so it
lands only with the user's approval.

## Signal

The user's direction: the language of the spec and of any text should be simple and
technical enough for programmers to understand, consistent, and not bloated, without losing
information.

## Goal

**Consistency** (goal 11), applied to the harness's own text: one term per thing, one form
for a condition, a list, a definition. **Maintainability** (goal 6): text a reader can
change without misreading it. No measure of either counts prose yet; the change's own
recurrence signal stands in.

## Evidence

- `GOALS.md` at `754e2ce`, written by the assistant hours earlier in long periodic
  sentences: its rewrite found nine sentences that could not be restated without choosing
  a meaning (which of the scan's counts fall as a module is ported; whether "the regions" is
  a figure of its own; what "it", "one", "its" and "those" refer to in four measures; whether
  two cases are examples or a definition; which scan goals 3 and 6 mean). The author settled
  each in `01830f8`.
- The assistant's first, stricter rewrite of a rule of `versioning` (in the session of
  2026-11-14) dropped four facts while shortening it from 88 words to 53, which is why the
  rules are relaxed and carry the test that every fact stays.
- The standards the rules draw on, not adopted whole: ASD-STE100 (sentence length, one
  meaning per word), ISO 24495-1 (plain language), EARS (a condition before its consequence).

## Destination

The spec: `authoring`, a new Language section; `GOALS.md`, its first application;
`EVOLUTION.md`, the row.

## Change

Crustify `f20edc9`:

- `authoring` §5, Language, one paragraph: Crustify's texts and a campaign's are written
  plainly, as the Google developer documentation style guide describes (one term per thing,
  short sentences, a condition before its consequence, lists for parallel items); a rule
  keeps its reason; a rewrite keeps every fact of the old text, which its review checks.
  `authoring`'s description names the language and sends a run that files a proposal or a
  lesson to it.
- `GOALS.md` rewritten: a heading per goal, its statement, then **Measure**, **Paired with**
  and **Not taken yet** entries; "pair", "tier" and "escape" defined once, in the opening.
  It is the text reviews 1 to 3 read, at `44186d8`, with two sentences back in their old
  order (goals 5 and 7).

The earlier commits gave the section seven numbered rules, four of them firm. Three reviews
rejected them on what "clause" and "condition" cover, each fix adding a definition the next
review found an edge in. The user judged the section too complicated on 2026-11-14 and chose
a public standard in its place, with the two rules that guide lacks.

## Record

The row for `EVOLUTION.md`, copied in when the change lands:

| change | from | landed as | signal answered | recurrence signal | status |
|---|---|---|---|---|---|
| `authoring` gains a Language section: texts are written plainly, as the Google developer documentation style guide describes, a rule keeps its reason, and a rewrite keeps every fact; `GOALS.md` is rewritten under it, every goal, measure, pair and marker kept | `libgit2-crustify`, `spec-language` | crustify `f20edc9` | the user's direction: the spec's language should be simple, consistent and not bloated, without losing information; the goals were written in long periodic sentences | a review finding, or a user's correction, that a rewrite dropped a fact or a rule's reason | watched |

## Validation

The subagent's checklist of 96 facts, each with its old and new line, in its report (the
assistant's session of 2026-11-14); the review checks it again.

## Review

`review-1` (session `2fcced49-5794-4bd5-8dcf-ed8f2ca10aa8`, Opus 5.5, `$1.38`, 6m58s), on `01830f8`: **rejected**, with seven
small fixes; every fact of the old `GOALS.md` is in the new one by its own list of about 65,
and the change weakens no check. Required, and made in `437cc1d`:

1. the rewrite broke rule 3 in five series, and the section's own opening broke the
   refinement: the rule now lists clauses and may leave short names inline;
2. "escape" was used before it was defined;
3. "regions" named both unsafe regions and coverage: the second is "region coverage";
4. the record's row misstated rule 4 and left out the refinements;
5. `authoring`'s description did not send a run that files a proposal to the rules;
6. the scope fell short of "any text": it names every prose text now;
7. readings the rewrite settled without the author: "the later tiers are" closed a list
   the old text left open, and is "later tiers include" again; the other four are the
   author's.

It found all nine of the author's settled readings allowed by the old text, two of them
(which counts fall; "a rule was loosened") no more likely than another reading on the text
alone. Its optional wordings are taken where cheap: five sentences over the default length
are split, a measure is not called a count, "escape" stays a noun.

`review-2` (session `dceee456-d61e-448a-816e-1fad7483835b`, Opus 5.5, `$0.92`, 4m33s), on `437cc1d`: **rejected**, on six
small fixes; review-1's fixes 2, 3, 4, 5 and 7 hold, and nothing weakens a check. Its report
is the longest `result` of `crustify/artifacts/evolution/spec-language/review-2/stream.jsonl`.
Among its required fixes: rule 3 as reworded puts its condition after its consequence, and
its two sentences overlap, "short" and "clause" being unbounded (A, B); splitting goal 11's
pair left its reason on one of the two measures only (C); goal 3's pair changed meaning when
its restriction became a sentence of its own (D); the scope still reads as a full list and
leaves texts out (E). Made in `44186d8`.

`review-3` (session `5fdd3f2e-09a0-4e12-8429-6db7b85f945e`, Opus 5.5), on `44186d8`:
**rejected**, on six fixes: three goal sentences that broke rule 3 as reworded, and three
gaps in the new wording of rules 2 and 3. Every fact of the old `GOALS.md` was kept and
nothing weakened a check. Its report is the longest `result` of
`crustify/artifacts/evolution/spec-language/review-3/stream.jsonl`. The fixes are not made:
the user replaced the numbered rules with the reference to a public guide, as `f20edc9`, and
the rules the fixes were on are gone.

`review-4` (session `b2fd3b94-7460-43fb-a815-23297e99e159`, Opus 5.5, `$0.85`, 3m05s), on `f20edc9`: **approved**, with
no fix required. `GOALS.md` keeps every fact of `5933a5d`, the two restored sentences match it
word for word, the provider build is clean and nothing weakens a check. Listed apart, not
required: the guide asks for unnumbered headings and a serial comma, which Crustify's texts
do not follow, the guide itself putting a project's own usage first; and the texts no review
reads are not checked for kept facts. `GOALS.md` is protected: the change waits for the
user's approval.
