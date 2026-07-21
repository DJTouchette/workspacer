# Workspacer Design Language

The reference for how the desktop renderer (and the web-remote build of it) should look
and be built. When adding or touching UI, conform to this file; when you make a deliberate
exception, document it here.

## 1. Icons

Two icon sets, one rule: **UI affordances are icon components, never raw unicode/emoji.**

| Set | Where | Style |
|---|---|---|
| **Workspacer pack** (`components/wksIcons.tsx`) | Purpose-built glyphs: panes, agent status, actions, diff, tools | 24×24 grid, 2.2 stroke, round caps, two-tone: `currentColor` body + accent node in `--wks-accent` (overridable via `accent` prop) |
| **lucide-react** | Everything the pack doesn't cover | Thin stroke (1.75–2.25), `currentColor` |

- `components/icons.tsx` is the hub: the `PaneIcon` map (pane type → icon), re-exports of
  commonly used lucide glyphs, and `export *` of the pack. Import from it or from
  `lucide-react` directly — both are established; never inline your own SVG for a concept
  either set already has.
- `StatusGlyph` (`components/statusGlyph.tsx`) maps ambient session state → pack glyph
  (`IconWorking`/`IconReviewing`/`IconQueued`/`IconIdle`) for use beside status labels.
- Agent/provider logos come from `components/agentLogos.tsx` — brand marks, not icons.

**Sizing** — match the surrounding text, don't freestyle:

| Context | Size | strokeWidth |
|---|---|---|
| Inline with 0.6–0.7rem text (chevrons, status marks) | 10–12 | 2 (2.25 at ≤10px) |
| Buttons / close ✕ / toolbar glyphs | 12–14 | 2 |
| Pane tabs, list leading icons | 14–16 | 1.75 |
| Empty states, tiles | 18–24 | 1.75 |

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
  and the sidebar's pulsing `■` busy square are status *tokens*, not icons.
- **Micro corner badges** — the `KIND_GLYPH`/`KIND_VISUAL` families (`! ? × ◷ ± ✓`) in
  `SideBar.tsx` and `attention/AttentionCard.tsx` render inside ≤17px tinted badges where
  bold text marks read better than icons. Migrate the whole family or not at all.
- **`✓`/`○` has-session markers** in dense sidebar mono rows — same status-token family.
- **Keyboard glyphs** — `⌘ ⌥ ⇧ ⌃ ↵ ⇥ ↑ ↓` in keycap hints and shortcut labels are text.
- **Prose arrows** — "Settings → Keybindings" style inline references are text.
- **`×N` multipliers** (e.g. file-frequency counts) are text.
- **`❯` prompt glyph** in SpawnAgentDialog's cwd field — deliberate shell aesthetic.
- **FileLink badges** (`M↓` etc.) — compact typographic file-type marks.
- **User-supplied emoji** — plugin `icon` fields and configured app-launcher icons are
  data; render them as given (code-side *fallbacks* must be icon components).

## 2. Color

All color goes through the `--wks-*` CSS custom properties, set by `applyTheme()`
(`themes.ts`, 18 built-in themes + user custom themes; `resolveTheme()` is the only
registry lookup). `App.css` `:root` carries first-paint defaults mirroring `darkTheme` —
keep the two in sync when adding a token.

| Family | Tokens |
|---|---|
| Surfaces | `--wks-bg-base / raised / surface / elevated / header / input / hover / selected / terminal` |
| Borders | `--wks-border`, `-subtle`, `-input`, `-active` |
| Text | `--wks-text-primary / secondary / tertiary / muted / faint / disabled` |
| Accent | `--wks-accent`, `-text`, `-glow`, `-bg` |
| Status | `--wks-success`, `--wks-error`, `--wks-warning`, `--wks-busy`, `--wks-purple` |
| Chrome | `--wks-overlay`, `--wks-shadow`, `--wks-scrollbar-*`, `--wks-glass-*`, `--wks-claude-*` |

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

| Step | Use |
|---|---|
| `0.6rem` | micro badges, keycaps, overlines |
| `0.66rem` | dense meta rows, mono labels |
| `0.72rem` | standard chrome text, secondary content |
| `0.8rem` | body / primary content |
| `0.9rem` | emphasized body, section headers |
| `1.05rem` | pane/dialog titles |

- Weights: 400 body · 500 labels · 600 emphasis/buttons · 700 titles.

## 4. Shape & space

- Radii via tokens only: `--wks-radius-sm` (5) / `-md` (8) / `-lg` (12) / `-pill`.
  They follow the theme's `corners` style (`rounded`/`soft`/`square`) — a hardcoded
  `borderRadius: 4` breaks square-corner themes. `50%` for true circles is fine.
- Spacing rhythm: **4 / 6 / 8 / 10 / 12 / 16 / 20**. Padding pairs from the same set
  (e.g. `'6px 12px'`). Avoid 5/7/9 stragglers.
- Elevation: overlays use `--wks-overlay`, floating chrome uses `--wks-glass-*` +
  `--wks-shadow`; don't hand-roll rgba glass.

## 5. Components

- Shared primitives live in `components/settings/primitives.tsx`: `SmallButton`,
  `ModeButton`, `Section`, `Row`, `CheckRow` (toggle), `SearchableSelect`, `inputStyle`.
  They are theme-correct — reach for them before hand-rolling a button/input, including
  outside Settings.
- Hover states: background shifts to `--wks-bg-hover` (or `--wks-bg-selected` for
  selection), 0.1–0.12s transitions.
- Interactive elements always get `cursor: pointer`, a `title` or `aria-label` when the
  label isn't text, and visible disabled styling (`--wks-text-disabled`).

## 6. Known debt / follow-ups

- Legacy `var(--wks-*, #hex)` fallback literals still exist in older code — strip to bare
  `var()` when touching a file (the `:root` defaults make them redundant).
- Font sizes: the 0.6–0.72rem drift cluster should collapse to the scale above.
- Inline `borderRadius: 3/4/6` numbers should migrate to radius tokens.
- Shared `Card`/`Dialog` primitives don't exist yet; the ~15 `*Card.tsx` components each
  hand-roll their container. Candidate for extraction.
- `deriveSupervisorName` bakes a 🧭 emoji into the supervisor *name string* (crosses
  process boundaries); display sites now use the `Compass` icon — unifying the name
  format needs a coordinated change.
- Default app-launcher emoji icons come from `config_defaults.json` (Go-embedded,
  generated into `hooks/configDefaults.generated.ts`) — changing them is a product call
  plus regen, not a renderer edit.
