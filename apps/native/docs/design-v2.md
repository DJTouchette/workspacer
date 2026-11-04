# Design v2 (preview)

The native client is being redesigned behind one device-local setting. This
page is the contract for everyone restyling an area: how the switch works,
where the tokens live, which shared primitives exist, and the one rule.

## The switch

| | |
|---|---|
| Setting | `design_v2: bool` in `native-settings.json` (`wks_native::navigation::Settings`), default `false` |
| UI | **Settings → Appearance → New design (preview)** (`setting-design-v2` row, `design-v2-switch`) |
| Force for one run | `wks-native --design-v2` (not saved; switching in Settings replaces it) |
| Smoke captures | `python3 scripts/smoke.py --setting design_v2=true …` seeds the isolated settings file |

With the switch off the app is the shipped design: `Design::V1` reproduces
every colour, size and theme mapping exactly, and switching off after on
restores gpui-component's theme exactly (`ui::tests::design`).

The flag is read before the first `configure_theme` in `main.rs`, so the first
frame already draws the chosen design. Toggling calls
`Workspace::set_design_v2`, which re-runs `configure_theme` and
`apply_typography` the way a theme change does; terminals, popped-out viewers,
the review diff cache and the syntax theme all follow.

## Reading the design in code

```rust
// In Workspace code:
let p = self.palette();      // colours + roles + tokens for theme × design
let t = self.tokens();       // same as p.tokens
if self.design().is_v2() { … }

// In free functions: take the palette as today; the tokens ride along.
fn row(p: Palette) -> Div { div().gap(px(p.tokens.space.md)) }
```

Never call `self.appearance.palette()` (that is always v1). `Appearance::palette()`
stays the v1 palette for tests and v1-only code; `Appearance::palette_for(design)`
picks either.

## Tokens (`src/tokens.rs`)

`Tokens` is `Copy` and lives on every `Palette` as `p.tokens`. `Tokens::V1`
names the literals the shipped app uses (fields v1 never draws carry the v2
value and say so); `Tokens::V2` is the redesign's set. **Designers tune V2
there and nowhere else.** `tokens::tests::v2_holds_the_direction_scale` keeps
V2 on the direction's scale.

| Group | V2 values |
|---|---|
| `text: TypeScale` | The R2.2 scale **12/13/14/15/17/20/24**: `title` 24 (page titles, the welcome question), `title_short` 20, `heading` 17, `lead` 15 (decision headlines), `subheading` 14 (card/row/dialog titles), `body` 13, `meta` 12, `caption` 12, `overline` 12 (keycaps: nothing is smaller, nothing is uppercase), `ui` 14, `mono` 13, `line_height` 1.5. Times, counts and stats use tabular numerals (`primitives::tabular`) |
| `weight: Weights` | `regular` 400, `medium` 500, `semibold` 600, `title` 600 (v1 700). Use `FontWeight(t.weight.medium)` |
| `space: Spacing` | `xxs` 2, `xs` 4, `sm` 6, `md` 8, `lg` 12, `xl` 16, `xxl` 24, `xxxl` 32, `huge` 48 |
| `icon: IconSizes` | `xs` 12 (status), `sm` 14 (buttons, inline), `md` 16 (nav, card icons), `lg` 20, `xl` 24 (empty states) |
| `radius: Radii` | `xs` 4, `sm` 6, `control` **8**, `panel` 12, `composer` 16, `full` (chips, badges, dots) |
| `elevation: Elevation` | `raised` 1/2/0 ×0.35, `floating` 8/24/−6, `modal` 16/48/−12 ×1.25, `glow_*`, `scrim_alpha` 0.5 |
| `wash: Washes` | `disabled` .04, `rest` .06, `hover` .09, `pressed` .13, `tone_rest` .12, `tone_hover` .18, `tone_pressed` .24, `attention_edge` .35 (the waiting container's border), `yielded` .6 (the composer's opacity while a decision waits) |
| `stroke: Strokes` | `hairline` 1, `focus` 2, `stripe` 3 (the attention stripe) |
| `layout: Layout` | controls `control_height_sm` **28** / `control_height` **36**, `icon_button` 28, `row_height` 32; reading `chat_width` 760 (comfortable), `reading_wide` 1040, `reading_full` (no cap); pages `page_width` 880, `page_top` 40, `page_gutter` 32; sidebar `sidebar_row` 44 / `sidebar_row_compact` 28, `footer_height` 44, `sidebar_width` 304; status `status_dot` 6, `status_box` 12, `status_column` 112; `avatar` 24 / `avatar_lg` 32; `fade` 32; plus rail, menu, dialog and meter measures; Settings `settings_rail` 192, `settings_width` 1104, `theme_tile` 160 (area-pages) |
| `motion: Motion` | `pulse_ms` 1400 (the Working pulse) |

