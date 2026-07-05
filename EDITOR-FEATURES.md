# Editor

How Workspacer opens files for editing. The old in-app CodeMirror `EditorPane`
(and its build-it-ourselves roadmap) has been **removed**; editing now happens
one of two ways.

## 1. Terminal engine — your `$EDITOR` in a PTY

The built-in `'editor'` pane type renders the **terminal engine**: it runs your
own editor (nvim, helix, …) in a PTY via `TerminalPane`. See
`components/ScrollContainer.tsx` (the `case 'editor'` branch).

Config lives in **Settings → Editor** (`components/settings/EditorSection.tsx`,
persisted as `config.editor`):

- `editor.engine` — `'terminal'` selects this path.
- `editor.terminalCommand` — the command to launch (default `nvim`); the file
  path is appended as the last argument. Must be on the daemon host's PATH.

This gives you your real editor with your full config, and is the right tool for
authoring from scratch.

## 2. Sandboxed editor plugin (`workspacer.editor`)

The in-app code editor is now a **plugin**, not core code — see
`services/hub/examples/editor/` (`plugin.json` + `ui/`). It contributes its own
`workspacer.editor` pane type (a webview) and a hotkey (`ctrl+shift+d`, "Open
Editor"). The app opens it rooted at the project cwd
(`App.tsx`, `pluginId === 'workspacer.editor'`); if the plugin isn't installed,
opening the editor is a no-op with a console warning.

Because it runs in the plugin webview sandbox, it only gets the filesystem
through **scoped hub capabilities** declared in its manifest, confined to
`${agentCwd}`:

- `fs.read`, `fs.write`, `fs.listEntries` — read/edit/browse.
- `fs.watch` / `fs.unwatch` + `consumes: fs.changed` — external-change
  detection (agents rewrite files out from under you).
- `search.project` — project-wide search.

Plugin settings: `vimMode`, `tabSize`, `lineWrap`.

> Note: the `engine: 'codemirror'` option still exists in `EditorSection` and is
> the default `config.editor.engine`, but the built-in `'editor'` pane no longer
> renders an in-app editor for it — it shows a message pointing you at the
> plugin (Open Editor). Treat `codemirror` as "use the plugin".

## File I/O pattern (for any editor-side feature)

Filesystem access goes through `window.electronAPI.readFile / writeFile /
readDir`, which resolves to **IPC on desktop** and the hub **`fs.*` capability
on web/remote**. Any new filesystem-touching feature follows the same shape:
implement once in Electron main (Node), expose as an IPC handler **and** a hub
capability provider (`src/main/services/hubCapabilities.ts`). The hub routes; it
does not execute. Plugins never get raw Node — only the scoped `fs.*` /
`search.project` capabilities their grant declares (see
`services/hub/internal/capspec` + `internal/bus/policy.go` for the path
confinement).
