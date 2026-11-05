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
| Primary | `p.primary`, `primary_hover`, `primary_pressed`, `on_primary` | The Primary button fill |
| Toggle | `roles.toggle_on` (the accent, deepened until white stands ≥ 3:1 on it), `roles.toggle_edge` (≥ 3:1 on every surface), `roles.toggle_knob` (white) | The toggle and the Settings switch |
| Status | `roles.status.{attention, fault, working, done, idle}` | Status glyphs and words only (see StatusCell) |
| Needs-you mark | `roles.status.attention_fill`, `roles.status.on_attention` | R8.7: amber as a *mark* (the Needs-you dot, the attention bar, the amber count badge) and the label on that badge. Dark themes: the text amber. Light themes: a brighter amber at ≥ 3:1, because text amber at 4.5:1 reads as muddy brown in a dot |
| To review | `roles.review` | R8.1: the second want-you state's own hue, a violet per theme. Glyph (the half ring), word and count badge fill. Never amber |
| Identity | `roles.avatar[6]`, `roles.avatar_text`, `roles.provider` | Monogram tiles, provider marks |
| Warning | `roles.caution` (`Intent::Caution`) | The capacity warning hue (R6.2): an orange, kept apart from amber and red in every theme (the separation test), readable as text. Meters at 80% and over, and their number |
| Scrim | `roles.scrim_alpha` | How much an overlay dims the whole window, sidebar included: 45–55% in every theme (dark 0.5, light 0.45) |

- **Amber** means blocked on you (Needs you). Nothing else is amber: warnings, meters, Full access and links use the warning hue or neutral.
- **Violet** (`roles.review`) means finished with changes to look at (To review).
- **Red** means a named failure.
- **Orange** (the warning hue) means a capacity running out: a meter at 80% or more.
- **Green** means done or healthy.
- Nothing else borrows these hues. Avatar tiles leave out amber, red and violet.

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
| `Overlay` | `roles.raised` | `border_strong` | `elevation.modal` plus `elevation.contact` (a tight shadow that keeps the edge visible on a dark page) | yes, when centred or when it takes the keyboard (`roles.scrim_alpha` of `p.shadow`) |

- `elevated(level, &p) -> Div` gives you the fill, edge, shadow and panel radius.
- `Level::{fill, edge, shadow}(&p)` give the parts separately.
- `scrim_color(&p)` gives the scrim colour.
- `edge(el, Side::{All, Top, Bottom, Left, Right}, level, &p)` draws a hairline in `level`'s edge colour: `Flat` for dividers (a header strip's underline, a footer's rule, the sidebar's frame), `Raised` for a card's edge, `Overlay` for an input or keyboard-highlighted row. It is the one way a surface draws a rule; never call `.border_color` in a surface.
- An overlay always has a shadow. A centred overlay also always has a scrim.
- **Overlays mount at the window root** (`Workspace::shell`, as deferred children), so the scrim covers the whole window, sidebar and header included. Never mount one inside a pane.
- Menus are Overlay but anchored, so they have no scrim. A popover that takes the keyboard (the model popover) hangs from its control but sits over a window scrim, so it never floats over undimmed text it does not own.
- `floating(el, &p)` adds the floating shadow (the composer, Jump to latest: above the transcript, under an Overlay); `lifted(el, level, &p)` adds a level's shadow to a surface that draws its own fill and edge (`src/ui/system/edge.rs`).

### Edges (`system::edge`, `src/ui/system/edge.rs`)

A surface never picks a border colour. It asks for a side and a tone:

| Tone | Colour | Width | Use |
|---|---|---|---|
| `Divider` | `border` | hairline | Header strips, panel splits, the event guide |
| `Card` | `card_border` | hairline | A Raised edge drawn without `elevated` |
| `Strong` | `border_strong` | hairline | Inputs, Overlay edges |
| `Quote` | `border` | `stroke.focus` | The rule beside quoted text |

- `edge(el, Side::{All, Top, Bottom, Left, Right}, EdgeTone, &p)` on any `Styled` element; in a builder chain, `.map(|d| system::edge(d, Side::Bottom, EdgeTone::Divider, &p))`.
- `framed(el, &p)` is `edge(el, Side::All, EdgeTone::Card, &p)`.

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
| `Destructive` | Secondary shape, `error` label | Stop, Discard, End, Remove. Never brand blue |
| `Danger` | Filled `error`, the theme ink that reads best on it (`danger_label`) | R8.2: **only** the armed second step of a destructive confirm ("Press again to delete", "Remove job and history") |

R7.2/R8.2: these variants draw three tiers plus Danger: Filled (`Primary`), Bordered (`Secondary`, or `Destructive` with its red label), Text (`Tertiary`), and Danger (`Danger`, `actions::armed_variant()`). Each verb's tier is fixed in [the tier table](#button-tiers-r72-r82) (`system::actions::Verb`). Deny is a **neutral** Bordered (`Secondary`): the red label is kept for what removes something, and the only red fill is an armed confirm.

- **States:** Rest, Hover, Pressed, Disabled, Focus.
  - Primary moves away from its label on hover and press.
  - Destructive hover and press take the neutral wash, and the label lifts `danger_lift` toward `text`.
  - Disabled Primary (and Danger) is a `selected` fill with a tertiary label.
  - Danger leans its fill toward its label on hover (`wash.hover`) and press (`wash.pressed`). A keycap on it takes the label colour (`keycap_in`).
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
  - `armed_variant() -> ButtonVariant::Danger` (R8.3)
- **Do:**
  - Use one Primary per surface. R8.2: every container's one forward action (Send, Commit, Ask agent to fix, Start, Allow; `Verb::Forward` for those that are not an answer) is Filled when it is enabled. The one exception: while a decision is pending, the decision holds the single Filled button.
  - Make Send and Queue Primary when they can send, and the *disabled* Primary when they cannot.
  - Use the same tier and slot for every page header's main action (the right end of `page_header`).
- **Don't:**
  - Outline an active Send.
  - Fill Stop or Deny blue, or give Deny a red label.
  - Fill anything red except an armed confirm.
  - Infer a variant from an id.
  - Use a second filled button beside a Primary.

### IconButton

- **Purpose:** an action shown as an icon only.
- **Anatomy:** a 28 square with a 14 icon. The tooltip gives the label and the shortcut as keycaps.
- **States:** Rest, Hover, Pressed, Disabled, Focus, Selected (a tool whose panel is open: the neutral `selected` fill).
- **Tokens:** `layout.icon_button`, `icon.sm`, `radius.control`.
- **API** (`system/icon_button.rs`):
  - `IconButton::new(id, impl Into<Icon>, label).shortcut("g d").variant(ButtonVariant).enabled(bool).selected(bool).render(&p) -> Stateful<Div>`. The default tier is Tertiary, and the id is also the debug selector.
  - `.shortcut_opt(Option<&str>)` when the shortcut depends on a setting (modal keys).
  - `.badge(Some(el))`: a trailing fact beside the icon (the Tasks count, "Branch +3 −1"); the button widens and keeps its height.
  - `.dot(Some(selector))`: an accent dot on the corner (Settings: an update is ready); `.dot_color(Some(selector), role)` in a status role (a running shell). Absolute, so the icon never moves.
  - `.icon_size(t.icon.md)`: the header strip's tools draw a 16 icon.
  - `.text(Some("Files"))`: a short visible label after the icon (R6.5). The header's tools carry one while the conversation column is at least `shell().toolbar_labels_at` (1100) wide ("Changes +17 −3", "Files", "Terminal", "Tasks 3"), and are icon-only below it (counts stay). A failed background task escalates to the toolbar (r6-qa): Tasks carries a Failed count badge (`CountTone::Failed`, `title-tasks-failed`) beside the running count, and its tooltip says how many failed.
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
  - `.hint("Scratch · files agents wrote for you to review and run")`: a one-line plain definition for the tooltip (the label otherwise).
  - `.count_id(selector)`: the count badge's debug selector.
  - `.enabled(false)`: a destination that cannot open now (New session in the demo): tertiary, no hover, not focusable.
