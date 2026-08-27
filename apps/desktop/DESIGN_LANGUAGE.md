# Workspacer Design Language

The reference for how the desktop renderer (and the web-remote build of it) should look
and be built. When adding or touching UI, conform to this file; when you make a deliberate
exception, document it here.

## 1. Icons

Two icon sets, one rule: **UI affordances are icon components, never raw unicode/emoji.**

| Set                                             | Where                                                           | Style                                                                                                                             |
| ----------------------------------------------- | --------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| **Workspacer pack** (`components/wksIcons.tsx`) | Purpose-built glyphs: panes, agent status, actions, diff, tools | 24×24 grid, 2.2 stroke, round caps, two-tone: `currentColor` body + accent node in `--wks-accent` (overridable via `accent` prop) |
| **lucide-react**                                | Everything the pack doesn't cover                               | Thin stroke (1.75–2.25), `currentColor`                                                                                           |

- `components/icons.tsx` is the hub: the `PaneIcon` map (pane type → icon), re-exports of
  commonly used lucide glyphs, and `export *` of the pack. Import from it or from
  `lucide-react` directly — both are established; never inline your own SVG for a concept
  either set already has.
- `StatusGlyph` (`components/statusGlyph.tsx`) maps ambient session state → pack glyph
  (`IconWorking`/`IconReviewing`/`IconQueued`/`IconIdle`) for use beside status labels.
- Agent/provider logos come from `components/agentLogos.tsx` — brand marks, not icons.

**Sizing** — match the surrounding text, don't freestyle:

| Context                                              | Size  | strokeWidth       |
| ---------------------------------------------------- | ----- | ----------------- |
| Inline with 0.6–0.7rem text (chevrons, status marks) | 10–12 | 2 (2.25 at ≤10px) |
| Buttons / close ✕ / toolbar glyphs                   | 12–14 | 2                 |
| Pane tabs, list leading icons                        | 14–16 | 1.75              |
| Empty states, tiles                                  | 18–24 | 1.75              |

**Mechanics**: icons inherit `currentColor` — color the parent, not the icon. A container
that holds only an icon gets `display:flex; alignItems:center`; icon-beside-text gets
`inline-flex` + `gap: 3–5`. Keep `title`/`aria-label` on the interactive element.

### Common vocabulary

Expand/collapse → `ChevronDown`/`ChevronRight` · close/dismiss → `X` · success/failure →
`Check`/`X` · warning → `AlertTriangle` · external link → `ExternalLink` · refresh/reset →
`RefreshCw`/`RotateCcw` · supervisor → `Compass` · overflow → `MoreHorizontal` · send →
`ArrowUp`. Don't invent a second mapping for a concept that already has one.

### Intentional typographic exceptions (do NOT "fix" these)

- **Status dots stay dots** — the colored `●`-style dots (rendered as styled spans/divs)
  and the sidebar's pulsing `■` busy square are status _tokens_, not icons.
- **Micro corner badges** — the `KIND_GLYPH`/`KIND_VISUAL` families (`! ? × ◷ ± ✓`) in
  `SideBar.tsx` and `attention/AttentionCard.tsx` render inside ≤17px tinted badges where
  bold text marks read better than icons. Migrate the whole family or not at all.
- **`✓`/`○` has-session markers** in dense sidebar mono rows — same status-token family.
- **Keyboard glyphs** — `⌘ ⌥ ⇧ ⌃ ↵ ⇥ ↑ ↓` in keycap hints and shortcut labels are text.
- **Prose arrows** — "Settings → Keybindings" style inline references are text.
- **`×N` multipliers** (e.g. file-frequency counts) are text.
- **`❯` prompt glyph** in SpawnAgentDialog's cwd field — deliberate shell aesthetic.
- **User-supplied emoji** — plugin `icon` fields and configured app-launcher icons are
  data; render them as given (code-side _fallbacks_ must be icon components).

## 2. Color

All color goes through the `--wks-*` CSS custom properties, set by `applyTheme()`
(`themes.ts`, 18 built-in themes + user custom themes; `resolveTheme()` is the only
registry lookup). `App.css` `:root` carries first-paint defaults mirroring `darkTheme` —
keep the two in sync when adding a token.

