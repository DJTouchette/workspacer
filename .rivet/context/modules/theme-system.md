---
title: Theme system: tokens, resolveTheme, custom themes & theme maker
tags: [renderer-ui, theme, custom-themes, css-vars, xterm]
related_paths:
  - "apps/desktop/src/renderer/src/themes.ts"
  - "apps/desktop/src/renderer/src/hooks/useTheme.ts"
  - "apps/desktop/src/renderer/src/components/settings/ThemeMaker.tsx"
  - "apps/desktop/src/renderer/src/components/settings/AppearanceSection.tsx"
  - "apps/desktop/src/renderer/src/lib/webviewTheme.ts"
owner: Damien Touchette
last_reviewed: 2026-07-11
---

# Theme system: tokens, resolveTheme, custom themes & theme maker

## Overview
`apps/desktop/src/renderer/src/themes.ts` defines the `Theme` interface (surface/border/text/accent/status tokens + a `TerminalTheme` xterm palette + a `CornerStyle`), 18 built-in `Theme` objects, and the resolve/apply pipeline that turns a theme id into live CSS custom properties (`--wks-*`) on `document.documentElement`. `apps/desktop/src/renderer/src/hooks/useTheme.ts` is the per-render orchestrator; `apps/desktop/src/renderer/src/components/settings/ThemeMaker.tsx` + `apps/desktop/src/renderer/src/components/settings/AppearanceSection.tsx` are the settings UI for forking a built-in into a user-editable `CustomTheme` stored in `config.ui.customThemes`. `apps/desktop/src/renderer/src/lib/webviewTheme.ts` re-projects the same tokens into embedded plugin `<webview>` documents, which don't inherit the host's CSS custom properties.

## Key modules
- `apps/desktop/src/renderer/src/themes.ts` — `Theme`/`TerminalTheme`/`CustomTheme` types, 18 built-in themes (`themes` registry), `resolveTheme`, `cssVarsOf`, `applyTheme`, `applyCorners`, `cornersOf`, `isCustomThemeId`, `newCustomThemeId`, `themeColorsOf`, `themeDisplayName`, `toHex`/`toRgba`, `titleBarOverlayOf`, `isLightTheme`.
- `apps/desktop/src/renderer/src/hooks/useTheme.ts` — `useTheme()` reads `config.ui.theme`/`cornerStyle`/`customThemes`/`borderColor`, calls `resolveTheme` → `applyTheme` + `applyCorners`, repaints the Windows title-bar overlay, and applies the focused-pane border override (`--wks-border-active`).
- `apps/desktop/src/renderer/src/components/settings/AppearanceSection.tsx` — theme picker (`themeOptions` built from `themes` + `customThemes`), `createTheme()` which forks the *currently rendered* theme via `themeColorsOf(activeTheme)` into a new `CustomTheme`.
- `apps/desktop/src/renderer/src/components/settings/ThemeMaker.tsx` — live token editor for the active custom theme; debounced (~300ms) save-to-config on every color change; `duplicate()` and `remove()` (delete falls back to `draft.base` or `'dark'`, and clears `cornerStyle`/`borderColor` overrides).
- `apps/desktop/src/renderer/src/lib/webviewTheme.ts` — `webviewThemeCSS`/`webviewThemeJS` inject `--wks-*` vars + `color-scheme` + `window.__WKS_THEME__` + a `wks-theme` `CustomEvent` into plugin webview guest pages (documented contract: `services/hub/docs/plugin-theming.md`).
- `apps/desktop/src/renderer/src/hooks/useConfig.ts` — `UIConfig.customThemes?: CustomThemes`, `cornerStyle`, `borderColor` fields persisted in `config.yaml` (`ui.*`).

