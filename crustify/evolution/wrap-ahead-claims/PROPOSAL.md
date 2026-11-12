# An item wrapped ahead is held by one batch across the campaign

Status: `landed`, as crustify `93674e1` (made in another workspace on
`review/evolution/wrap-ahead-claims`) and `66e2ae8` (the assistant's follow-up), on `devel`,
with the user's approval on 2026-11-12.

## Signal

Skills that leave a conflict no rule resolves (`evolution` §2: a defect in the spec).

A wrap batch ahead of its consumer held "only what has not landed", judged on its
consumer's wave branch. Modules that run at once do not see each other's branches. So two
modules that both needed an unlanded item of a third each bound it, in the same home, and
the second integration met a conflict that the additive rules do not cover.

## Evidence

A read of the spec for consistency, not a campaign's run: the commit says no duplicate was
looked for in a campaign's history. The gap is live in this campaign, which runs several
modules' waves at once.

## Destination

The spec: `scheduling` (Wrap batches), `versioning` (Progress, Integration, Lifecycle),
`modules-schema` (Status), `binding-workflow` (Waves and batches), and the `wavefront`
tool skill (Scheduling › Wrap batches).

## Change

- **One holder per item.** A wrap batch ahead holds only what no batch of `schedule.json`
  holds on any branch, a failed batch aside. A wave that needs an item another module's
  run holds waits for that run's integration. It then starts in a run of its own module,
  forked after the integration.
- **Freezing.** Each module a wrap-ahead batch homes an item in is frozen when the batch
  launches. That `status` is committed with the progress records.
- **Integration.** A module file in which two runs homed different items merges
  additively.
- **Follow-up (`66e2ae8`).** The section's opening and its closure sentence now leave out
  what another batch holds. The `wavefront` skill names the section it defers to.

## Validation

Read for consistency against the skills' other references to what has landed (`git grep
"not landed\|has landed"`). There is no run to compare, since no wave has run under it yet.
It takes effect from the next wave.
