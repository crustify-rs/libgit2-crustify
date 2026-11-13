This run is not a review batch: it reviews one change to Crustify, as the `evolution` skill's Review section describes, and changes, commits and lands nothing. You run under the campaign's working branch of Crustify, `libgit2-crustify/devel` (`10df2c5`).

- The change: crustify `libgit2-crustify/evolution/sys-bindings-layout` at `63b92be`, i.e. `git -C /opt/crustify diff 10df2c5 63b92be`, one commit. Read it with git, or in `/opt/crustify.wt/sys-bindings-layout` (read-only). Judge the change; your own skills are the working branch's.
- The proposal: `/target/libgit2-crustify/crustify/evolution/sys-bindings-layout/PROPOSAL.md`. The signal is the user's decision, which is not under review: the raw bindings' dir is `sys/<link-unit>/` and the Rust module follows it (`crate::sys::<link_unit>`, imported as `sys`), for this campaign and later ones.

Check, citing file and line:
1. Completeness: every place in Crustify (skills, agents and the generated `providers/claude/agents/`, templates, the tool skills, README or docs) that still names the old layout or module (`ffi/<link-unit>/`, `ffi/` as the bindings' dir, `src/ffi.rs`, `crate::ffi`, the `ffi` module, `ffi::<name>` as a raw binding), and anything the rename makes ambiguous or wrong (for example a collision with `std::ffi`/`core::ffi` wording, or with `ffi_import`/`ffi_export`, which deliberately keep their names).
2. The Review section's checks: placement and wording per `authoring`, no rule in the target's names, the validation the proposal states (rerun the grep and `python3 providers/claude/compose.py build` in a `git archive` copy of `63b92be` under a temp dir, diffing `providers/claude/`), the `EVOLUTION.md` entry's recurrence signal, and whether it weakens any check.

Give your verdict, approved or rejected with why, briefly, in your final message.
