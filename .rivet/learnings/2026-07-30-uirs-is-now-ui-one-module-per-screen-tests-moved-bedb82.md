---
title: ui.rs is now ui/ — one module per screen, tests moved with their code
date: 2026-07-30
promoted: true
---

# ui.rs is now ui/ — one module per screen, tests moved with their code

## Observation

Split `apps/tui/src/ui.rs` (2736 prod lines, 60 render fns) into `ui/` on
2026-07-30: `chrome`, `sidebar`, `dashboard`, `detail`, `chat`, `panes`,
`overlays`, `review`, `runs`. `mod.rs` keeps `render()`, `ModalY`/`modal_rect`
and `wrap`.

Mechanics worth reusing next time:

- Children do `use super::*;` to inherit `mod.rs`'s ratatui + crate imports. A
  private `use` is visible to descendants — the same rule that makes
  `mod tests { use super::*; }` work — so no module repeats the prelude.
- Every moved item is `pub(super)`, and **struct fields need it too**.
  `ToolRow`'s fields stayed private at first and only the *test* build caught
  it (E0451): nothing on the prod path constructs one field-by-field.
- `mod.rs` re-exports each child with `use <mod>::*;` so `render()` and the
  tests call everything unqualified, exactly as before the split.
- Tests moved to the module they cover. `line_texts` was the only helper two
  modules needed; it lives in a small `testutil` module rather than copied.

The verification that mattered: `ui_render_tests.rs` was not touched by a
single line, and all 303 tests passed either side of the move. That harness was
written the commit before, against the single-file layout, specifically so it
could hold this one still. Commit `a6edd3a` is in `.git-blame-ignore-revs`.

Gotcha when scripting a split like this: item ranges must be extended
**backwards** over the preceding doc comment. A `///` block sits above its item,
so naive start-line ranges leave every doc comment at the tail of the *previous*
segment — which produces a wall of `E0753: expected outer doc comment`.

## Disposition
Folded into .rivet/context/domains/tui-client.md (ui/ module split note); the generic split-scripting mechanics stay here.