## Failure modes
- Unknown theme id (built-in or custom) silently falls back to `darkTheme` in `resolveTheme` — no error surfaced; a bad `config.ui.theme` value just looks like a reset.
- `isLightTheme` only parses `#rrggbb` hex or `rgb()/rgba()` strings; other CSS color syntaxes (named colors, `hsl()`, `color-mix()`) fail the regex and it silently assumes dark, which can pick the wrong native `color-scheme` for a custom theme using such values.
- `ThemeMaker`'s debounced save (300ms `setTimeout`) is flushed on unmount via `pendingSaveRef` so navigating away mid-edit doesn't drop the last keystroke, but a crash/quit inside the debounce window (before `useConfig`'s own save/quit handshake) can still lose the trailing edit.
- `ColorRow` text inputs accept arbitrary CSS color strings (rgba/color-mix); `toHex()` best-effort-parses only hex/rgb for the swatch and silently returns `#000000` for anything it can't parse — the swatch can visually disagree with the actual applied color.

## Gotchas
- **Custom themes are stored fully resolved, never deep-merged against the base.** `AppearanceSection.createTheme()` snapshots `themeColorsOf(activeTheme)` (every token, flattened) into `CustomTheme.colors` at creation time. Editing a built-in theme's values in `themes.ts` afterward never restyles previously-saved custom themes forked from it — this is intentional (comment in `themes.ts` above `CustomTheme`), not a bug.
- **`resolveTheme` still spreads the stored `base` under `custom.colors`** (`{ ...base, ...custom.colors, terminal: {...} }`) purely to backfill tokens *added to the `Theme` interface after* the custom theme was saved — old custom themes missing a newer field (e.g. `busy`, `borderActive`) get it from `base`, not from a current built-in of the same name (base is resolved via the static `themes` registry, so if that built-in itself changed, the old value from theme-creation time — not the new one — was already baked into `colors`, so this backfill only fills gaps, never overwrites saved edits).
- **`base` is also the delete fallback**: `ThemeMaker.remove()` reverts `config.ui.theme` to `draft.base` (or `'dark'` if that base no longer exists).
- **Adding a new `Theme` token is a 3-place exhaustive change**: (1) every built-in `Theme` object in `themes.ts` needs a real value (optional tokens like `busy`/`purple`/`borderActive` get inline `??` fallbacks in `cssVarsOf`/`ThemeMaker` instead), (2) `cssVarsOf()` must emit the new `--wks-*` var, (3) `ThemeMaker`'s `TOKEN_GROUPS` (or `TERMINAL_TOKENS`) must list it or the editor can't touch it — saved custom themes then rely purely on the resolve-time base-backfill described above until re-saved.
- **`custom:<slug>` id scheme**: `isCustomThemeId` is a prefix check (`CUSTOM_THEME_PREFIX = 'custom:'`); `newCustomThemeId(name, existing)` slugifies the *name* and appends `-2`, `-3`, … only to avoid id collisions — two custom themes CAN share a display `name` while having distinct ids. `themeDisplayName(id, customThemes)` returns `custom.name` for customs, else prettifies the built-in id (`'tokyo-night'` → `'Tokyo Night'`).
- **Corners**: each `Theme.corners` is backfilled at module-load time from `THEME_CORNERS` (curated per-theme defaults, default `'soft'`) via a `for` loop over the `themes` registry — custom themes never get a `corners` field (`ThemeColors` explicitly omits `name`/`corners`/`terminal` shape mismatch), so `cornersOf()` for a custom theme falls through to `theme.corners` which is `undefined` unless `resolveTheme` spread it from `base` (it does, since `corners` lives on the base `Theme` object) — effectively customs inherit their base's curated corner style unless the user sets `config.ui.cornerStyle`.
- **Two independent projections of the same tokens must stay in sync**: `apps/desktop/src/renderer/src/hooks/useTheme.ts`'s `applyTheme` (host document) and `apps/desktop/src/renderer/src/lib/webviewTheme.ts`'s `webviewThemeCSS`/`webviewThemeJS` (plugin webviews) both call the *same* `cssVarsOf(theme)` — do not hand-duplicate the var list in either caller.
- **Literal fallbacks are load-bearing**: ~93 files use `var(--wks-*, <literal>)` (e.g. `var(--wks-bg-elevated, #1e1e21)` in `apps/desktop/src/renderer/src/App.css`); before `useTheme`'s first effect runs (or before any theme is applied, e.g. very early paint) the UI renders from these literals, so they should be kept visually close to `darkTheme`.