| Family   | Tokens                                                                                       |
| -------- | -------------------------------------------------------------------------------------------- |
| Surfaces | `--wks-bg-base / raised / surface / elevated / header / input / hover / selected / terminal` |
| Borders  | `--wks-border`, `-subtle`, `-input`, `-active`                                               |
| Text     | `--wks-text-primary / secondary / tertiary / muted / faint / disabled`                       |
| Accent   | `--wks-accent`, `-text`, `-glow`, `-bg`                                                      |
| Status   | `--wks-success`, `--wks-error`, `--wks-warning`, `--wks-busy`, `--wks-purple`                |
| Chrome   | `--wks-overlay`, `--wks-shadow`, `--wks-scrollbar-*`, `--wks-glass-*`, `--wks-claude-*`      |

Rules:

- **Bare `var(--wks-x)` — no per-site fallback literals.** `:root` guarantees resolution;
  inline fallbacks drift from the real theme values (this happened: `#4a9eff` ≠ the actual
  accent, and `--wks-danger` never existed so its fallback always won. The error token is
  `--wks-error`).
- **Tints via `color-mix`**: `color-mix(in srgb, var(--wks-error) 10%, transparent)` — not
  a hand-computed rgba of the hue.
- **Semantics**: success = done/healthy · warning = needs-you (approval/input/stale) ·
  error = failure/danger/destructive actions · busy = working (thinking/streaming/
  background) · purple = waiting-input accents · accent = selection/interaction.
- Pure `#000`/`#fff` in shadows and on-accent foregrounds are fine.
- **Allowed constant palettes** (not theme-dependent): file-type hues and diff-status hues
  (`claude/ChangedFilesCard.tsx`), provider brand tints, terminal ANSI palettes.
- In `components/claude/*`, prefer the `claudeColors` aliases from `claude-shared.tsx`
  (they resolve to the same tokens).

## 3. Typography

- Sans (chrome/body): `--wks-font-sans` (Hanken Grotesk). Mono (status bars, code-ish
  labels, dense rows): `--wks-font-mono` (JetBrains Mono). Terminal/transcript code uses
  `--claude-mono-font` (user's terminal font) — don't mix them up.
- Sizes are rem. **Pick from the scale; don't invent in-between values** (the 0.62/0.64/
  0.68/0.69 cluster is historical drift — collapse toward these steps when touching code):

| Step      | Use                                     |
| --------- | --------------------------------------- |
| `0.6rem`  | micro badges, keycaps, overlines        |
| `0.66rem` | dense meta rows, mono labels            |
| `0.72rem` | standard chrome text, secondary content |
| `0.8rem`  | body / primary content                  |
| `0.9rem`  | emphasized body, section headers        |
| `1.05rem` | pane/dialog titles                      |

- Weights: 400 body · 500 labels · 600 emphasis/buttons · 700 titles.

## 4. Shape & space

- Radii via tokens only: `--wks-radius-sm` (5) / `-md` (8) / `-lg` (12) / `-pill`.
  They follow the theme's `corners` style (`rounded`/`soft`/`square`) — a hardcoded
  `borderRadius: 4` breaks square-corner themes. `50%` for true circles is fine.
- Spacing rhythm: **4 / 6 / 8 / 10 / 12 / 16 / 20**. Padding pairs from the same set
  (e.g. `'6px 12px'`). Avoid 5/7/9 stragglers.
- Elevation: overlays use `--wks-overlay`, floating chrome uses `--wks-glass-*` +
  `--wks-shadow`; don't hand-roll rgba glass.
- **The chat measure is a token**: `--wks-chat-width` (900px) is the centered column the
  transcript, composer, tasks card, needs-you dock, GUI status row and the Agent Watch
  transcript all share (`maxWidth: 'var(--wks-chat-width)'`, `margin: '0 auto'`). It used
  to be `1040` copy-pasted across six sites; never reintroduce a literal — anything that
  must sit flush under the composer reads the token.

## 5. Components

### `Surface` — the container primitive

`components/Surface.tsx` is the one way to draw a card/panel/tile container.
Reach for it before hand-rolling a `<div>` with a border and a background.

**The rule it enforces: a surface separates itself with a border OR a fill, never
both — and surfaces do not nest more than two deep.**

