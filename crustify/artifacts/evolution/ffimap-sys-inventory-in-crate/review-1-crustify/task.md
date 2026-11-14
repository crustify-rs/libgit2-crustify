This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/ffimap-sys-inventory-in-crate/review-1-crustify/task.json`.
You run under Crustify's `libgit2-crustify/devel` (`a49c1b1`), this checkout.

- The proposal: `/target/libgit2-crustify/crustify/evolution/ffimap-sys-inventory-in-crate/PROPOSAL.md`.
- The change, one commit: `git -C /crustify-rs/crustify diff a49c1b1 3dcc464`: one paragraph of
  the `ffimap` tool skill, `skills/tools/ffimap/SKILL.md`.

It is the Crustify half of a change whose other half is ffimap `a0797e6`, reviewed apart: ffimap's
`--sys-crates` takes a module of the scanned crate, written `crate::<path>`. The paragraph
has a scan name such a module. Check it against the `bindgen` skill, which puts a campaign's
raw bindings at `crate::sys`, against the rest of the `ffimap` skill, whose scans name "its
`-sys` crates", and against the tool skills' rules in `authoring`. Say whether the paragraph
sits in a section the `evolution` skill's protected paths name (a tool skill's Testing,
Reviewing or Audit sections and landing scans), in which case the user reads your verdict
before it lands. It must not land before the ffimap half, since the form it names does not
exist until then.

The user's instruction for reviews: run every check of your role and of the Review section,
and report all you find. Require a fix only for what would lead a run wrong, leave a signal
unfiled, let a check be weakened unseen, state a rule in the target's names, or contradict
another rule. The rest is listed apart, as not required. Verdict, approved or rejected with
the fixes required, briefly, in your final message. Create no scratch.
