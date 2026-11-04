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
names the literals the shipped app uses; `Tokens::V2` is the redesign's set and
currently starts as a copy of V1. **Designers tune V2 there and nowhere else.**

| Group | Fields |
|---|---|
| `text: TypeScale` | `title` 22, `title_short` 18, `heading` 15, `body` 13, `meta` 12, `caption` 11, `overline` 10, `ui` 15 (gpui-component base), `mono` 13, `line_height` 1.45 |
| `space: Spacing` | `xxs` 2, `xs` 4, `sm` 6, `md` 8, `lg` 12, `xl` 16, `xxl` 24, `xxxl` 32 |
| `icon: IconSizes` | `xs` 11, `sm` 12, `md` 13, `lg` 14, `xl` 20 |
| `radius: Radii` | `xs` 4, `sm` 6, `control` 12, `panel` 16, `composer` 24, `full` |
| `elevation: Elevation` | `raised` / `floating` / `modal` `Shadow { offset_y, blur, spread, opacity }`, `glow_blur`, `glow_spread`, `glow_alpha`, `scrim_alpha` |
| `wash: Washes` | `disabled` .04, `rest` .07, `hover` .12, `pressed` .17, `tone_rest` .08, `tone_hover` .14, `tone_pressed` .2 |
| `stroke: Strokes` | `hairline` 1, `focus` 2 |
| `layout: Layout` | `chat_width` 900, `chat_gutter` 20, `rail_width` 56, `rail_row` 44, `rail_session` 30, `rail_child` 22, `flyout_gap` 6, `sidebar_width` 304, `control_height` 32, `icon_button` 28, `status_dot` 6, `meter_height` 4, `row_height` 30, `menu_width` 240, `dialog_width` 440 |

All sizes are logical px: pass them through the UI's zoomed `px()`.

The v1 constants (`chrome::scale::*`, `CHAT_WIDTH`, `portrait::RAIL_WIDTH`,
`sidebar::RAIL_*`, the default sidebar width) are now defined from
`Tokens::V1`, so the two can never drift. They stay v1 forever: v2 code reads
`p.tokens.*` instead.

### Colours

Colours stay per theme on `Palette` (all eight themes work in both designs):

- The v1 fields (`base`, `surface`, `chat`, `selected`, `border`, `text`,
  `prose`, `muted`, `disabled`, `accent`, `primary*`, `on_primary`, tones,
  code surfaces, `selection`, `shadow`, `shadow_opacity`).
- `p.roles: Roles`, the extra roles v2 needs: `hover`, `pressed`, `raised`,
  `text_secondary`, `text_tertiary` (not the disabled colour), `border_strong`,
  `focus`, `info`. In v1 each is derived from an existing colour.
- Per-theme v2 overrides go in `Appearance::v2_colors` (`src/appearance.rs`),
  one arm per theme. `palette_v2` also copies the token radii into
  `control_radius` / `panel_radius` / `composer_radius`.
- Syntax colours: `ui/syntax.rs::syntax_for(appearance, design)`.
- gpui-component: v1 maps the shipped ~20 `ThemeColor` fields; v2
  (`ui.rs::configure_theme_v2`) also maps lists, tables, tabs, scrollbars,
  popover, overlay, sidebar, accordion, sliders, progress, tone families and
  charts, so no widget falls back to the stock theme.

The appearance tests hold every v2 theme to v1's contrast floors.

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
Interactive ones use `chrome::interactive_control` (keyboard focus ring,
Enter/Space activation, pointer cursor when enabled).

| Primitive | Signature |
|---|---|
| Button | `button(id, label, icon: Option<IconName>, ButtonVariant, enabled, &p) -> Stateful<Div>` |
| Icon button | `icon_button(id, IconName, tooltip: &'static str, ButtonVariant, enabled, &p) -> Stateful<Div>` |
| Variants | `ButtonVariant::{Primary, Secondary, Ghost, Danger}`: explicit, never inferred from the id |
| Pill (status) | `pill(label, Intent, &p) -> Div` |
| Chip (toggle) | `chip(id, label, selected, enabled, &p) -> Stateful<Div>` |
| Badge (count) | `badge(label, Intent, &p) -> Div` |
| Meter | `meter(fraction 0–1, Intent, &p) -> Div` (give it a width) |
| Empty state | `empty_state(IconName, title, body: Option<SharedString>, &p) -> Div` |
| Section label | `section_label(label, &p) -> Div` |
| Card | `card(&p) -> Div`, `card_header(title, description: Option<SharedString>, &p) -> Div` |
| List row | `list_row(id, selected, enabled, &p) -> Stateful<Div>` |
| Segmented | `segmented(&p) -> Div` holding `segment(id, label, selected, &p) -> Stateful<Div>` |
| Popover / menu | `popover(&p)`, `menu(&p) -> Div`, `menu_item(id, label, icon: Option<IconName>, danger, enabled, &p) -> Stateful<Div>`, `menu_separator(&p)` |
| Dialog | `scrim(&p) -> Div` (absolute, fills parent) holding `dialog(&p)`, with `dialog_header(title, description, &p)` and `dialog_footer(&p)` |
| Keycap | `kbd(label, &p) -> Div` |
| Status dot | `status_dot(Intent, &p) -> Div` |
| Helpers | `Intent::{Neutral, Accent, Info, Success, Warning, Danger, Busy}` with `.color(&p)`, `wash(color, alpha)`, `tonal(&p, alpha)`, `elevation(&p, Shadow) -> Vec<BoxShadow>` |
| Review | `gallery(&p) -> AnyElement` renders every primitive once |

Example:

```rust
let p = self.palette();
primitives::dialog_footer(&p)
    .child(
        primitives::button("discard-cancel", "Cancel", None, ButtonVariant::Secondary, true, &p)
            .on_click(cx.listener(|this, _, _, cx| this.close_dialog(cx))),
    )
    .child(
        primitives::button("discard", "Discard", None, ButtonVariant::Danger, true, &p)
            .debug_selector(|| "discard".into())
            .on_click(cx.listener(|this, _, window, cx| this.discard(window, cx))),
    )
```

These primitives replace the v1 duplicates listed in the UI audit (pills in
hosts/jobs/fleet_card/scratch, three meters, four empty states, four section
labels, bespoke menu and modal layers, `Workspace::button`'s id matching).
Migrate an area by drawing it from primitives under `if self.design().is_v2()`
and leaving the v1 path as it is; the v1 helpers are deleted once v2 ships.
