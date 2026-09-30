# Plugin theming

Plugin panes render as webviews inside Workspacer, and the host injects its
active theme into every plugin page — on load and again whenever the user
switches theme. Your plugin should use these tokens instead of hardcoding
colors so it always matches the app.

## What gets injected

**1. CSS custom properties on `:root`** — the full `--wks-*` token set
(same names the app itself uses), plus `color-scheme: dark | light` so native
controls and scrollbars match.

**2. Zero-specificity defaults** — `:where(html, body)` gets the theme's
background/text color. Because `:where()` has zero specificity, *any* rule you
write yourself wins. A plugin with no styling at all still looks native.

**3. A JS hook** — for canvas/chart UIs that can't use CSS vars:

```js
// Current theme, available after load:
window.__WKS_THEME__            // { name: 'dark', vars: { '--wks-accent': '...', ... } }
document.documentElement.dataset.wksTheme  // 'dark' — for CSS hooks like [data-wks-theme="light"]

// Fired on load and on every live theme switch:
window.addEventListener('wks-theme', (e) => {
  redraw(e.detail.vars['--wks-accent']);
});
```

## Usage

Reference tokens with a fallback so the page still works standalone in a
normal browser (where nothing is injected):

```css
body  { background: var(--wks-bg-base, #1a1a1a); color: var(--wks-text-primary, #eee); }
.card { background: var(--wks-bg-raised, #222); border: 1px solid var(--wks-border, #333); }
a     { color: var(--wks-accent-text, #60a5fa); }
```

See `examples/clock-plugin/index.html` for a minimal working example.

## Token reference

| Token | Use |
| --- | --- |
| `--wks-bg-base` | page background |
| `--wks-bg-raised` | cards, list rows |
| `--wks-bg-surface` | secondary surfaces |
| `--wks-bg-elevated` | popovers, menus |
| `--wks-bg-header` | header bars |
| `--wks-bg-input` | inputs, wells |
| `--wks-bg-hover` | hover state |
| `--wks-bg-selected` | selected state |
| `--wks-border` / `--wks-border-subtle` / `--wks-border-input` | borders |
| `--wks-text-primary` / `-secondary` / `-tertiary` | text |
| `--wks-text-muted` / `-faint` / `-disabled` | de-emphasized text |
| `--wks-accent` / `--wks-accent-text` | accent fills / accent-colored text |
| `--wks-accent-glow` / `--wks-accent-bg` | translucent accent washes |
| `--wks-success` / `--wks-error` / `--wks-warning` | status colors |
| `--wks-overlay` / `--wks-shadow` | scrims, drop shadows |
| `--wks-scrollbar-thumb` / `--wks-scrollbar-hover` | custom scrollbars |

(There are also `--wks-claude-*` tokens used by the Claude pane; plugins can
use them but they're tuned for that pane's chrome.)

## Notes

- Injection happens for **plugin panes only** — pages opened in the regular
  browser pane are never modified.
- The injection re-runs on every navigation within your plugin, so multi-page
  plugins keep the theme.
- Host-side implementation: `src/renderer/src/lib/webviewTheme.ts` and the
  token map in `src/renderer/src/themes.ts` (`cssVarsOf`).
