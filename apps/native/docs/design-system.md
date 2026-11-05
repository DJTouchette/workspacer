# Workspacer design system (native, design v2)

**This is the source of truth.** Where `design-v2.md` or
`design-v2-direction.md` disagrees, this page wins. Every v2 surface is
built from the foundations and components below. A surface never styles
locally, and two tests enforce that:

| Gate | Where | Fails when |
|---|---|---|
| Contrast | `src/ui/system/contrast.rs` | Any component state in any of the 8 built-in themes is under 4.5:1 for text or 3:1 for non-text, plus the two extra rules under [Contrast](#contrast) |
| Design lint | `src/ui/system/lint.rs` | A surface file gains a raw colour, a `px(<number>)`, a `.shadow…(…)` or a `.border_color(…)`, or an allowlisted count no longer matches (the allowlist only shrinks) |
| Gallery | `Screen::Gallery`, `src/ui/system/gallery.rs` | Not a test. It is reviewed in the `system-gallery-<theme>` shots |

The code lives in:

- `src/tokens.rs` (`Tokens`, `Tokens::system()`)
- `src/appearance.rs` (colour roles per theme)
- `src/ui/primitives.rs` (older components)
- `src/ui/system/` (components added in R5, one file each, re-exported as `system::*`)
- `src/ui/decision.rs` (DecisionCard)
- `src/ui/status.rs` (status derivation)

## Using the system

- Everything takes `&Palette` (`let p = self.palette()`; `let t = p.tokens`).
- Components return `Div` or `Stateful<Div>`. The caller adds `.on_click(cx.listener(…))`, children and `.debug_selector(…)`.
- Ids double as debug selectors where noted. Keep existing selectors stable.
- v1 stays pixel-identical. Branch on `self.design().is_v2()` once, high up, and leave the v1 branch alone.

---

## Foundations

### Colour roles (`Palette`, `p.roles`)

| Role | Field | Use |
|---|---|---|
| Canvas | `p.chat` | Conversation and page background |
| Chrome | `p.base` | Sidebar, table heads |
| Surface | `p.surface` | Cards, tables, code blocks |
| Raised | `p.roles.raised` | Menus, popovers, dialogs, tooltips |
| Row fills | `p.selected` (always neutral), `roles.hover`, `roles.pressed` | List, nav and menu rows |
| Edges | `p.border` (dividers), `roles.card_border` (Raised edge), `roles.border_strong` (Overlay edge, inputs, Secondary buttons, off toggle track) | |
| Accent | `p.accent` = `roles.focus` | Links and the focus ring. **Blue means interactive.** It is never a status |
| Primary | `p.primary`, `primary_hover`, `primary_pressed`, `on_primary` | The Primary button fill, the on toggle |
| Status | `roles.status.{attention, fault, working, done, idle}` | Status glyphs and words only (see StatusCell) |
| Identity | `roles.avatar[6]`, `roles.avatar_text`, `roles.provider` | Monogram tiles, provider marks |

- **Amber** means blocked on you.
- **Red** means a named failure.
- **Green** means done or healthy.
- Nothing else borrows these hues. No purple.

### Text: three levels

| Level | Field | For |
|---|---|---|
| Primary | `p.text` (`p.prose` for chat body copy) | Titles, labels, answers |
| Secondary | `roles.text_secondary` | Descriptions, activity rows, inactive nav |
| Tertiary | `roles.text_tertiary` | Meta, timestamps, keycap hints, captions |

All three are at least 4.5:1 on chat, surface, base and raised, and on selected and hover rows. Disabled controls also use tertiary. There is no fourth "faint" grey.

### Type scale (`t.text`, weights `t.weight`)

| Token | Size | Use |
|---|---|---|
| `title` | 24 / 600 | Page titles, welcome question |
| `title_short` | 20 | Dialog titles |
| `heading` | 17 | Section headings in prose, empty-state titles |
| `lead` | 15 | Decision headlines |
| `subheading` | 14 | Card, row and dialog titles; Large buttons |
| `body` | 13 | Body copy, rows, Small buttons |
| `meta` / `caption` / `overline` | 12 | Meta, section headers (sentence case 12/600), keycaps |

- Nothing is smaller than 12.
- Nothing is uppercase.
- Times, counts and stats use `primitives::tabular`.

### Spacing (`t.space`, the 4/8 grid)

`xxs` 2 · `xs` 4 · `sm` 6 · `md` 8 · `lg` 12 · `xl` 16 · `xxl` 24 · `xxxl` 32 · `huge` 48

- Inside a control: `xs` to `md`.
- Between rows: `xxs` to `sm`.
- Between groups: `lg` to `xl`.
- Between page sections: `xxl`.

### Radii (`t.radius`)

| Token | Size | Use |
|---|---|---|
| `xs` | 4 | Keycaps, code spans |
| `control` | 8 | Buttons, rows, inputs |
| `panel` | 12 | Cards, menus, dialogs |
| `composer` | 16 | The composer |
| `full` | — | Badges, chips, toggles, dots |

### Elevation (`system::Level`, `src/ui/system/elevation.rs`)

| Level | Fill | Edge | Shadow | Scrim |
|---|---|---|---|---|
| `Flat` | `chat` | `border` | none | no |
| `Raised` | `surface` | `card_border` | `elevation.raised` | no |
| `Overlay` | `roles.raised` | `border_strong` | `elevation.modal` | yes, when centred (`elevation.scrim_alpha` of `p.shadow`) |

- `elevated(level, &p) -> Div` gives you the fill, edge, shadow and panel radius.
- `Level::{fill, edge, shadow}(&p)` give the parts separately.
- `scrim_color(&p)` gives the scrim colour.
- An overlay always has a shadow. A centred overlay also always has a scrim.
- Menus and popovers are Overlay but anchored, so they have no scrim.

### Motion (`t.motion`, `t.signals()`)

| Motion | Duration |
|---|---|
| Working arc | one turn per `spin_ms` (1000) |
| Working pulse | `pulse_ms` (1400) |
| Dock slide | `panels.slide_ms` (240) |

- Under Reduce motion, every loop holds still.
- Nothing else animates.

### Component measures (`t.system()`)

| Token | Value |
|---|---|
| `glyph_slot` | 16 |
| `nav_row` | 32 |
| `nav_icon_slot` | 16 |
| `nav_tile` | 32 |
| `toggle_width` × `toggle_height` | 32 × 20 |
| `toggle_knob` | 16 |
| `toggle_inset` | 2 (= the focus border) |
| `menu_gap` | 4 |
| `overlay_top` | 96 |
| `overlay_width` | 640 |
| `tile_min` | 280 |
| `danger_lift` | .55 |

---

## Components

Each entry gives:

- what the component is for
- its parts, in order
- its states
- the tokens it reads
- the Rust API and its file
- do and don't rules

### Button

- **Purpose:** an action with a word.
- **Anatomy:** an optional leading icon (14), the label, and an optional trailing keycap hint.
- **Tiers (R5.2):**

| Tier | Look | Use |
|---|---|---|
| `Primary` | Filled `primary`, `on_primary` label | One per surface: the forward action |
| `Secondary` | `border_strong` hairline, no fill | Peers |
| `Tertiary` | Text only | Rows, toolbars, back links |
| `Destructive` | Secondary shape, `error` label | Stop, Deny, Discard, End, Remove. Never brand blue |

- **States:** Rest, Hover, Pressed, Disabled, Focus.
  - Primary moves away from its label on hover and press.
  - Destructive hover and press take the neutral wash, and the label lifts `danger_lift` toward `text`.
  - Disabled Primary is a `selected` fill with a tertiary label.
  - Focus is the 2px `focus` ring.
- **Tokens:** `layout.control_height_sm` 28 (`Small`) or `control_height` 36 (`Large`), `radius.control`, `text.body` or `subheading` / `weight.medium`, `wash.*`.
- **API** (`primitives.rs`):
  - `button(id, label, Option<IconName>, ButtonVariant, ButtonSize, enabled, &p) -> Stateful<Div>`
  - `button_with_icon(id, label, impl Into<Icon>, …)`
  - `keyed_button(id, key, label, variant, size, enabled, &p)`
  - `split_button_parts(id, label, icon, variant, size, enabled, &p) -> (main, chevron)`, joined with `split_button(main, chevron, &p)` or `split_button_with_menu(main, chevron, Option<menu>, &p)`
  - `back_link(id, label, &p)`
  - `button_paint(variant, ControlState, &p) -> (fill, label, edge)` and `button_specimen(label, variant, size, ControlState, &p)` for static states (the gallery and tests)
- **State rules** (`system::actions`):
  - `send_state(can_send) -> (ButtonVariant::Primary, can_send)`
  - `stop_variant() -> ButtonVariant::Destructive`
- **Do:**
  - Use one Primary per surface.
  - Make Send and Queue Primary when they can send, and the *disabled* Primary when they cannot.
  - Use the same tier and slot for every page header's main action (the right end of `page_header`).
- **Don't:**
  - Outline an active Send.
  - Fill Stop or Deny blue.
  - Infer a variant from an id.
  - Use a second filled button beside a Primary.

### IconButton

- **Purpose:** an action shown as an icon only.
- **Anatomy:** a 28 square with a 14 icon. The tooltip gives the label and the shortcut as keycaps.
- **States:** Rest, Hover, Pressed, Disabled, Focus, Selected (a tool whose panel is open: the neutral `selected` fill).
- **Tokens:** `layout.icon_button`, `icon.sm`, `radius.control`.
- **API** (`system/icon_button.rs`):
  - `IconButton::new(id, impl Into<Icon>, label).shortcut("g d").variant(ButtonVariant).enabled(bool).selected(bool).render(&p) -> Stateful<Div>`. The default tier is Tertiary, and the id is also the debug selector.
  - The older `primitives::icon_button` and `icon_button_with_shortcut` remain for existing call sites and are the same look.
  - `overflow_trigger(id, &p)` is the "…" button.
- **Do:**
  - Always pass the label.
  - Pass the shortcut whenever one exists.
  - Use `Concept` glyphs so that no two destinations share an icon.
- **Don't:**
  - Ship an icon-only control without a tooltip.
  - Reuse one glyph for two ideas.

### NavItem

- **Purpose:** a sidebar destination.
- **Anatomy:**
  - Row: the icon slot (16), the label, then the count slot.
  - Tile: a 32 square icon whose label and shortcut are in the tooltip. The count sits absolutely on the top-right corner.
- **States:** Rest (secondary), Hover (hover fill, primary text), Pressed, Selected (`selected` fill, primary text), Focus.
- **Tokens:** `system().nav_row`, `nav_icon_slot`, `nav_tile`, `icon.md`, `text.body` / `weight.medium`.
- **API** (`system/nav.rs`): `NavItem::new(id, icon, label).selected(bool).count(n, CountTone).shortcut("g p")`, then `.render(&p)` for the row or `.render_tile(&p)` for the tile. Both return `Stateful<Div>`. Keep the `nav-*` ids.
- **Do:**
  - Prefer labelled rows.
  - Give each destination a distinct icon.
  - Use `CountTone::NeedsYou` only for what waits on the user now.
- **Don't:**
  - Put a badge inline beside an icon. It shifts the icon.

### StatusCell

- **Purpose:** a state in a table, list or card.
- **Anatomy:** the glyph slot (16, glyph 8), the word (12/500, the kind's text colour), then an optional tertiary detail.
- **States:** every `StatusKind`. **Every state has a glyph:**

| Kind | Word | Glyph | Colour |
|---|---|---|---|
| Working | Working | Ring with a turning arc | `status.working` |
| NeedsYou | Needs you | Filled dot | `attention` |
| NotResponding | Not responding | Hollow clock | `fault` |
| Failed | Failed | ✕ | `fault` |
| Idle | Idle (Waiting to start, Unknown, Ended) | Hollow ring | `idle` |
| Done | Done | Check | `done` |
| Paused | Paused | Pause bars | `idle` |
| Offline | Offline | Dash | `idle` |
| Ready | Ready (Accepting work, Online, Set up) | Ring with a dot | `done` |

- **Tokens:** `system().glyph_slot`, `signals().glyph`, `glyph_stroke`, `text.meta`.
- **API:**
  - `system::StatusCell::new(kind).label("Accepting work").detail("· 3 agents").render(&p) -> Div` (`system/status_cell.rs`). Selector: `status-cell-<Kind>`.
  - `primitives::status_mark(kind, size, &p)` is the glyph that is always present.
  - `StatusBadge` and `status_indicator` stay for sidebar rows, where Idle shows an age instead of a glyph.
  - Derive a session's kind with `status::derive`, never locally.
- **Do:**
  - Map each screen's states onto `StatusKind` in one small function.
  - Use Ready for healthy states.
- **Don't:**
  - Leave a healthy row without a glyph.
  - Draw a bespoke pill or dot.
  - Tint a word that is not amber or red.

### Toggle

- **Purpose:** an on/off setting.
- **Anatomy:** a 32×20 track with a 16 knob, inset by the 2px focus border.
- **States:**
  - On: `primary` track, `on_primary` knob at the end.
  - Off: `border_strong` track (a quiet well, at most 2:1 against its surface), `text_secondary` knob at the start.
  - Disabled: either state at `wash.yielded` opacity.
  - Focus: the ring.
- **Tokens:** `system().toggle_*`, `radius.full`.
- **API:**
  - `system::toggle(id, on, enabled, &p) -> Stateful<Div>` (`system/toggle.rs`)
  - `system::toggle_colors(on, enabled, &p) -> ToggleColors { track, knob }`
  - gpui-component's `Switch` draws the same colours under v2, through `configure_theme_v2` and the vendored `ThemeColor::switch_thumb_off`. Existing Settings switches are therefore already correct.
- **Do:** put the label on the row and the tooltip on the toggle.
- **Don't:** use a dark filled off track. It reads as "on" in light themes.

### TextField / Composer

- **Purpose:** text entry. There is one composer, shared by the welcome screen, New session and the conversation (`launch_task_box_v2`, `composer_v2.rs`).
- **Anatomy:** a `border_strong` field on `surface`, then the toolbar chips (`primitives::toolbar_chip`), then the Send slot.
- **States:** Rest, Focus (ring), Yielded (a decision waits: `wash.yielded`, but still visible and still bordered), Disabled.
- **API:** gpui-component `Input` styled by `configure_theme_v2`; `toolbar_chip(id, leading, label, opens, enabled, &p)`.
- **Don't:**
  - Hide the field while a decision waits.
  - Build a second composer.

### Menu

- **Purpose:** a short list of actions, opened from a control.
- **Anatomy:** an Overlay surface (`menu_width`) holding `menu_item` rows (icon 16, label, optional keycap) and `menu_separator`s.
- **States:** item Rest, Hover (`roles.hover`), Disabled (tertiary), Destructive (`error`).
- **Tokens:** `layout.menu_width`, `row_height`, `system().menu_gap`.
- **API:**
  - `primitives::menu(&p)`
  - `menu_item(id, label, Option<IconName>, danger, enabled, &p)`
  - `menu_item_icon(…)`
  - `menu_separator(&p)`
  - Anchor it with `system::anchored_menu(trigger, Option<menu>, MenuAnchor::{BelowStart, BelowEnd, AboveStart, AboveEnd}, &p) -> Div` (`system/menu.rs`). `split_button_with_menu` uses it.
  - The caller owns the open flag. Close the menu on a choice, on Esc and from `.on_mouse_down_out`.
- **Don't:**
  - Hang a menu from a window corner.
  - Let a menu cover its own trigger.

### Overlay / Dialog

- **Purpose:** a surface that takes the keyboard: a confirmation, the command palette, the shortcuts sheet.
- **Anatomy:** a scrim (`scrim_color`) holding an Overlay panel. For a dialog the panel is `dialog`: the `dialog_header` (20 title plus description), the body, then the `dialog_footer` (right-aligned: Secondary, then Primary or Destructive).
- **States:** open or closed. Panel content follows its components.
- **Tokens:** `elevation.modal`, `scrim_alpha`, `layout.dialog_width`, `system().overlay_top`, `overlay_width`.
- **API:**
  - `system::overlay(Placement::{Center, Top}, panel, &p) -> Div` (`system/overlay.rs`). Mount it as the **last child of the window root**, so it centres on the window.
  - `primitives::dialog(&p)`, `dialog_header`, `dialog_footer`.
  - `system::elevated(Level::Overlay, &p)` for other panels.
- **Don't:**
  - Float an overlay over undimmed text.
  - Centre an overlay on a pane.

### Card / Tile

- **Purpose:** grouped content that rests on a page.
- **Anatomy:** a Raised surface (`p xl`, `gap md`) with a `CardHead` (icon or avatar, title 14/600, StatusBadge, one tertiary meta line, actions on the right), then the body.
- **States:** Rest. A card waiting on you is an `attention_frame` / `waiting_card`.
- **API:**
  - `primitives::card(&p)`, `CardHead::new(title).icon(..).avatar(..).status(..).meta(..).action(..).render(&p)`.
  - `system::tile(&p)` (a card at natural height) inside `system::tile_grid(columns, &p)` (top-aligned, never stretched).
  - `primitives::inset(&p)` for the one recessed well inside a card.
- **Do:** use `tile_grid` for cards of different lengths (Fleet).
- **Don't:**
  - Nest cards.
  - Stretch a short card to its row (that is `page_grid`; use it only for equal content).

### Table

- **Purpose:** dense rows: Jobs, Machines, History, the Fleet table.
- **Anatomy:** a `page_table` (surface, hairline, clipped) holding a `page_table_head` (on `base`, 12/600 secondary) and `page_table_row`s. The cells come from `Col::{Fixed, Keep, Flex, Auto}` widths in `pages()`. The status column is a **fixed slot** holding a StatusCell. The actions column is a **fixed slot**: `action_slots(vec![action_slot(width, Option<control>)…])`, with `slot_button` for buttons whose label swaps.
- **States:** row Rest and Hover (add `.id` plus a hover fill). Empty slots keep their width.
- **API:**
  - `primitives::page_table`, `page_table_head(&[(label, Col)], &p)`, `page_table_row`, `page_cell(Col)`, `action_slot`, `action_slots`, `action_slots_width(n, &p)`, `slot_button`
  - `table(headers, rows, &p)` for simple tables
  - `table_style(&p)` (Markdown tables share it)
- **Don't:**
  - Leave a status cell blank. Use Idle "Unknown", or drop the column.
  - Let hover actions cover a value.

### DecisionCard

- **Purpose:** a decision waiting on the user: a tool approval, a question, a plan, a job proposal.
- **Anatomy:**
  - The question headline (15/600, code spans wrap).
  - The reason.
  - The full request (mono, when it is long).
  - Where it applies (folder · host, tertiary).
  - The actions: Allow (Primary), alternates (Secondary, "Always allow" with its exact rule in the tooltip), Deny (Secondary split with "Deny with note…"), then Details (Tertiary, expanding the request in place).
- **States:** keys on or off (keycaps appear only on the decision that the number keys answer), Details open, note open (the note's send owns the Primary; "Deny and tell it" is Destructive).
- **API** (`decision.rs`):
  - `Decision::new(selector, Choice::new(id, label).key(Some(1)), Frame::{Card { compact }, Inline})`. Fill in `headline`, `reason`, `request`, `folder`, `host`, `alternates`, `deny`, `deny_note`, `details` and `keys`, then call `.render(&p)`.
  - `Choice::new(id, label).key(..).tooltip(..).enabled(..).on_click(..)`.
- **Don't:**
  - Stack the actions one per line.
  - Make Deny blue.
  - Show keycaps on an unfocused card.

### PageShell

- **Purpose:** one template for every page.
- **Anatomy:** the page scroller (`Workspace::page_view`) gives every page the same top offset and gutter. `page_header` contains:
  - the back link
  - the 24/600 title
  - a one-line description
  - the actions in **one** right slot

  The body follows, `space.xxl` below. The header and the body share one width rule.
- **Widths:** `PageWidth::Wide` (`pages().wide`, 1240) for tables and grids; `PageWidth::Form` (`pages().form`, 760, left-aligned) for New session, Agent setup, Session details, Model, Handoff and Pull request.
- **API:**
  - `system::page_shell(header, Option<notice>, body, PageWidth, &p) -> Div` (`system/page.rs`), passed to `page_view(id, pages().wide, false, …)`.
  - `primitives::page_header(back, title, description, actions, &p)`, `back_link`.
  - Used by `feature_page_v2` (every feature page) and New session.
- **Don't:**
  - Add a page-specific top offset. `short_page_top` is gone.
  - Let header actions run wider than the content.

### CountBadge

- **Purpose:** a count ("3", "99+").
- **Anatomy:** a fully rounded chip, 12/600 tabular figures.
- **States:** `CountTone::Neutral` (a wash with secondary text), `NeedsYou` (amber fill, only for what blocks the user now), `Failed` (red fill, a named failure).
- **API:** `primitives::count_badge(label, CountTone, &p)`, `count_text(n)`.
- **Don't:** use `badge(…)`, the old solid flag, for counts.

### Keycap

- **Purpose:** a key in a tooltip, a sheet or a hint.
- **Anatomy:** a 12 mono label in a hairline box (`kbd`). A **hint** (`keycap_hint`) trails a control: tertiary with a faint edge, or the label colour with no box on a Primary fill.
- **API:** `primitives::kbd(label, &p)`, `keycap_hint(key, on_fill, &p)`. In tooltips, spaces in the shortcut separate the keycaps (`tooltip_view(label, Some("g n"), &p)`; `tooltip_view_keys` takes an owned shortcut).
- **Don't:** tint a keycap, or join keycaps without gaps.

---

## Patterns

### Page anatomy

The page scroller, then PageShell (header, then an optional notice, then body sections `space.xxl` apart). Each section has a `section_header`, then a table, tile grid or form.

- **Empty:** `empty_state(icon, title, body, &p)`. Say why it is empty and give one action.
- **Loading:** keep the layout and show tertiary "Loading…" in place. Never a spinner over a blank page.
- **Error:** a `notice_line` under the header, naming what failed, with Retry (Secondary).

### Conversation hierarchy

Prose is above activity, and activity is above meta.

- Prose and headings are primary.
- Activity rows (Ran, Edited, Changed 2 files, subagents) are 13 secondary, with the verb in medium.
- Meta is tertiary.
- Turns are 32–40 apart.
- A finished turn folds into one summary row ("Worked 5m 40s · 6 tools"), coloured by its **final** outcome. A failure that later recovered is not red at the turn level.
- The composer stays a visible bordered field, even while a decision is pending.

### Fleet card anatomy

- A `tile` at natural height.
- Title (14/600), then elapsed time.
- StatusCell, then project, model and context.
- Up to three `EventRow`s.
- The ask as an inline `Decision` (`Frame::Inline`), with Allow and Deny side by side.

The focused tile alone shows keycaps and the focus ring outside its attention bar. Waiting tiles use `attention_frame`.

### Empty, loading and error states

See Page anatomy. The same rules apply inside cards and panels: one line, tertiary, and one way out.

---

## Contrast

`system/contrast.rs` checks every theme in `Appearance::ALL`:

- **Text tiers, prose and links** on chat, surface, base, raised, selected and hover: 4.5.
- **Status words:** 4.5. **Status glyphs:** 3.
- **Every Button tier** at Rest, Hover and Pressed, on every surface, with washes composited: 4.5.
- **Primary fill:** 3. A **light** Primary fill (one with a dark label) needs 4.5, so it never reads as disabled. Nord's old frost Allow was 3.7.
- **Focus ring:** 3.
- **Count badges:** 4.5.
- **Keycaps**, including on Primary at rest, hover and pressed: 4.5.
- **NavItem** rest, hover and selected: 4.5.
- **Menu items and destructive items** on raised and hover: 4.5.
- **Avatars:** 4.5.
- **Toggle and Switch:**
  - on knob vs track: 3
  - off knob vs track: 3
  - on track vs surface: 3
  - off knob vs surface: 3
  - off track vs surface: **at most** 2 (Light's dark off pills were 6:1)

## Design lint

`system/lint.rs` scans every `src/ui/**` file that reads v2 (`is_v2()`, `_v2`, `p.tokens`, `primitives::`, `system::`), except the component layer.

- **Rules:** raw colour literals, `px(<number>)`, `.shadow…(…)`, `.border_color(…)`.
- **Allowlist:** one table, `ALLOWED`, grouped by file, with a reason per entry:
  - `TO_MIGRATE`: v2-only files
  - `MIXED`: shared v1/v2 files whose v1 branch stays
- **Migrating a surface:** move it onto components and tokens, run `cargo test --features ui-tests system::lint`, then lower or delete its entries. The test fails until the counts match exactly, so the list only shrinks.
- **Printing the table:** run with `DESIGN_LINT_PRINT=1`.
