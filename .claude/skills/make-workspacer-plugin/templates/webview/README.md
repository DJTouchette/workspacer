# My Webview — a workspacer webview plugin

Static HTML/JS the hub serves at `/plugins/ui/<id>/`. No process to run — the
trusted hub serves the files and the page talks to the **hub bus** with a scoped
token injected into the pane URL. Ideal for dashboards / HUDs / live panels.

## Layout

```
plugin.json     manifest (id, apiVersion "1", ui: "ui", panes, hotkeys, settings, capabilities, consumes …)
ui/index.html   the pane: bus connect + reconnect, call/publish, onEvent, --wks-* theming, live settings
README.md       this file
LICENSE         MIT
.gitignore      ignores loader-owned files (.bus-token, .settings.json, …)
```

## Make it yours

1. Edit `plugin.json`:
   - `id` → `owner.name` (unique). Keep `apiVersion` exactly `"1"` and `ui` = the
     assets subdir (`"ui"`).
   - List capabilities you call in `capabilities[]` and events you want in
     `consumes[]`. **Filesystem capabilities** (`fs.*`, `search.project`) must use
     the object form `{ "method": "fs.read", "paths": ["${pluginDir}"] }`. Webview
     pane tokens can also resolve `${agentCwd}` (the pane's agent working dir).
2. In `ui/index.html`, keep `SOURCE` and `TOPICS` in sync with the manifest's `id`
   and `consumes`. Use `call()` / `publish()` and handle events in `onEvent()`.

## Token, theme & settings

- Token: read from `location.search` as `?busToken=…`; the WS URL is built from
  `location.host` (falls back to `ws://127.0.0.1:7895/bus`), passed as `?token=…`.
- Theme: style with `var(--wks-*, fallback)` CSS vars supplied by the host.
- Settings: delivered **live** via `window.__WKS_SETTINGS__` plus a settings bus
  event — no reload (see `applySettings`).

## Verify

Open `ui/index.html` in a browser: it must parse with no JS errors and show
"disconnected" (then keep trying to reconnect) when there is no bus.

## Do NOT commit

The loader owns these (already in `.gitignore`): `.bus-token`, `.settings.json`,
`.install-source`, `.disabled`.

## Learn more

See the project skill `make-workspacer-plugin`, the manifest schema
(`services/hub/internal/plugin/manifest.go`), and the live example
`../workspacer-plugins/cost-hud/ui/index.html`.