All sizes are logical px: pass them through the UI's zoomed `px()`.

The v1 constants (`chrome::scale::*`, `CHAT_WIDTH`, `portrait::RAIL_WIDTH`,
`sidebar::RAIL_*`, the default sidebar width) are defined from `Tokens::V1`, so
the two can never drift. They stay v1 forever: v2 code reads `p.tokens.*`.

### Colours

Colours are per theme on `Palette`; all eight themes have a v2 set
(`Appearance::v2_colors` → `V2Colors`, one arm per theme, in
`src/appearance.rs`). `palette_v2` derives the rest so the rules hold
everywhere:

| Role | Where it lives in v2 |
|---|---|
| Surfaces | `base` (sidebar/chrome), `chat` (canvas), `surface` (cards, code blocks), `roles.raised` (menus, popovers, dialogs, tooltips) |
| Fills | `selected` (always neutral), `roles.hover`, `roles.pressed` |
| Edges | `border` (dividers), `roles.border_strong` (inputs, secondary buttons, popovers), `roles.card_border` (the 1px card edge; visible in Light) |
| Text tiers | `text`, `prose` (chat copy), `roles.text_secondary` (= `muted`), `roles.text_tertiary` (= `disabled`, so v1 code under v2 inherits the hierarchy) |
| One accent | `accent` (links, focus = `roles.focus`), `primary` / `primary_hover` / `primary_pressed` (hover and press move away from the label colour), `on_primary` |
| Status (R2.1) | `roles.status: StatusRoles { attention, fault, working, done, idle }`, aliased by `warning` / `error` / `busy` / `success` / `text_tertiary`: attention amber (Light #B45309), fault red, working **cyan/teal, never the accent hue** (Gruvbox and Everforest take their aqua), done muted green, idle grey. Status colours are only for status; **accent blue means interactive**. No purple |
| Identity | `roles.provider` (= tertiary: provider glyphs and model names); avatars: `Palette::avatar_fill(name)` = OKLCH at the fixed `roles.avatar_tone` (lightness, chroma) with a hue hashed from the name (`format::avatar_hue`, purple band skipped); `roles.avatar[6]` keeps six sample tones |
| Code | `code_block` = `surface` (with a `border` edge), `code_header` = raised, `code_inline` |

Per-theme notes: Light has off-white canvas, white cards with a 1px
`card_border`, amber-700 attention text and a grey selected row. Nord's
primary is frost (Nord8) with nord0 text, so it never reads disabled.
Everforest's accent is its blue (green is success only).

`appearance::tests::v2_meets_the_direction_floors` enforces, in all 8 themes,
on chat / surface / base / raised: text and prose ≥ 4.5:1, secondary ≥ 4.5:1,
tertiary (meta text) ≥ 4.5:1, five distinct status roles with working ≥ 25°
from the accent and from done, Light attention = #B45309, Nord's label on
primary ≥ 4.5:1, accent and status colours ≥ 3:1; primary button vs surface
and chat ≥ 3:1; on-accent label ≥ 4.5:1 (rest, hover and pressed); visible
borders and selection; a real tier hierarchy; code on the card surface;
neutral providers; one accent hue; nothing purple; neutral selected/hover
fills; avatar text ≥ 4.5:1. `appearance::contrast(a, b)` is public for new
tests.

- Syntax colours: `ui/syntax.rs::syntax_for(appearance, design)`.
- gpui-component: v1 maps the shipped ~20 `ThemeColor` fields; v2
  (`ui.rs::configure_theme_v2`) also maps lists, tables, tabs, scrollbars,
  popover/tooltip, overlay, sidebar, accordion, sliders, progress, switch
  tracks, tone families and charts, so Switch, Select, Input, scrollbars and
  tooltips follow the v2 palette in every theme.

## The rule

**Every v2 style reads tokens, no literals.** No `px(12.)`, no `0x…` colours,
no `.opacity(0.08)`, no tailwind rem helpers (`px_3`, `gap_2`: they scale with
the reader's chat text size, not the interface size) in v2 code. If a measure
is missing, add it to `Tokens` (both sets, V1 naming the value v1 uses) and
document it here. Branch on `self.design()` only where v1 must stay byte-for-byte
the same; keep the v1 branch untouched.

Keep every existing `debug_selector`: UI tests find elements by them.

## Primitives (`src/ui/primitives.rs`)

Free functions that take `&Palette` and return a `Div` / `Stateful<Div>`.
Callers add `.on_click(cx.listener(…))`, children and `.debug_selector(…)`.
Interactive ones use `chrome::interactive_control` (keyboard focus ring in
the 2px border, Enter/Space activation, pointer cursor when enabled).

| Primitive | Signature |
|---|---|
| Button | `button(id, label, icon: Option<IconName>, ButtonVariant, ButtonSize, enabled, &p) -> Stateful<Div>` |
| Variants | `ButtonVariant::{Primary, Secondary, Ghost, DangerGhost}`: explicit, never inferred from the id. One primary per region; destructive = `DangerGhost` with confirm/undo, or the overflow menu |
| Sizes | `ButtonSize::{Small (28), Large (36)}`, `.height(&p)` |
| Icon button | `icon_button(id, IconName, tooltip, variant, enabled, &p)`; `icon_button_with_shortcut(id, icon, tooltip, Some("Ctrl K"), variant, enabled, &p)` |
| Split button | `let (main, menu) = split_button_parts(id, label, icon, variant, size, enabled, &p);` wire both, then `split_button(main, menu, &p) -> Div` |
| Overflow | `overflow_trigger(id, &p) -> Stateful<Div>` ("…", ghost, tooltip "More actions"); open a `menu(&p)` from it |
| Status | `status(StatusKind, &p) -> Div` (dot + vocabulary label); `StatusBadge::new(kind).label("Succeeded 3h ago").render(&p)`; `status_indicator(kind, &p)` (the fixed 12px box alone, for rails and dense rows) |
| Status kinds | `StatusKind::{NeedsYou, Working, Idle, Done, Failed, Stalled, Paused, Queued}` with `.label()`, `.color(&p)`, `.text_color(&p)`, `.needs_attention()`. Colour AND shape: NeedsYou amber filled dot; Failed red filled dot; **Stalled** red ring with a bar, label "Not responding" (`format::not_responding` → "Not responding · 2m"; never also Working); Working teal pulsing ring (reduced motion holds still); Queued teal still ring; Done muted-green check; Idle grey hollow dot; Paused grey pause bars |
| Page header | `page_header(back: Option<AnyElement>, title, description: Option<SharedString>, actions: Vec<AnyElement>, &p) -> Div` (22/600 title; back row height kept even without a back link, so titles share one y), `back_link(id, label, &p)`, `page_column(&p)` (shared `page_width`, `page_top`) |
| Card anatomy | `CardHead::new(title).icon(IconName)` or `.avatar(name)`, `.status(StatusBadge)`, `.meta(text)`, `.action(el)`…, `.render(&p) -> Div`; put it in `card(&p)` |
| Avatar | `avatar_tile(name, &p)`, `avatar_tile_sized(name, size, &p)`: the one avatar (projects, rail, cards): rounded-square monogram on `p.avatar_fill(name)` (hash hue, fixed OKLCH lightness/chroma), no emoji |
| Fade mask | `fade_mask(FadeEdge::{Top, Bottom}, color, &p) -> Div`: absolute, full width, `layout.fade` tall, colour → clear. Put it behind floating chrome |
| Tooltip | `tooltip_view(label, shortcut: Option<&'static str>, &p)` for `.tooltip(…)`; `with_tooltip(control, label, shortcut, &p)` for a control without one (GPUI allows one tooltip per element). Spaces in the shortcut separate keycaps ("g n") |
| Truncation | `truncated(id, text, &p) -> Stateful<Div>`: one line, ellipsis, full value in a tooltip |
| Pill (static label) | `pill(label, Intent, &p) -> Div` (prefer `status` for states) |
| Chip (toggle) | `chip(id, label, selected, enabled, &p) -> Stateful<Div>` |
| Badge (count) | `badge(label, Intent, &p) -> Div` |
| Meter | `meter(fraction 0–1, Intent, &p) -> Div` (give it a width) |
| Empty state | `empty_state(IconName, title, body: Option<SharedString>, &p) -> Div` |
| Section header | `section_header(label, count: Option<SharedString>, &p)`: THE section header, sentence case 12/600 secondary, count tertiary in tabular numerals. `section_label(label, &p)` is the same, padded (no uppercase anywhere) |
| Card | `card(&p) -> Div` (surface, `card_border`), `card_header(title, description, &p) -> Div` |
| List row | `list_row(id, selected, enabled, &p) -> Stateful<Div>` |
| Segmented | `segmented(&p) -> Div` holding `segment(id, label, selected, &p) -> Stateful<Div>` |
| Popover / menu | `popover(&p)`, `menu(&p) -> Div`, `menu_item(id, label, icon, danger, enabled, &p)`, `menu_separator(&p)` |
| Dialog | `scrim(&p)` holding `dialog(&p)`, with `dialog_header(title, description, &p)` and `dialog_footer(&p)` |
| Keycap | `kbd(label, &p) -> Div` |
| Helpers | `Intent::{Neutral, Accent, Info, Success, Warning, Danger, Busy}` with `.color(&p)`, `wash(color, alpha)`, `tonal(&p, alpha)`, `elevation(&p, Shadow) -> Vec<BoxShadow>` |
| Review | `gallery(&p) -> AnyElement` renders every primitive once (`ui::tests::design`) |
| Keyed button | `keyed_button(id, key, label, variant, size, enabled, &p)`: a button led by the key that triggers it ("1 Allow" on the approval strip) |
| Button roles (R2.2) | `ButtonVariant::{PRIMARY, SECONDARY, TERTIARY, DESTRUCTIVE}` (= `Primary`, `Secondary`, `Ghost`, `DangerGhost`), plus `Danger` (filled red) only inside a confirm step. Approval chips: 1 Allow primary, 2 Always allow secondary, 3 Deny destructive, 4 Deny with note tertiary |
| Inset | `inset(&p) -> Div`: the one recessed well inside a card (plan body, approval details). One card level plus this, never more |
| Table | `table(headers, rows, &p)`; `table_style(&p) -> TableStyle` is the one table style: `configure_theme_v2` maps Markdown tables from it, so both match (header on `base`, 12/600 secondary; hairline row dividers; no zebra, no outer border) |
| Event row | `EventRow::new(icon, verb).argument(…).summary(…).status(kind).duration(…).render(&p)`: icon · verb · mono argument · result summary · right-aligned duration (tabular) and status. Group them under `event_group_header(id, format::worked_summary(ms, tools), expanded, &p)` ("Worked 2m 14s · 18 tools") |
| Figures | `tabular(el)`: tabular numerals for times, counts and stats |
| Page grid | `page_grid(columns, &p)`: equal tracks at `space.lg`, rows stretched so card actions line up; inside `page_column` |
| Waiting on you | `waiting_card(compact, &p)` (neutral surface, 3px attention stripe, attention edge at 35%), `waiting_head(headline, where_why, pager, trailing, &p)`, `headline(vec![Headline::Text(…), Headline::Code(…)], &p)` (15/600, code spans truncate), `waiting_actions(&p)`. Used by approval, plan (+ limit), limit and questions; see below |
| Toolbar chip | `toolbar_chip(id, leading: Option<AnyElement>, label, opens, enabled, &p)`: a quiet chip in the composer toolbar (folder, model); `opens` adds a chevron and hover |

## Formatting (`wks_native::format`)

Pure and unit-tested; use these instead of ad-hoc `format!`s.

| Function | Result |
|---|---|
| `duration(ms)` | `"850ms"`, `"42s"`, `"12m 05s"`, `"5h 12m"`, and from 48h `"34d 3h"` |
| `is_stale(last_beat: Option<i64>, now)` / `stale_after(…, threshold)` | true past `HEARTBEAT_STALE_MS` (5 min): show `LOST_HEARTBEAT` ("Lost heartbeat") in `warning` instead of a growing timer |
| `timestamp(at: Option<i64>, now, twelve_hour)` | `"14:07"` today, `"Oct 14 · 14:07"` otherwise; never seconds |
| `truncate_end(text, max)` / `truncate_middle(text, max)` | `Some(cut…)` when it did not fit (then add a tooltip with the full value), `None` when it did |
| `monogram(name)`, `avatar_index(name, tones)`, `avatar_hue(name)` | what `avatar_tile` draws |
| `model_name(id)` | the one human model name ("Sonnet 4.6", "GPT-5.6 Luna"); the raw id goes in a tooltip |
| `not_responding(last_beat, now)` | "Not responding · 2m" for a Stalled agent |
| `worked_summary(ms, tools)` | "Worked 2m 14s · 18 tools" |

## Migrating a screen (recipe)

The Jobs page is the worked example: `ui/jobs.rs` (`jobs_page_v2`,
`job_chip`, `job_status`, `job_actions`, `run_row`) and its test
`ui::tests::job_list::jobs_page_under_design_v2_keeps_its_actions`.

1. **Branch once, high up.** `if self.design().is_v2() { return self.thing_v2(…) }`
   or a per-element `if`; leave the v1 branch byte-for-byte as it is.
2. **Read tokens, not literals.** `let p = self.palette(); let t = p.tokens;`
   sizes from `t.text.*` / `t.space.*` / `t.icon.*` / `t.layout.*`, weights
   from `FontWeight(t.weight.*)`, colours from `p.*` and `p.roles.*`. No
   `px(12.)`, no `0x…`, no `.opacity(0.08)`, no `px_3`/`gap_2`.
3. **Page frame.** Full pages use `page_header(back_link(…), title,
   description, actions)` and `t.layout.page_width` (`page_view(id,
   t.layout.page_width, …)` or `page_column(&p)`).
4. **Status.** Map the screen's states onto `StatusKind` with one small
   function (Jobs: `run_kind`) and draw `StatusBadge`s: never a bespoke pill
   or dot. "Ready"/"Ended" are `Idle`; green is only `Done`.
5. **Buttons.** Pick a variant per action: one `Primary` per region (or
   none), `Secondary` for the rest, `Ghost` in rows and toolbars,
   `DangerGhost` (after a confirm step) or the overflow menu for destructive
   actions. `ButtonSize::Small` in rows/cards, `Large` for page-level and
   composer actions.
6. **Cards and rows.** `card(&p)` + `CardHead` (icon, title, status, meta,
   actions right); long values via `truncated`; avatars via `avatar_tile`.
7. **Keep selectors and ids.** Reuse every existing element id and
   `debug_selector` (pass them to the primitive); add new ones for new
   elements.
8. **Test both designs.** Parameterise the screen's UI test over
   `Design::V1`/`V2` (`this.force_design(Some(design))`), and shoot it with
   `scripts/design-shots/shoot-all.py --setting design_v2=true --only <slug>`
   in dark, light and one more theme.

## Shell & navigation (area-shell)

| Piece | Where |
|---|---|
| Groups (Needs you · Working · Idle · Ended) | `ui/attention.rs` (`Bucket`, `group`, `next_needing`): pure, unit-tested. A top-level tree goes where its most urgent member belongs; Needs you sorts by `telemetry.last_activity_ms`, oldest first; Ended starts folded |
| Sidebar, rail, footer, cheat sheet | `ui/shell.rs` (`render_sidebar_v2`, `render_rail_v2`, `render_shortcuts`). `nav_sessions(cx)` is the list's visible order: j/k, Enter in search and `g n` walk it |
| Density | `Settings::sidebar_compact` (Settings → Appearance → Compact session list): `layout.sidebar_row` 44 or `sidebar_row_compact` 28 |
| Title capsule, tool rail | `ui/capsule.rs` (`render_title_bar_v2`, `chat_actions_v2`, `render_tool_rail_v2`) |
| Model popover | `ui/model_popover.rs`; the Model page stays on `g m` and "All model settings" |
| Keys | `g n` / Alt+N next session that needs you (v1 keeps `g n` for Scratch), `g x` Scratch, `?` / Ctrl+/ cheat sheet |

## Transcript (area-transcript)

The v2 conversation lives in `ui/transcript_v2.rs`; the v1 renderers
(`transcript.rs`, `work.rs`, `tools.rs`, `children.rs`, `fleet_card.rs`,
`plan.rs`, `session_tools.rs` thinking) branch into it once.

| | |
|---|---|
| Reading width | `Settings.reading_width: ReadingWidth::{Comfortable, Wide, Full}` (`"comfortable"`…), Settings → Appearance → Conversation → Reading width (v2 folds Chat into Appearance; v1 has no control). `Workspace::reading_column()` / `reading_frame()` give the transcript and dock one column (v1: `chat_column` / `chat_frame`); it gives way to the floating tool rail |
| Tokens | `p.tokens.transcript()` → `TranscriptTokens`: `user_max` 0.7, `tool_row` 28, `visible_steps` 4, `row_gap`, `turn_gap`, `details_max`, `brief_max`, `prose_line_height`, `paragraph_gap`, `heading_steps`, image preview size |
| Tool rows | `tool_line`: status indicator · verb · mono target · +/− · `tool_result_meta` ("3 lines", "1 match") · duration (≥ 1s) · chevron on hover |
| Cards | Subagent/workflow calls, subagent overviews and pending fleet wakes are `transcript_card` + `CardHead` + `StatusBadge`; resolved wakes collapse to one line (`fleet_eyebrow`: "Decision made", "2 agents finished") |
| Markdown | `left_align_text_columns` re-marks non-numeric right/centre table columns as left; code blocks on `code_block` (= surface) with a `card_border` hairline; inline code and bullets neutral |
| Syntax | `syntax_v2(appearance)`: one calm set per theme, nothing purple (`v2_syntax_reads_on_the_card_surface_and_is_never_purple`) |
| Shots | `shoot-all.py --only transcript-` (earlier, start, fleet, wide, full); the rig's `("wheel", notches, x, y)` step scrolls the chat |

## Panels (area-panels)

`Tokens::panels` (`src/tokens.rs`) holds the side panels' measures and
tints: `diff_row` (one stacked-diff slot), `diff_gutter`, `diff_marker`,
`diff_bar`, `diff_line_alpha` / `diff_emphasis_alpha` (the tone washes of a
changed line and of its changed words), `dock_min` / `dock_share` /
`dock_max` / `dock_handle` (the Changes dock), `header_height` (every panel
header), `files_width`, `slide_ms`, `terminal_line` and `cursor_alpha`.

- **Changes** (`ui/review/v2.rs`, `ui/review/v2/page.rs`, pure model in
  `ui/review/stack.rs`): under v2, `open_feature(Screen::Changes)` opens a
  resizable dock beside the conversation (`Workspace::open_changes_dock`);
  `review.v2.page_once` forces the page, as commit links and the dock's
  expand button do. Every changed file is read at once
  (`Request::DiffSet`, slot `diff-set`) and stacked in one `uniform_list`;
  file headers are drawn by a `UniformListDecoration` so they stick to the
  top. `Settings::review_dock_width` and `Settings::review_split` persist.
- **Tasks** (`ui/tasks/v2.rs`): the title chip is the shell's words chip
  (`capsule::render_tasks_words`, "3 subagents"); the panel gives a `StatusBadge` per task (`task_kind`),
  Copy for shell commands, Open log and Retry on failed tasks (Retry drafts
  a request in the composer), and Stop behind Keep / Stop task.
- **File viewer** (`ui/file_viewer/v2.rs`): breadcrumb header, icon
  actions, one primary (Save, only when dirty); the editor sits on
  `code_block` with a tertiary gutter (`syntax::highlight_theme`).
- **Terminal** (`ui/terminal/v2.rs`): panel header with a StatusBadge, and
  ANSI colours derived from the theme (`ansi_v2`).

## Waiting on you, composer and welcome (r2-systems)

| | |
|---|---|
| One container | Approval (`render_approval_strip`), plan (`render_plan_card_v2`), usage limit (`render_limit_card_v2`) and questions (`render_questions_v2`) are all `waiting_card`s in the dock above the composer, in that order. Each has a question headline, an always-visible where/why line, a pager ("2/3") when more than one waits, and actions in the four roles with number keys |
| Approval | `approval_headline` ("Run `cargo test` in apps/native?", "Edit `src/ui.rs`?") and `approval_context` (reason · folder on machine). Details (tertiary) opens an `inset` of facts |
| Plan + limit | ONE card (`decision_cards` folds Limit into Plan): Approve plan primary; "Approve · run with Codex" secondary only while a limit applies; Keep planning tertiary. A limit alone asks "Continue with Codex while Claude is paused?" A limit *warning* is a neutral note |
| Questions | A stepper: one question at a time (`ComposerUi::question_step`), headline = the question, "Choose one, or press its number", number keys pick (a single choice moves on), Back tertiary, Next / Send primary; no inner scroll |
| Composer | While a decision waits it yields: `wash.yielded` opacity until focused or typed in, placeholder from `composer_placeholder`, send drops to secondary. Idle: "Send". Working: "Queue" (secondary, Tab) and a split "Send now" whose menu holds Steer (Enter) and Queue with explanations. The slash menu floats in a popover above the composer (deferred), never between a decision and the input. The model chip shows the human name, raw id on hover |
| Welcome | One 24px question ("What should we work on in workspacer?"), no wordmark or "Your sessions" capsule. Agents read `Workspace::agent_setup` (`AgentSetup`: Ready / Needs sign-in / Not installed / Not checked), the same source and words as Setup, with the next step and a copyable install or sign-in command |
| Syntax | `syntax_v2` keeps every token ≥ 24 RGB units from the four status roles (`v2_syntax_never_borrows_a_status_colour`) |
