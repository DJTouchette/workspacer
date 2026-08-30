---
title: WAKE_COST_NOTE length is load-bearing: the nodes overlay is height-limited
date: 2026-08-25
confidence: high
suggested_doc: tui-client
promoted: true
promoted_to: tui-client
---

# WAKE_COST_NOTE length is load-bearing: the nodes overlay is height-limited

## Observation
`apps/tui/src/nodes.rs::WAKE_COST_NOTE` cannot be extended freely. It is wrapped and rendered inside the fixed-height nodes overlay (`apps/tui/src/ui/nodes.rs:170`), so adding words to it pushes content off the top of the overlay. Adding one clause ("or the phone") broke TWO render tests, and only one of them was about the note: `the_overlay_prints_what_a_wake_costs_beside_the_action` failed because "not this one" now straddled a wrap boundary (`joined()` concatenates padded screen rows with \n, so any prose assertion is wrap-sensitive), and `the_overlay_surfaces_a_crash_record_and_failed_wakes` failed because the extra wrapped line scrolled the "did not end cleanly" crash notice out of the visible overlay entirely. Dropping two words ("the desktop, web app or phone" rather than "the desktop, the web app or the phone") fixed both.</observation>
<parameter name="impact">The second failure is the dangerous one: a longer note silently evicts the node's crash record from the only place a TUI user ever sees it. The render tests are the guard, and their failure message points at the note rather than at the eviction, so it reads like a wrap-boundary nuisance when it is actually content loss.

## Recommendation
Treat the overlay's prose strings as budgeted, not free-form. When editing WAKE_COST_NOTE or any other sentence in `nodes.rs`, run `cargo test` in apps/tui and read the rendered overlay dump in the failure output rather than just patching the assertion: if a line other than the one you edited disappeared, you spent height you did not have. Prose assertions over `joined()` are wrap-sensitive by construction; if a future edit genuinely needs the length, add a whitespace-collapsing helper instead of reflowing the sentence to suit the wrap.</recommendation>
<parameter name="related_paths">["apps/tui/src/nodes.rs", "apps/tui/src/ui/nodes.rs", "apps/tui/src/ui_render_tests.rs"]