- **The sidebar** (r5-shell): seven 32 rows, no gaps: New session, Fleet, Projects, Jobs, History, Machines, Scratch. Settings is the footer's gear (an IconButton with the update dot). The collapsed rail draws the same list as tiles (`*-rail` ids), New session at its top. Glyphs: Plus, `Concept::Fleet`, Folder, Calendar, `Concept::History`, `Concept::Machines`, `Concept::Scratch` (a sticky note: never a second folder).
- **Do:**
  - Prefer labelled rows.
  - Give each destination a distinct icon.
  - Use `CountTone::NeedsYou` only for what waits on the user now.
- **Don't:**
  - Put a badge inline beside an icon. It shifts the icon.

### StatusCell

- **Purpose:** a state in a table, list or card.
- **Anatomy:** the glyph slot (16, glyph 8), the word (12/500, the kind's text colour), then an optional tertiary detail.
- **States:** every `StatusKind`. **The status table** (R6.2) is the one source: `primitives::STATUS_TABLE`, read by `StatusKind::{label, color, text_color}`, `StatusCell`, `status_mark`, `status_glyph` and `StatusBadge`. **Every state draws a glyph** (r7-status: at most seven shapes, shared only within one colour; a test keeps the words distinct):

The table itself, its seven glyphs and the words, are in [Status and vocabulary (R7)](#status-and-vocabulary-r7).

- Words: amber and red states carry their hue; Working, Ready and Done are secondary; the rest tertiary.
- A parent session with a stalled subagent reads **Needs you** (amber, in the Needs you group, its ask from `child_stall_reason`); the stalled subagent itself reads Not responding, with Nudge, Stop and View output wherever it appears.
- r7-status (R7.3): Idle draws its ring everywhere, sidebar rows included (the age follows the title).
- Agent readiness uses the table everywhere (r6-qa, r7-status): Ready shares Done's green check, Not installed is Offline's dash, Not checked is Unknown's ring; the word says which.
- **Compact session list** (r6-qa): one line throughout, `layout.sidebar_row_compact` (24) tall. A Needs-you ask follows the title inline, faint, taking only the room the title leaves.
- An ended session (`status::derive` keeps it Idle in the Ended group) shows Ended's glyph on its badge (`Status::badge`).
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
- **States (R6.1):**
  - On: the accent track (`roles.toggle_on`) and a white knob at the end.
  - Off: a light track (`roles.hover`, at most 2:1 against its surface), a white knob at the start, and a hairline (`roles.toggle_edge`) around the track and the knob.
  - Disabled: either state at `wash.yielded` opacity.
  - Focus: the ring.
  - Checked visually in every theme in the gallery (`system-gallery-<theme>`) and in Settings (shot 39).
- **Tokens:** `system().toggle_*`, `radius.full`.
- **API:**
  - `system::toggle(id, on, enabled, &p) -> Stateful<Div>` (`system/toggle.rs`)
  - `system::toggle_colors(on, enabled, &p) -> ToggleColors { track, knob, edge }` (`edge` is `Some` when off)
  - gpui-component's `Switch` draws the same colours under v2, through `configure_theme_v2` and the vendored `ThemeColor::{switch_on, switch_thumb_off, switch_border}`. Existing Settings switches are therefore already correct.
- **Do:** put the label on the row and the tooltip on the toggle.
- **Don't:** use a dark filled off track or a dark knob. Either reads as "on" in light themes.

### TextField / Composer

- **Purpose:** text entry. There is one composer component, used by the welcome screen, New session (`launch_task_box_v2`) and the conversation (`render_composer_v2`). Pages own where it sits; the composer owns how it looks.
- **Anatomy:** the field (`surface`, a hairline edge, `radius.composer`, the floating shadow), holding the body: draft attachments, the text (gpui-component `Input` with `appearance(false)`), then the toolbar. The toolbar's leading side holds attach (IconButton) and context chips (`toolbar_chip`: folder, model, context meter), which shrink and clip first. Its trailing side holds the status, then the Send slot, which never shrink.
- **States** (`system::ComposerState`):

| State | Edge | Body | Placeholder | Send |
|---|---|---|---|---|
| `Rest` | `border_strong` | full | "Ask anything, or describe a task…" | Primary, disabled until there is a draft |
| `Focus` | `focus` | full | as Rest | Primary when there is a draft |
| `Yielded` (a decision waits) | — | R8.4: **hidden**. While an approval, a plan or a structured question waits, the conversation's composer does not draw at all; its input moves into the card (see "Decision and reply" below). `Yielded` remains for a usage limit, which does not take the slot | "Reply — the request stays open" | **Secondary**: the decision card owns the Primary |
| `Disabled` (offline, busy) | `border_strong` | `wash.yielded` | the connection's words | disabled |

- **Send slot:** Send (`system::composer_send_state(can_send, deciding)`), Queue (a Primary split button while the agent works and there is a draft; its menu holds Send now and After this turn) or Stop · Esc (Destructive while the agent works and nothing is drafted). One slot, one place.
- **Where digit keys go (R6.3):** digits answer the keyed decision only while no text field has focus (`ComposerUi::digits_to_card`, set once per frame by the dock). Then the decision shows its keycaps; the card is the only place they appear (R8.2). While the card's reply has focus, digits type there. Esc closes the reply and hands the digits back. A folded card (below) takes no digits.
- **Access chip (R6.4):** `access_chip` sits beside the model chip ("Sonnet · Ask first"): the mode in force, or its default until the agent reports one. It opens `access_option` rows that apply at once (no Apply). Full access shows a warning glyph and its word in the warning hue, `roles.caution`. One word per mode everywhere, `Permission::word()`: Ask first, Can edit, Plan only, Full access. The model popover follows the same rule for all three of its choices: a model, an effort or an access picked there applies at once, with no Apply button; the header's access chip uses the same words and the same warning hue. Its lists share one selection idiom: a leading check in a fixed slot (models and access alike), and Full access reads in the warning hue there too.
- **New session is the welcome (R6.4):** the centred 24/600 question, then this composer (`launch_task_box_v2`) with machine, project, agent, model and access chips and Start (Primary) inside it. A chip opens its choices in one card below; context window and name fold under "More options".
- **Slash menu:** a `menu` of `list_row`s anchored above the composer (`anchored_menu`, `MenuAnchor::AboveStart`). In a card's reply it hangs above the reply's input, inside the card's own space.
- **API** (`src/ui/system/composer.rs`):
  - `composer_field(state, &p) -> Div`, then `composer_body(state, compact, &p)` holding `composer_text(min_lines, &p)` (the input) and `composer_toolbar(leading, trailing, &p)`.
  - `composer_specimen(id, state, text, placeholder, deciding, can_send, &p)` draws a static composer (the gallery: the live one needs an `InputState`).
- **Do:**
  - Hide the composer while a decision holds its slot, and reply from the card (R8.4).
  - Use `composer_send_state`; never pick Send's tier locally.
- **Don't:**
  - Fold the composer to a line under a card, or show a second input beside the card's reply.
  - Build a second composer, or style a task box locally.
  - Let the slash menu (or any menu) cover a pending decision.

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
  - `menu_shortcut("g h", &p)`: an item's trailing keys as keycaps (`.child` it onto the item).
  - Anchor it with `system::anchored_menu(trigger, Option<menu>, MenuAnchor::{BelowStart, BelowEnd, AboveStart, AboveEnd, BesideEnd}, &p) -> Div` (`system/menu.rs`). The header's "…" uses it. The model popover hangs from whichever control opened it (the composer's chip, left edges aligned, or the header's "…"), decision pending or not.
  - **A card's menus open inside or below the card** and never cover a row they don't own (R6.1). `split_button_with_menu` (Deny's "Deny with note…") opens `BesideEnd`: to the right of the chevron, top edges aligned, so "Always allow" below stays visible. `split_button_with_menu_at(…, anchor, &p)` picks another anchor: Queue's send menu hangs `AboveEnd` from Queue itself, inside the composer's reach, clear of the last message and its timestamp.
  - The caller owns the open flag. Close the menu on a choice, on Esc and from `.on_mouse_down_out`.
- **Don't:**
  - Hang a menu from a window corner.
  - Let a menu cover its own trigger, or text it doesn't own.

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
  - `system::waiting_tile(&p)`: a tile that waits on the user (the `border_strong` edge and the attention bar).
  - `system::waiting_tile_focused(&p)` (r6): the same tile while it has the keyboard focus. R8.4: focus changes only weight, so the attention bar **stays**; the focus ring is added around the tile (`tile_frame`) and nothing moves.
  - `system::tile_columns(columns, cards, &p)` (R8.5): masonry. Card `i` goes to column `i % columns` and each column stacks its cards at their own height, so reading order is still row by row and a tall card leaves no hole beside it. The Fleet uses it for its band and for the rest.
  - `system::side_sheet(&p)` (R8.5): a panel on the right edge of a `.relative()` page frame, full height, `system().overlay_width` wide, on the Overlay elevation, over the page (which does not reflow). Give it a header with a close button; Esc closes it. The Fleet's Details open in one.
  - `system::tile_frame(id, focused, &p)`: the frame a tile sits in, carrying the 2px focus ring outside the tile.
  - `primitives::inset(&p)` for the one recessed well inside a card.
- **Do:** use `tile_grid` (or, where cards differ a lot in height, `tile_columns`) for every grid of cards (R7.6: a card is as tall as its content; a row without a decision does not stretch to match one with it). `even_tile_grid` is gone.
- **Don't:**
  - Nest cards.
  - Stretch a short card to its row (that is `page_grid`; use it only for equal content).

### Table

- **Purpose:** dense rows: Jobs, Machines, History, the Fleet table. R7.6: **one** table style, `system::table` (see [Table](#table-systemtable-srcuisystemtablers) under R7.6); `page_table*` are its aliases.
- **Anatomy:** a `page_table` (surface, hairline, clipped) holding a `page_table_head` (on `base`, 12/600 secondary) and `page_table_row`s. The cells come from `Col::{Fixed, Keep, End, Flex, Auto}` widths in `pages()`. `Col::End` is a fixed width right-aligned, for numbers and times: its header sits over its values' right edge (History's "Last active"). The status column is a **fixed slot** holding a StatusCell. The actions column is a **fixed slot**: `action_slots(vec![action_slot(width, Option<control>)…])`, with `slot_button` for buttons whose label swaps.
- **States:** row Rest and Hover (add `.id` plus a hover fill). Empty slots keep their width.
- **API:**
  - `primitives::page_table`, `page_table_head(&[(label, Col)], &p)`, `page_table_row`, `page_cell(Col)`, `action_slot`, `action_slots`, `action_slots_width(n, &p)`, `slot_button`
  - `table(headers, rows, &p)` for simple tables
  - Row actions revealed on hover still get their own `Col::Keep(action_slots_width(n, &p))` column (History), so they never cover a value.
  - `table_style(&p)` (Markdown tables share it)
- **Don't:**
  - Leave a status cell blank. Use Idle "Unknown", or drop the column.
  - Let hover actions cover a value.

### DecisionCard

- **Purpose:** a decision waiting on the user: a tool approval, a question, a plan, a job proposal. One component in the conversation's dock, on a Fleet tile and in Jobs. The gallery shows one specimen per kind.
- **Anatomy:**
  - The question headline (15/600, code spans wrap).
  - The reason.
  - The full request (mono, when it is long).
  - Where it applies (folder · host, tertiary).
  - The risk chip, when one is plain from the request: "Deletes files", "Deletes branches", "Writes outside the project", "Uses the network" (r6: a visible chip, its glyph and words on a quiet wash; the deleting ones in the neutral-strong `caution`, the others secondary; never amber or red). `decision::risk(approval, cwd)` derives it. A job proposal's command is read the same way (`jobs::job_risk`), so a scheduled `git branch -d` carries its chip in Jobs, on its Fleet tile and in the table.
  - The body, when there is one: a plan, a question's option list or its answer field.
  - The answer row: **Allow (Filled) and Deny (a neutral Bordered, R8.2) side by side on one line, never wrapped apart**, then any variant of the positive answer ("Approve · run with Codex", "Allow all 3 · cargo test"), Bordered, on Shift+1. R8.3: when the agent's limit is hit (`runnable_first`), the variant that can run now leads and is the Filled one; the positive answer, which would only wait, is Bordered.
  - The quiet row: each **"Always allow"** as a short Tertiary button with the rule and scope it saves beside it ("Saves `cargo test *` · this session"; the full explanation in its tooltip), Deny with note, then **Details** (Tertiary), always **at the right end** (R8.4). With no grant and no Deny with note, Details ends the answer row instead of standing alone on a line. In the conversation and Jobs, Details expands **below** its toggle; on the Fleet it opens the side sheet. It shows only what the card does not already say (the tool, other arguments, the exact provider rule).
- **States:**
  - **Keys** (R7.1): the one key map, [below](#decisions-one-key-map-and-three-tiers). R8.2: keycaps show only after keyboard use (`Workspace::keycaps(focused)`: the last input was a key; a pointer click, on release, puts them away) and only on the focused decision (`keys`: the dock's top decision, the focused Fleet card or table row, Jobs' first proposal, a stalled subagent's view); unfocused tiles show none. While the focused decision's digits type into the composer (`keys_live` off) its keycaps dim (`wash.yielded`), so nothing moves.
  - **Destructive** (r6): a request whose risk deletes files or branches (`Risk::confirms`), approval or job proposal, is allowed on the **second** press, by click or key. The first arms it (`Workspace::confirm_allow`, keyed by `decision::confirm_key(session, card)`); Allow then reads "Press again to delete" as the filled red **Danger** (`armed`, R8.2), whatever the region's focus. **Esc cancels** (`Workspace::escape_decision`). Deny never asks twice. A batch never holds one. The same on every surface (R8.3): the dock, Jobs, a Fleet tile, a Fleet table row, a proposal on the Fleet and in its table.
  - Details open.
  - **Note open** (r6): the note **replaces** the answer row, the grants and Details: the note field, Cancel and its send ("Deny and tell it", Destructive) are the only controls; never a second Allow / Deny under it.
- **Frames:** `Card { compact }` (the dock, Jobs), `Inline` (a Fleet tile), `Row` (r6: a table row's actions alone, right-aligned, same tiers and keys: the Fleet table's Allow is the filled Primary, as everywhere).
- **One component** (r6): every approve / deny is a `Decision`: the dock, Jobs proposals, Fleet tiles, the Fleet table, a stalled subagent (tile, row and its own view: `Workspace::stall_decision`). A Fleet card with a stall behind another ask shows the stall's row without keys and without a Primary (the ask owns both).
- **Tiers:** Allow is Primary unless a note owns it. Deny is a neutral Bordered (`actions::tier(Verb::Deny)`, the default `deny_variant`); Stop keeps the red label (`Destructive`). Armed, Allow is Danger.
- **Submit label:** every question card submits with `decision::SUBMIT_ANSWER` ("Send answer"; "Send answers" for a set). "Next" only steps a multi-question set.
- **Scope:** the hub offers session-scoped grants only, so the scope reads "this session". Project scope needs a hub option.
- **API** (`decision.rs`):
  - `Decision::new(selector, Choice::new(id, label).key(Some(1)), Frame::{Card { compact }, Inline})`. Fill in `headline`, `reason`, `request`, `folder`, `host`, `risk`, `body`, `alternates`, `grants`, `deny`, `deny_variant`, `deny_note`, `details` and `keys`; `show_allow = false` when the body answers (an option list). Then call `.render(&p)`.
  - `Choice::new(id, label).key(..).tooltip(..).enabled(..).on_click(..)`.
  - `Grant::new(choice, provider_words, command)`: the rule as read, the exact rule and the scope (`grant_rule`).
  - `Details::facts(toggle, open, selector, prefix, rows)` (rows are filtered against the card) or `Details::element(toggle, open, content)`.
- **Fixed slots (R8.4):** the same parts on every surface, Jobs included: the amber rail when it needs you, the R7.1 slots, Details at the right. Focus changes only weight (the Filled tier, the keycaps, the focus ring), never which parts are shown: a Fleet row shows Deny with note ("Note") and Details whether focused or not, and its whole command on every row.
- **Don't:**
  - Stack Allow and Deny, or let a long "Always allow" button outweigh Deny.
  - Make Deny blue or red.
  - Hide the keycaps of the focused decision while the keyboard is in use; dim them where the keys do not go. Keycaps keep their hairline box, also on a Primary fill, so they never read as counts.
  - Repeat the card in Details.
  - Draw an approve / deny pair outside `Decision`.
  - Number anything outside the key map, or move a key to fill a gap.

### SegmentedControl

- **Purpose (r6, R6.7):** one choice of a few, always visible: Grid / Table, By status / Waiting longest, Needs you / Active / All.
- **Anatomy:** a track (the `wash.hover` tonal wash) holding segments: a label and an optional count.
- **States:** selected (the `roles.segment_selected` fill: white on Light's grey track, the lighter `selected` fill in dark themes; a `border_strong` hairline, the raised shadow, 600 primary text, its count too), rest (secondary text on the track; count tertiary), hover (a wash, primary text). The selected one reads as selected at a glance in every theme.
- **API** (`system/segmented.rs`): `system::segmented_control(&p).child(system::Segment::new(id, label).count(n).selected(on).render(&p).on_click(…))`.
- **Don't:** use count badges inside a segment, or a filter chip row for an exclusive choice.

### PageShell

- **Purpose:** the page templates. **There are exactly two (R6.5):**
  - **Form** (`PageWidth::Form`, body `pages().form` 880, left-aligned under the full-width header, R7.6): Settings, Agent setup, Session details, Model, Handoff, Pull request. New session is the welcome's centred composer (`PageWidth::Composer`).
  - **Full-width table** (`PageWidth::Wide`, `pages().wide` 1240): Projects, Jobs, History, Machines, Scratch, Changes. Fleet's tile grid is the same template at the grid's cap (`PageWidth::Grid`, 1440).
- **Anatomy:** the page scroller (`Workspace::page_view`) gives every page the same top offset and gutter. `page_header` contains:
  - the back link, **only on drill-in pages** (Session details, Model, Changes…). The sidebar's destinations (Projects, Jobs, History, Machines, Scratch, Settings, Fleet, New session) have none: the sidebar is the navigation, and Esc still returns to the conversation. `page_back_v2` returns `None` for them.
  - the 24/600 title
  - a one-line description
  - the actions in **one** right slot

  The body follows, `space.xxl` below. The header and the body share one width rule.
- **Widths:** see [One page, one table, one composer](#one-page-one-table-one-composer-r76). The header always takes the page frame; only the body is capped.
- **Header actions:** one right slot. Refresh (Tertiary, `Concept::Refresh`) comes first; the page's main action (Add…, New job) is last and **Secondary on every page**, so a pending decision on the page can own the one Primary.
- **Pages on it (r5-shell):** every feature page (`feature_page_v2`), Projects, Settings, New session, the Changes page (stretched to the window: `.flex_1().min_h_0()` on the shell) and Fleet. Session history keeps its rows as the scroller's own children (`page_list`) for the keyboard cursor, under the same header width and spacing.
- **Repeated facts go to the group header (R6.5).** A folder row inside a Projects group shows only what is live there (needs you, working); the group's head carries the totals (scratch files to run, sessions). On Machines, an offline machine with nothing to offer is named once above the links table, not as a muted row under every project.
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
- **States:** `CountTone::Neutral` (a wash with secondary text), `NeedsYou` (`attention_fill` under `on_attention`, only for what blocks the user now), `Review` (r8: `roles.review` under `p.base`, a To-review count), `Failed` (red fill, a named failure).
- **Tokens (R7.8):** `system().badge_height` 16, `badge_min` 18, `badge_pad` 4, `text.overline` / `weight.semibold`, `radius.full`.
- **One style everywhere (R8.2).** Every count is this badge: sidebar group headers (right after the label, a quiet Neutral badge for Working, Idle and Ended), nav items, segments. Its amber fill is Needs you's alone.
- **API:** `primitives::count_badge(label, CountTone, &p)`, `count_text(n)`.

### Meter

- **Purpose:** a share of a limit (usage windows, context).
- **Two colours (R7.4):** below 80% `Intent::Neutral`; from 80% (`system().meter_warn_pct`), the limit included, `Intent::Caution` (`roles.caution`, the warning hue). `level_intent(format::level(pct))` and `level_color` pick them, for usage, context and machine pressure alike. Never amber, never teal, never red: a spent limit says so in words.
- **Tokens (R7.8):** `system().meter_height` 4, the track `wash.hover`, `radius.full`.
- **Usage in the sidebar footer (R7.5):** one line beside the gear, the account nearest its limit, naming its window: "Claude · 5h limit ▬ 42%" (`format::window_name`, `format::usage_short`), the number in the meter's colour. Every account's sentence ("42% of 5-hour limit used", and when it resets) is in the tooltip; a click opens the details.
- **API:** `primitives::meter(fraction, intent, &p)`.
- **Don't:** use `badge(…)`, the old solid flag, for counts.

### Keycap

- **Purpose:** a key in a tooltip, a sheet or a hint.
- **Anatomy:** a 12 mono label in a hairline box (`kbd`). A **hint** (`keycap_hint`) trails a control: tertiary with a faint edge, or the label colour with no box on a Primary fill.
- **When (R8.2):** a decision's keycaps show only on the focused decision and only after keyboard use (`Workspace::keycaps`). The card is the only place they appear: there is no keycap row in the composer or the Fleet footer.
- **API:** `primitives::kbd(label, &p)`, `keycap_hint(key, on_fill, &p)`, `keycap_in(key, label_colour, &p)` (on any fill: the Primary's, the armed Danger's). In tooltips, spaces in the shortcut separate the keycaps (`tooltip_view(label, Some("g n"), &p)`; `tooltip_view_keys` takes an owned shortcut).
- **Don't:** tint a keycap, or join keycaps without gaps.

## One page, one table, one composer (R7.6)

Round 7 leaves one rule for each of these. Each is one component; a page,
table or composer that does not use it is a bug.

### Page template (`system::page_shell`, `src/ui/system/page.rs`)

Every top-level page (Projects, History, Jobs, Machines, Scratch, Settings,
Agent setup) and every drill-in page has the same header in the same place:

| Part | Rule |
|---|---|
| Frame | The page scroller (`page_view`) gives every page the same top offset and side gutter. No page adds its own |
| Header | Title (24/600) and its one-line purpose at the frame's left edge; actions in **one** slot at the frame's right edge. The header always spans the frame (`PageWidth::frame`, `pages().wide` 1240), whatever the body's width, so titles and actions never move between pages |
| Back link | Only on drill-in pages. Sidebar destinations and Agent setup have none (`top_level_page`) |
| Body | `space.xxl` below the header, left-aligned, at one of two widths |

| Body width | `PageWidth` | Pages |
|---|---|---|
| Form, 880 (`pages().form`) | `Form` | Settings, Agent setup, Session details, Model, Handoff, Pull request |
| Full width (`pages().wide`) | `Wide` | Projects, Jobs, History, Machines, Scratch, Changes |
| Fleet's grid cap | `Grid` | Fleet (header on the grid's edge) |
| Composer width, centred | `Composer` | New session only: it is the welcome |

Nothing else. A form is never centred under a left-aligned header.

### Table (`system::table`, `src/ui/system/table.rs`)

One table style for History, Jobs, Machines (both its tables) and the Fleet
table:

- **Frame:** `table(&p)`: the card surface, a hairline `card_border` edge,
  `radius.panel`, clipped.
- **Head:** `table_head(columns, &p)`: the first child, on `base`, 12/600
  secondary, `layout.row_height` tall, no edge above.
- **Rows:** `table_row(&p)`: `pages().row` tall at least, `cell_x` padding,
  a hairline (`border`) above, the `space.lg` cell gap. Hover fill
  `roles.hover`; the keyboard cursor's row `selected`.
- **Group rows:** `table_band(&p)`: a row on `base`, 14/600, inside the same
  frame (History's projects). Never a heading floating between tables.
- **Columns:** `primitives::page_cell(Col)` (`Fixed`, `Keep`, `End`, `Flex`,
  `Auto`), shared by head and rows so they line up.
- **List-built tables:** where each row must be the scroller's own child
  (History, for the keyboard cursor), wrap each item in
  `table_part(TablePart::of(i, len), &p)`. The slices draw the same frame:
  `Head` closes the top, `Body` the sides, `Foot` the bottom.
- `primitives::page_table`, `page_table_head` and `page_table_row` are thin
  aliases of these, kept for existing call sites.

### Composer width (`system::composer_width`)

The composer is one width everywhere: `pages().form`, 880, the
Comfortable reading width.

- **Welcome:** the welcome column is capped at it (headline, composer,
  starters, agent checklist).
- **New session:** `PageWidth::Composer`, the same centred column.
- **Conversation:** the dock caps the composer at it, centred, even when the
  reading width is Wide or Full (the transcript widens; the composer does
  not).

### Conversation details that go with these (R7.6, R7.8)

- **Short conversations read from the top.** The transcript list is
  bottom-aligned (it follows the tail). While following, the room it would
  leave above the first row goes under the last row instead
  (`short_pad`, `settle_short_pad`), and shrinks to nothing once the rows
  fill the viewport.
- **Subagent view:** one "Back to parent", the labelled button on the
  read-only bar. The header has no back arrow and labels Refresh; the
  "parent is waiting" line only says so.
- **Wake strip:** opaque canvas with a hairline under it. No fade under it.
- **Jump-to-latest fade:** two reading lines tall, the lower line solid
  canvas, so a line the viewport cuts is hidden whole, never sliced.
- **Stop and Queue:** typing in a running session keeps Stop (Bordered
  destructive) beside Queue (Primary).
- **Inline code:** `system::inline_code_bg(&p)`, a `wash.tone_rest` wash of
  the text colour over its surface, in prose, decision headlines and saved
  rules. The contrast gate checks text on it and the chip against every
  surface.
- **"Changes +N −M"** loads when a session is selected
  (`sync_change_summary`), not only once the panel opens.

---

---

## Status and vocabulary (R7)

### The glyph table (`primitives::STATUS_TABLE`)

Seven glyphs (`MAX_STATUS_GLYPHS`), 13 states. A glyph is shared only by states of one colour, and the word tells them apart. Every state draws its glyph wherever it is listed, Idle included.

| Glyph | Shape | Colour | States (word) |
|---|---|---|---|
| Arc | Faint ring with a turning arc | `status.working` | Working ("Running", "Sending…") |
| Dot | Filled dot | `attention_fill` (amber) | Needs you |
| Half | Ring, left half filled | `roles.review` (violet) | To review |
| Cross | ✕ | `fault` (red) | Failed ("Exit code 1"), Not responding |
| Ring | Hollow ring | `idle` | Idle, Waiting to start, Unknown |
| Check | Check | `done` (green) | Done ("Passed"), Ready ("Online", "Set up") |
| Dash | Level dash | `idle` | Ended, Paused, Offline |

### One status per element (R7.3)

- `status::derive` gives each session one status, in this order: Offline, Paused/Ended (stopped), Failed, Needs you, Not responding, Needs you (stalled subagent), Working, **Ready for review**, Idle.
- **To review** (`StatusKind::Review`, `Bucket::Review`): the turn is over, the session is live, and the hub reports uncommitted changes (`Session::uncommitted`, row field `uncommittedChanges: {files, added, removed}`; design v2 only). It has its own group under Needs you in the sidebar ("To review", its row's line "3 files changed · +48 −12", verb Open) and its own Fleet filter. `Bucket::wants_you()` is true for Needs you and To review: "All clear" is false while either has a member, and Alt+N stops on both. See [The attention model (R8)](#the-attention-model-r8).
- A parent rolls up its worst child: a run whose delegated subagent is still working shows the child's glyph, never a check beside a spinner.
- A failed transcript row is red (`EventRow::failed`), even in a run that later recovered; the turn's summary says "1 failed, then passed".
- The toolbar's failed count is the open failures only (`Workspace::open_failures`): a failure since passed or dismissed does not stay red.

### Verification line (R7.3)

The latest test run of a session, for decision cards and Fleet tiles (rendered by the decisions area).

- **Data:** `Workspace::session_verification(&Session) -> Option<status::Verification>`. Pure form: `status::verification(&session.tasks, claim: Option<&str>)`. It reads the session's background tasks, keeps the test commands (`status::is_test_command`: cargo test, npm test, pytest, go test, vitest, jest…), and takes the newest by end time. `None` when the session ran no tests (or the last one was killed).
- **`Verification`:** `outcome` (`TestOutcome::{Running, Passed, Failed}`), `command` ("cargo test", cut before its first flag), `at` (epoch ms), `contradicted` (the latest reply claims a pass after this failure: `status::claims_pass`; checked for the open conversation only).
- **Newest run wins (R8.1).** The headline is always the newest run. `Verification::earlier` is the outcome of the run before it when that one ended the other way.
- **Words:** `v.text(now)` = "Tests failed 4m ago · cargo test", plus `v.disagreement()`: "the reply says they pass" when the transcript claims a pass after the failure, else "earlier run passed" / "earlier run failed" when an older run disagrees. Never both, so the line keeps three parts. `v.headline(now)` is the part before the command; `v.kind()` is Failed / Done / Working.
- **Render:** `system::verification_line(&v, now, &p) -> Div` (selector `verification-line`): a StatusCell, red ✕ and red word when it failed, the command and any contradiction as its tertiary detail. The sidebar row's tooltip carries `v.text(now)`.

### Words (R7.4)

One word per concept, everywhere:

| Concept | Word | Not |
|---|---|---|
| A conversation with an agent | Session | Chat, thread, worker |
| A helper a session starts | Subagent | Helper, child, delegate |
| A shell or agent running beside the turn | Background task (toolbar label "Background tasks", `format::BACKGROUND_TASKS`, the panel's title) | Tasks, job |
| Scheduled work | Job | Task, cron |
| All sessions at once | Fleet | Dashboard |
| Finished with changes to look at | To review | Ready for review, Done, idle |
| A subagent's count | "1 subagent", "3 subagents this turn" | "1 delegated" |
| A subagent's built-in role | Its plain verb (`format::subagent_role`): "Reading code", "Planning" | "Explore", "Plan", "general-purpose" |
| News from other sessions | "2 updates" (the wake strip) | "2 updates from other sessions" |
| An agent | "Claude Code", "Codex CLI" (`format::agent_name`) | "Claude", "Codex", "OpenAI Codex" (an account's usage keeps the account name: "Codex · 93% …") |
| What a want-you row offers | Open (`InboxAction::label`, every row) | Review, Answer, Allow |
| Asking a quiet agent to carry on | Nudge (ask to continue) (`format::NUDGE`) | Nudge, ping |
| A machine's capacity | "1 of 4 agent slots in use", "0 of 2 command slots in use" (`hosts::Resources::{agent_words, shell_words}`) | "1/4 agents", "0/2 shell" |
| A queue item that cannot start | "every agent slot is busy" (`hosts::plain_wait`) | "no free agent slot" |
| Context in the composer | "Context 42% used" (`format::context_label`) | "Context 42% full", "ctx" |
| A usage meter | "Codex · 93% of 5h limit used", then the meter and "resets in 2h" (`format::usage_words`, `format::resets_in`) | "Claude · 5h limit 42%", "Claude 42%" |

- **Fleet counts:** the summary line ("2 need you · 1 to review · 3 working · 5 idle · 20 ended", `fleet::summary`) counts top-level sessions per group, job proposals in Needs you. The Needs-you band's header shows the same Needs-you number (never a tally of its cards), and "All clear" shows only when both want-you numbers are 0. Filters: Needs you, To review, Active, All. Each filter's count is one of those numbers or a sum of them, and its tooltip says which (`fleet::filter_tip`).
- **Page subtitles and nav tooltips say the same thing (R8.6):** Fleet `fleet::FLEET_SUBTITLE`, Machines `hosts::HOSTS_DESCRIPTION`, Scratch `scratch::SCRATCH_DESCRIPTION_V2`; the nav item's tooltip is "Fleet · " plus the subtitle.
- **Welcome (R8.6):** three starter tasks (`states::STARTERS`) under the composer; the placeholder (`states::PROMPT_PLACEHOLDER_V2`, "Describe a task, or leave this empty to start a blank session") says what the box takes and never repeats the hero's question. Usage is hidden until a session exists.

### Meta text (R7.8)

- `format::meta(parts)` is the one join helper: it trims each part, drops empty parts and stray "·", and joins with `format::SEP` (" · "). Never start a line with "·" or build one with `join(" · ")` by hand in new code.
- `system::meta_row(parts, &p)` is the same join for a line whose parts are elements (a StatusBadge, a link, then words): the parts sit between tertiary "·" dots, never bare gaps (r7-qa: Projects' group and folder rows).
- `system::key_hint(keys, label, &p)` and `system::key_hints(hints, &p)` draw every keyboard hint footer: faint keycaps, then plain words ("↑ ↓ Move · ↵ Open"). The Fleet and Projects footers use them; no footer spells shortcuts out in prose.
- `format::EMPTY` ("—", an em dash) is the one empty-cell mark.

### Meta lines: three items, one emphasised (R8.2)

Every meta line (a turn's work line, an event row, a subagent row in the transcript) shows **at most three items, one of them emphasised** (medium weight, the line's first item). The rest go to the hover or to Details. Status badges and actions on the right are not meta items.

- **Turn meta line** (`transcript_v2::work_meta`, a `MetaLine`): **"Worked 5m 12s"** · "6 tools" · one extra, the most urgent of a failure ("1 failed", red), a recovery ("1 failed, then passed") and the subagents ("1 subagent"; never "delegated", R8.6). The skill it used and any other extra are on the row's hover.
- **Event row** (`event_row`): the verb (emphasised) counts one, the target one per item ("Explore · Haiku 4.5" is two, `meta_items`), the scope and diff one. What room is left takes the facts in order of urgency: a fault (always shown), then the result, then the duration (`keep_facts`). Facts that do not fit are the hover of the facts that do (`event-facts-hover`), or of the verb.
- **Subagent row:** the task (emphasised), then what it is doing now or, when idle, its role, then its time. The role, model, tools, tokens and cost are on the row's hover.
- **Don't:** add a fourth item to a line, or emphasise two.

### A calm sidebar (R7.5)

- Rows that want the user (Needs you, To review, a fault) are semibold in the primary text colour. The selected row is primary, regular. Every other row is regular in `text_secondary`.
- Only those rows carry a second line (the ask, or the review's change counts).
- Group labels are 12/500 tertiary, each followed by its count badge (R8.2): amber for Needs you (it is also the "next that needs you" button, Alt+N), violet for To review, Neutral for the rest.
- **Rows are one height (R8.2):** `shell().row_single` for every session, subagent and summary row; a want-you row adds exactly one line for its ask (`row_double`). Subagent rows are indented and lighter (secondary text unless open); a stalled one says "Not responding" and keeps how long in its tooltip, so the title keeps its room.
- Usage is one footer line (see Meter).
- The collapsed rail lists Needs you, To review and Working sessions; every tile and icon has a hover label (a session's title and project in its hover card, each destination's name in its tooltip).
- **Rail tiles (R8.6):** the project's colour (`primitives::avatar_with_status_keyed(title, project, …)`), the session's two letters and the status glyph at the corner. The hover card names the session, its ask, its project and "Last active 23m ago".

## The attention model (R8)

Exactly two states want the user. Every surface (sidebar groups, the Fleet summary and filters, the rail, Projects, row tooltips) shows them the same way, from one source, and never folds one into the other.

| State | Means | Hue | Glyph | Count words | Row verb |
|---|---|---|---|---|---|
| Needs you (`Bucket::NeedsYou`) | Blocked on you: an approval, a question, a plan, a stalled subagent, a job proposal | Amber (`attention_fill` for marks, `attention` for words) | Dot | "1 needs you", "3 need you" | Open |
| To review (`Bucket::Review`) | Finished with uncommitted changes | Violet (`roles.review`) | Half ring | "1 to review" | Open |

- **One count source.** `Workspace::attention_counts` (top-level trees per group, job proposals in Needs you) → `attention::Wants { needs_you, to_review }`. Words: `attention::needs_you_words`, `review_words`, `Wants::words()` ("3 need you · 1 to review").
- **One row verb.** Every want-you row's chip says "Open" (`InboxAction::label`). The card it opens says what to do; "Review" is the state's name, never a button on a row.
- **Amber is reserved** for Needs you: warnings, the usage meter, Full access and links such as "+1 more" use `roles.caution` or neutral.

### The hue table (per theme)

| Theme | Needs-you text (`attention`) | Needs-you mark (`attention_fill`) | Badge label (`on_attention`) | To review (`review`) |
|---|---|---|---|---|
| Dark | `#e9b464` | `#e9b464` | base | `#be9df7` |
| Light | `#8a5800` | `#b97200` | text, deepened to ≥ 4.5:1 | `#6e3ead` |
| Nord | `#eec46e` (was the pale `#ebcb8b`) | `#eec46e` | base | `#c2adf3` |
| Tokyo Night | `#e0af68` | `#e0af68` | base | `#be9df7` |
| Catppuccin | `#f8c970` (was the pale `#f9e2af`) | `#f8c970` | base | `#d0aafc` |
| Gruvbox | `#d8a657` | `#d8a657` | base | `#c4a2ed` |
| Everforest | `#dbbc7f` | `#dbbc7f` | base | `#c4acf2` |
| Catppuccin Latte | `#7c5800` | `#ac6b00` | text, deepened to ≥ 4.5:1 | `#6b3baa` |

Tested in `system/contrast.rs`: the review word ≥ 4.5:1 and its glyph ≥ 3:1 on every surface and row; both count badges ≥ 4.5:1; the amber mark ≥ 3:1; and `needs_you_fault_and_warning_stay_apart_in_every_theme` keeps To review ≥ 0.10 OKLab and 30° from amber (text and mark), red, green and the warning, and ≥ 0.06 and 30° from the accent blue. The Nord primary keeps its ≥ 4.5:1 dark label (`v2_meets_the_direction_floors`).

### Time formats (R8.1)

One format per meaning, everywhere:

| Meaning | Format | Helper |
|---|---|---|
| Recency, in a sentence | "Last active 23m ago" | `format::last_active(at, now)` |
| Recency, in a cell or a verification line | "23m ago", "just now" (under 10s) | `format::ago(at, now)` |
| Recency, in a tight row (sidebar, rail) | "23m" (its tooltip says "Last active 23m ago") | `format::age(at, now)` |
| Duration | "Ran 3m 31s" | `format::ran(ms)`, `format::duration(ms)` |
| Until something happens | "resets in 2h 13m" | `format::resets_in(ms)`, `format::countdown(ms)` |

`ago` and `age` share thresholds, so a row's "23m" and History's "23m ago" are the same fact. History's Last active column reads a live session's last activity, as the sidebar does.

### Brand moment (R8.7)

`shell::brand_moment(p)` (selector `brand-moment`): the brace-and-cursor mark at title size, alone, centred. It sits above the welcome question and replaces the icon in empty states that have nothing of their own to show (the generic Inbox case). Error, ended and subagent empty states keep their meaning-bearing icons.

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
- **Rhythm (R6.6):** a fixed 8/16/32, from tokens only: `transcript().group_gap` 8 inside a group (notes between calls), `row_gap` 16 between blocks (half above and below each row), `turn_gap` 32 between turns (the row gap included) and above a heading (`heading_space`; 8 below it, `heading_gap`). A reply's hover actions (copy, time) float in the gap under it, so they never add to the rhythm. The turn's Changed block sits one `row_gap` under the reply.
- **One meta line per turn (R6.6):** a finished skill directly before a run, and the subagents pinned at the run's end, join the run's line: "Used skill code-review · Worked 5m 12s · 6 tools · 1 delegated", the delegates' rows beneath it while any works or the run is open (`transcript_v2::meta_line`, `folded_skill`, `folded_overview`).
- A finished turn folds into one summary row ("Worked 5m 40s · 6 tools"), coloured by its **final** outcome (`transcript_v2::recovered`: every failed call was run again, same tool and target, and passed). A recovered run says so in words, with a neutral retry glyph: "Worked 2m 07s · 3 tools · 1 failed, then passed" (`recovered_line`; never a green check, never red), and does not open by default; expanded, only the failed step keeps a muted mark (tertiary "Failed · exit code 1", no red badge).
- One "Changed N files" per turn. A wake from another session inside a turn does not split it (`starts_turn`).
- **Other sessions (R6.6):** wakes leave the transcript for **one strip above it** (`render_wake_strip_v2`, selector `wake-strip`): the wakes since your last message. One wake is its own line; several read "2 updates from other sessions" with the one Go button (the newest that waits on you), and open into each wake's line. Chosen over sidebar-only because the strip keeps the "Go to…" jump where the reader is, while the sidebar already shows each session's live state. Each wake's line is: an inbox mark (or the amber/red glyph when one waits or failed), "From other sessions" (only where no header says it already: a lone wake; the opened strip's lines lead with their news), the agents' states in tertiary words, and one Tertiary button ("Go to Release checklist"). An ask that has since been answered reads "answered" / "resolved", never as pending. Clicking the line opens what the wake carried. The transcript under the strip fades in over `layout.fade` (`wake-strip-fade`), as it fades out above the dock, so no line shows sliced at the strip's edge.
- **Subagents:** every subagent state comes from `status::derive` (`children::child_status_v2` for a provider subagent), so its own header and the sidebar agree on "Not responding · 7m". A transcript block that knows one turn's delegations says so ("2 delegated this turn"); the tree's total lives in the sidebar ("4 subagents · 3 working"). A delegation rolls up its worst child.
- The composer stays a visible bordered field, even while a decision is pending (one line until it has focus).
- **Change counts (R6.6):** the turn's line says "This turn"; Changes and the header say "Branch". The two scopes may differ (earlier turns are on the branch); the fixture keeps every file the turn touched on the branch with the same counts.

### Fleet layout (R8.5)

- **Header:** the title, a one-line subtitle (`fleet::FLEET_SUBTITLE`: "Every active session on one screen. Answer requests without opening each one."), and a slim toolbar: Grid / Table, the filter (each with its count), and an overflow menu (`fleet-more`) that holds the sort ("Sort by status", "Sort by waiting longest").
- **Counts:** one quiet line above the cards (`fleet-summary`, `fleet::summary`), or "All clear" when nothing needs you.
- **The Needs-you band** (`fleet-band`): a full-width section at the top ("Needs you N") holding every tile that waits on you, then the job proposals.
- **Everything else** (`fleet-rest`): compact tiles below the band.
- Both flow in columns (`system::tile_columns`, masonry): no holes. The arrows walk them by column (`fleet::step_band`): Down from the band's last card in a column lands in the same column of the rest; Left and Right follow the reading order.
- **Details** open in a side sheet (`fleet-sheet`, `system::side_sheet`) over the page's right edge, not in place, so the grid does not reflow. The tile's or row's Details toggles it; its close button and Esc close it.
- **Subagent actions** sit on the subagent's own line in the tile and name it: "Nudge Trace session", "Stop Trace session", View output (`Stall::target`).
- **The table:** Session at a fixed measure and what a row is doing folded into one flexible **Needs · doing** column (the ask when there is one, else what it is doing, quiet), so the ask has the widest room. An approval row shows its risk chip, the ask and the whole command on every row, and its answer row (`Frame::Row`): Allow, Deny, Note, Details. A **Review** row shows its review words and its newest test result (`fleet-row-verification-{n}`) with a Bordered **Open**. A **text question** is answered in its row: the field in Needs, Send answer in the actions. "+1 more" (a second ask behind the first) takes the warning hue, never amber.

### Fleet card anatomy

- A `tile` in `tile_columns` (R7.6: each card is as tall as its content, top-aligned; the decision first).
- Title (14/600), then elapsed time.
- StatusCell, then project, model and context.
- The ask, near the top, as an inline `Decision` (`Frame::Inline`): Allow and Deny side by side; a question answered on the card (an option list whose keys send, or a field where Enter sends); a job proposal is a card of its own (Allow, Deny).
- Up to three `EventRow`s: a working session's last tools, live. A named fault (a subagent not responding, a failed command) draws its status glyph **in the icon's place** (`EventRow::lead_status`), beside the words it qualifies, never at the tile's far edge.

The focused card shows its keycaps (after keyboard use, R8.2), with the focus ring (`tile_frame`) around it; its attention bar stays (R8.4). Waiting tiles use `waiting_tile`. Every filter count is the number of cards it shows, job proposals included. The dense view is a Table: one row per session (StatusCell, title, project, waiting, doing, needs, the decision's `Frame::Row`), toggled with Grid/Table and remembered per device (`Settings::fleet_table`).

r6 (R6.7) at scale: the sidebar is its rail while the Fleet is on screen (`track_fleet_rail`; restored on the way out). The header holds two SegmentedControls, Grid / Table and the filter, and (R8.5) an overflow menu with the sort: by status, or waiting longest (what waits on you first, longest wait first, then the rest by how long each has been quiet; `fleet::sort_tiles`). A tile waiting on you says "Waiting 3m". Identical requests (the same command in the same folder, none that deletes files: `fleet::batch_of`) offer "Allow all 3 · cargo test" on Shift+1 (R7.1: never a number), which allows each on its own card. A request or job that deletes is never in a batch. A session whose subagent stopped answering is Needs you (`status.rs`: `child_stalled`), its reason naming the child ("Trace session… not responding · 7m"), its ask Nudge / Stop / View output.

### Empty, loading and error states

See Page anatomy. The same rules apply inside cards and panels: one line, tertiary, and one way out.

---

## Decision and reply: one area (R8.4)

The conversation column is three parts: the header, the transcript, then one decision-and-reply area.

### Decision and reply (`system::card_reply`, `src/ui/system/composer.rs`)

- **One component, one input.** While an approval, a plan or a structured question waits, the decision card and the reply are one component and the composer is **hidden, not folded** (`Workspace::composer_hidden`).
- **Reply instead.** The card's quiet row carries "Reply instead", a Text action with no key (`Decision::reply`; a question card puts it at the end of its footer). It opens the card's one input, last in the card (`Decision::reply_field`): a divider, the composer's own `Input` (a draft survives; Enter sends as in the composer; Esc cancels), a tertiary line "The request stays open until you answer it" (with "1 file attached" when there is one), then Cancel (Tertiary) and **"Send to Claude"** (Secondary, arrow-up): the send names where the text goes. The card keeps the one Filled button.
- **Opening.** "Reply instead", Ctrl+L (focus the composer), or a draft already there opens the reply (`settle_reply`, once per frame). It stays open for that decision (`ComposerUi::reply_open`, keyed by session and card). A note (Deny with note, Keep planning) closes it: one input at a time.
- **API:** `card_reply(input, note, actions, &p) -> Div`.

### The folded card (`system::decision_line`)

- When the reader scrolls up through the transcript (not following) and is not writing in the card, the card folds to **one line**: the attention frame, "● Needs you · Run cargo test in native? · Open" (`decision_line(summary, action, &p)`, `Workspace::decision_summary`). Open goes back to the latest and unfolds the card.
- A folded card takes no digit keys.
- **Jump to latest never covers text.** It sits in its own band in the gutter above the dock (the transcript's viewport ends above it). While the folded card shows, the line replaces it: there is no separate pill.

### Updates chip (`system::updates_chip`, `update_row`, `updates_popover`)

- Other sessions' wakes never sit in the transcript. They are one chip in the conversation header, after the title: "2 updates" (R8.6: never "from other sessions"), led by the worst state's glyph (Needs you, else a failure) or a quiet inbox icon.
- The chip opens a popover anchored below it (`MenuAnchor::BelowStart`): **one row per event** (each agent a wake reports on), **failures first**, then what needs you, then the rest, newest first. Each row is its state glyph, what happened in words (the full line on hover), and its own **Open** jump link when that session can be opened from here.
- The chip counts the events its popover lists (one number, one list).
- Opening the popover never moves the reading position.

## Decisions: one key map and three tiers

R7.1 and R7.2. One rule each, enforced in code and by tests.

### The key map (R7.1)

Every decision surface uses the same slots: the conversation's dock, the sidebar, a Fleet tile, a Fleet table row, Jobs, a plan card and a stalled subagent. `decision::Slot` holds the map, and `Choice::slot(Slot)` puts a choice on its slot.

| Key | Slot | Approval | Plan | Job proposal | Stalled subagent |
|---|---|---|---|---|---|
| 1 | `Positive` | Allow | Approve plan | Allow | Nudge |
| 2 | `Deny` | Deny | Keep planning | Deny | Stop |
| 3 | `Always` | Always allow (the first grant) | — | — | — |
| 4 | `DenyNote` | Deny with note | — | — | — |
| ⇧1 | variant | Allow all N (Fleet batch) | Approve · run with Codex | — | — |
| no key | | a second Always allow | | | View output |

The rules:

- **A slot that is not offered stays empty.** Its key does nothing, and no other action moves up to take its number.
- **Variants never take a number.** A variant of the positive answer goes on Shift+1 (`Choice::variant_key()`, keycap `⇧1`, `decision::is_variant_key`; the Fleet binds `shift-1` to `FleetVariant`). Any other extra takes no key.
- **One verb pair.** Every approval says "Allow / Deny", job proposals included.
- **Deny with note is a visible Text button** on slot 4 (`Decision::deny_note`), never a menu item. On a Fleet tile or row it opens the session with the note field ready.
- **Keycaps show only on the focused decision, and only after keyboard use** (R8.2). Unfocused tiles and rows show none.
- **The card is the only place keycaps appear** (R8.2). The composer's keycap row and the Fleet footer's answer keys are gone.
- **Question cards are the one exception.** An option list is the answer itself, so its options are numbered as listed.
- **Enforcement.** `Decision::key_map()` reports every key a decision draws and what it answers. `decision::audit` records every decision drawn in a test. The test `every_decision_keeps_the_one_key_map` (`tests/fleet.rs`) walks the focus over every Fleet card in Grid and Table, then the dock's approval and plan, then Jobs, and holds each one to `slot_map_problems`.

### Button tiers (R7.2, R8.2)

There are three tiers, Filled, Bordered and Text, plus Danger. Each verb has exactly one tier, given by `system::actions::Verb::tier()`. A surface takes its variant from `actions::tier(verb)`, or from `actions::region_variant(verb, focused)` where a region can lose focus. It never picks a tier by hand.

| Verb | Tier | `ButtonVariant` |
|---|---|---|
| Send, Queue | Filled | `Primary` |
| Allow, Approve plan, Nudge (a stall's answer), Send answer | Filled | `Primary` |
| Commit, Ask agent to fix, Start (`Verb::Forward`) | Filled | `Primary` |
| Allow all, Approve · run with Codex | Bordered | `Secondary` (Filled when the limit is hit, R8.3) |
| Deny | Bordered | `Secondary` (neutral, R8.2) |
| Keep planning | Bordered | `Secondary` |
| Stop | Bordered | `Destructive` |
| Discard, Remove, End | Bordered | `Destructive` |
| A page header's main action (New job, Add…) | Bordered | `Secondary` |
| Always allow | Text | `Tertiary` |
| Deny with note | Text | `Tertiary` |
| Details, View output, Open, Cancel | Text | `Tertiary` |
| Nudge in a toolbar (not a decision's answer) | Text | `Tertiary` |
| An armed destructive confirm (`Verb::ConfirmDelete`) | Danger | `Danger` |

The rules:

- **One Filled per focused region.** On the Fleet, only the focused card's or row's positive answer is Filled (`Decision::filled`); the rest are Bordered. In Jobs, the first proposal's Allow is Filled. While a decision waits on the selected session, the docked side panel's main action (Commit, Ask agent to fix) is Bordered (`Workspace::side_panel_variant`). The test `one_filled_button_per_focused_region` counts the Filled buttons with `decision::audit`.
- **Risk is the same everywhere (R8.3).** A request or job that deletes (`Risk::confirms`) arms on the first press, on every surface: the dock, Jobs, a Fleet tile, a Fleet table row, a proposal's card and row. Allow then becomes the filled red Danger and reads "Press again to delete"; Esc cancels. Destructive requests are never in Allow all (`fleet::batch_of`). The test `every_destructive_decision_needs_two_presses_on_every_surface` presses each one.
- **Limits (R8.3).** When the agent's limit is hit, the action that can run now ("Approve · run with Codex") is the Filled one and leads (`Decision::runnable_first`); the one that would only wait is Bordered.
- **Tests.** `one_filled_button_per_focused_region` also holds Deny to a neutral tier and Danger to armed decisions; `every_decision_keeps_the_one_key_map` also holds each decision's parts fixed while the focus moves (R8.4); `keycaps_show_only_after_keyboard_use` (R8.2).

### Decision card content (R7.7)

- **Three lines of metadata.** The headline, the reason, then one line with the risk chip and where it runs (`decision-meta`). A long command shows whole in its mono block, which is the request itself and not metadata.
- **Details** (`Decision::detail_rows`) shows the whole command, the agent's stated reason, the access mode (`decision::access_line`: "Ask first · Asks before it runs a command or edits a file.") and, in words, what each Always allow would let through, with the provider's exact rule. After those come the rest of the request (the tool and other arguments). The folder and host are already on the card, so Details leaves them out.
- **Verification (R7.3).** Under the meta line, the session's latest test result (`Decision::verification`, `system::verification_line`). A Fleet tile carries the same line under its status (`fleet-verification-{n}`), with or without an ask.
- **Composer.** While a decision waits, the composer is hidden; the card's "Reply instead" opens its one input (R8.4).
- **Fleet table.** A job proposal's Needs cell names the command it would run ("Run git branch --merged … | xargs git branch -d") with its risk chip. An approval row carries its chip in the same cell.

## Contrast

`system/contrast.rs` checks every theme in `Appearance::ALL`:

- **Text tiers, prose and links** on chat, surface, base, raised, selected and hover: 4.5.
- **Status words:** 4.5. **Status glyphs:** 3.
- **Every Button tier** at Rest, Hover and Pressed, on every surface, with washes composited: 4.5. Danger included (its label on its red fill), and the Danger fill vs every surface: 3.
- **Primary fill:** 3. A **light** Primary fill (one with a dark label) needs 4.5, so it never reads as disabled. Nord's old frost Allow was 3.7.
- **Focus ring:** 3.
- **Count badges:** 4.5.
- **Keycaps**, including on Primary at rest, hover and pressed: 4.5.
- **NavItem** rest, hover and selected: 4.5.
- **Menu items and destructive items** on raised and hover: 4.5.
- **Avatars:** 4.5.
- **Toggle and Switch (R6.1):**
  - the knob is white, on and off
  - on knob vs track: 3
  - on track vs surface: 3
  - off hairline vs surface: 3, and vs the white knob: 3
  - off track vs surface: **at most** 2 (Light's dark off pills were 6:1)
- **The warning hue** as text on every surface and row: 4.5.
- **Separation (R6.2,** `needs_you_fault_and_warning_stay_apart_in_every_theme`**):** in every theme, Needs-you amber and fault red are at least 0.10 apart in OKLab and 30° apart in hue; the warning hue is at least 0.06 and 15° from each.
- **Scrim (R6.1):** 45–55% in every theme.
- **SegmentedControl:** the selected segment's fill (`roles.segment_selected`) stands ≥ 1.15:1 off its track (`wash.hover`) on every surface, plus its hairline and shadow.

## Design lint

`system/lint.rs` scans every `src/ui/**` file that reads v2 (`is_v2()`, `_v2`, `p.tokens`, `primitives::`, `system::`), except the component layer.

- **Rules:** raw colour literals, `px(<number>)`, `.shadow…(…)`, `.border_color(…)`.
- **Allowlist:** one table, `ALLOWED`, grouped by file, with a reason per entry:
  - `TO_MIGRATE`: v2-only files
  - `MIXED`: shared v1/v2 files whose v1 branch stays
- **Migrating a surface:** move it onto components and tokens, run `cargo test --features ui-tests system::lint`, then lower or delete its entries. The test fails until the counts match exactly, so the list only shrinks.
- **Printing the table:** run with `DESIGN_LINT_PRINT=1`.
