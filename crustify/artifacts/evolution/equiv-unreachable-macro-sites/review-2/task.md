This run is not a review batch: it reviews one change to Crustify, as the `evolution`
skill's Review section describes, and changes, commits and lands nothing. Its task is
`/target/libgit2-crustify/crustify/artifacts/evolution/equiv-unreachable-macro-sites/review-2/task.json`.
You run under Crustify's `libgit2-crustify/devel`, this checkout.

This is a second, narrow review. Review-1 rejected `73b731e` with three fixes, which `e99130a`
makes: "no function the raw bindings reach runs any of its expansions, directly or through
the functions it calls" for "the raw bindings reach none of them"; "unit tests" for
"`unit_pub` tests"; and "a macro that implements logic". It also adds the reason review-1
suggested. Review-1's verdict is in `/target/libgit2-crustify/crustify/evolution/equiv-unreachable-macro-sites/PROPOSAL.md`. The change:
`git -C /crustify-rs/crustify show e99130a`; what review-1 read: `show 73b731e`.

Check only that the three fixes are made as review-1 asked, that nothing else changed but the
reason clause, that the text now weakens no check, and that the row matches. Do not repeat
review-1's other checks, and do not raise again what it listed as not required. The change
is on a protected path: the user reads your verdict. Verdict, approved or rejected, in a few
lines, in your final message. Create no scratch.
