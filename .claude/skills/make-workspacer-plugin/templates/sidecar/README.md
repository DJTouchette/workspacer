# My Sidecar — a workspacer sidecar plugin

A long-running sidecar process that connects to the workspacer **hub bus**, reacts
to agent/workspace events, and can call bus capabilities. Zero runtime
dependencies — pure Node built-ins (Node >= 22 for the global `WebSocket`).

## Layout

```
plugin.json   manifest (id, apiVersion "1", server, panes, settings, capabilities, consumes …)
server.js     the sidecar: bus connect + reconnect, call/publish/log, /health + status pane
README.md     this file
LICENSE       MIT
.gitignore    ignores loader-owned files (.bus-token, .settings.json, …) and node_modules
```

## Make it yours

1. Edit `plugin.json`:
   - `id` → `owner.name` (unique).
   - Keep `apiVersion` exactly `"1"`.
   - List the capabilities you'll call in `capabilities[]`, and the events you
     want in `consumes[]`. **Filesystem capabilities** (`fs.*`, `search.project`)
     must use the object form `{ "method": "fs.read", "paths": ["${pluginDir}"] }`.
2. Put your logic in `onEvent()` in `server.js`. Use `call(method, params)` to
   invoke a capability and `publish(type, data)` to emit an event you declared.
3. To watch files, use node's local `fs.watch` — the `fs.watch` *capability* is
   pane-scoped and won't resolve for a sidecar token.

## Token & settings

- The hub injects the bus token as the `HUB_TOKEN` env var (the scaffold also
  falls back to `WKS_BUS_TOKEN` / a `.bus-token` file).
- Settings arrive as JSON in `WKS_SETTINGS`, read **only at spawn** — changing a
  setting restarts the sidecar.

## Verify

```sh
node --check server.js     # must be clean
node server.js             # must start and keep retrying if the hub is down
curl -s 127.0.0.1:9200/health   # → ok
```

## Do NOT commit

The loader owns these (already in `.gitignore`): `.bus-token`, `.settings.json`,
`.install-source`, `.disabled`, `node_modules/`.

## Learn more

See the project skill `make-workspacer-plugin`, the manifest schema
(`services/hub/internal/plugin/manifest.go`), and the live example
`../workspacer-plugins/test-on-save/server.js`.