That rule exists because density, not palette, was what made the app feel busy: a
blocked agent card stacked card → footer band → question picker → option row, each
drawing border _and_ fill, so a single card contributed eight edges and the fleet
view showed ~20 bordered rectangles at rest. Depth is the budget. If you need a
third level, you almost always want a fill step or a single left accent rule
instead of another box.

- `elevation="raised"` is **lit, not outlined**: an inset top lip + ambient halo +
  hairline drop. A plain fill step cannot carry it — several themes put
  `--wks-bg-surface` within 2–3 RGB units of `--wks-bg-base` (everforest, kanagawa)
  and one-dark puts surface _below_ base. See the module comment for the full
  reasoning before changing the treatment.
- Hover/edge declarations live in an injected stylesheet, not inline, so `:hover`
  wins on specificity without `!important` and a caller's own `onMouseEnter` still
  works.
- A **docked chrome bar** (the Fleet Deck footer, a pane header) may keep
  `borderTop` + fill. It is an edge, not a nested object — the rule is about
  surfaces inside surfaces.

Converted so far: `AgentCard`, `AgentCardBody`, `AttentionCard`, `ApprovalPrompt`,
`QuestionPicker`, `TasksCard`, `ToolTraceCard`, `WorkflowRunCard`,
`ChangedFilesCard`, and the Fleet Deck's own chrome.

### Other primitives

- Shared primitives live in `components/settings/primitives.tsx`: `SmallButton`,
  `ModeButton`, `Section`, `Row`, `CheckRow` (toggle), `SearchableSelect`, `inputStyle`.
  They are theme-correct — reach for them before hand-rolling a button/input, including
  outside Settings.
- Hover states: background shifts to `--wks-bg-hover` (or `--wks-bg-selected` for
  selection), 0.1–0.12s transitions.
- **A path affordance wears its destination's icon**: `FileLink` renders a leading
  `PaneIcon` for the pane a click will open (editor / `mdpreview`), 11px at 0.5 opacity,
  full opacity on hover. Both the icon and the click read `defaultOpenTarget()`, so the
  badge can't advertise a surface the click doesn't open. This replaced the old
  typographic `M↓` / `⊕` file-type marks — a destination is an action, and actions are
  icons here.
- Hover-revealed actions on a content block (copy a message, …) use the `.wks-hover-host`
  → `.wks-hover-actions` → `.wks-hover-action` family in `App.css`: the action row always
  occupies its 18px, only opacity moves, so revealing it can never reflow a streaming
  transcript. `CopyTextButton` (`components/claude/CopyTextButton.tsx`) is the first
  member. Touch devices get it at 0.5 opacity permanently (no hover to reveal it) —
  same `@media (hover: none)` escape hatch as `.wks-tab-close`.
- Interactive elements always get `cursor: pointer`, a `title` or `aria-label` when the
  label isn't text, and visible disabled styling (`--wks-text-disabled`).

## 6. Known debt / follow-ups

- Legacy `var(--wks-*, #hex)` fallback literals still exist in older code — strip to bare
  `var()` when touching a file (the `:root` defaults make them redundant).
- Font sizes: the 0.6–0.72rem drift cluster should collapse to the scale above.
- Inline `borderRadius: 3/4/6` numbers should migrate to radius tokens.
- ~~Shared `Card`/`Dialog` primitives don't exist yet~~ — `Surface` (§5) now covers the
  card/panel container and ten call sites are converted. Still outstanding:
  `InspectorCard.tsx` (6 borders / 14 backgrounds / 19 radii — the largest holdout),
  `CommandCard.tsx`, `AnsweredQuestionCard.tsx`, `WorkCard.tsx`, and a `Dialog`
  equivalent for the modal family.
- `SideBar.tsx` still has 5 border+fill blocks and 7 hardcoded `borderRadius` numbers.
  It was deliberately left alone during the Surface pass to keep that diff reviewable;
  it is the next file to convert.
- `deriveSupervisorName` bakes a 🧭 emoji into the supervisor _name string_ (crosses
  process boundaries); display sites now use the `Compass` icon — unifying the name
  format needs a coordinated change.
- Default app-launcher emoji icons come from `config_defaults.json` (Go-embedded,
  generated into `hooks/configDefaults.generated.ts`) — changing them is a product call
  plus regen, not a renderer edit.
