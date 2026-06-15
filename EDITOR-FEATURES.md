# Editor Features

Planning + priority doc for the in-app editor (`EditorPane`, CodeMirror 6 engine).

## Context

The editor is a pane inside an agent-orchestration tool. Its primary job is
**reading code an agent just wrote** and making **quick edits** — not authoring
large features from scratch (the `terminal` engine, i.e. your `$EDITOR`/nvim in a
PTY, covers that). This framing drives the priorities below: features that matter
for a full IDE are often *not* must-haves here, and one feature a generic editor
would never prioritize (external-change reload) is our #1.

Two editor engines exist:
- `codemirror` — the in-app editor (this doc). Runs entirely in the renderer.
- `terminal` — your `$EDITOR` in a PTY, handled in `ScrollContainer`.

File I/O goes through `window.electronAPI.readFile / writeFile / readDir`, which
resolves to IPC on desktop and the hub `fs.*` capability on web/remote. Any new
filesystem-touching feature should follow the same pattern: implement once in
Electron main (Node), expose as an IPC handler **and** a hub capability provider
(`src/main/services/hubCapabilities.ts`) — the hub routes, it does not execute.

## Already have (don't rebuild)

In-buffer editing is largely done via CodeMirror's `basicSetup` plus our additions:

- Undo/redo history, multi-cursor / multi-selection
- **Find & replace** (Ctrl+F panel via `searchKeymap`) — regex, case, whole-word
- Code folding, bracket matching, auto-close brackets
- Basic word autocomplete, indent-on-input
- Line numbers, active-line highlight, selection-match highlight
- Syntax highlighting (theme-matched to the ANSI terminal palette)
- Vim mode (`@replit/codemirror-vim`)
- Ctrl+S save + dirty/unsaved indicator
- File-tree sidebar (custom; rooted at the agent's cwd)
- **Git diff** — already exists (the review/diff pane), so a dedicated in-editor
  diff view is *not* needed.

## Must-have (build, ranked)

### 1. External-change detection + reload — highest priority
Agents edit files out from under you. With a file open, if the agent rewrites it
and you hit Ctrl+S, you silently clobber the agent's work with your stale buffer
(`savedRef` only tracks what *you* last wrote, so it can't detect this).

- Watch the open file (fs.watch / hub event).
- On external change: reload if the buffer is clean; if dirty, prompt
  "changed on disk — reload / keep mine".

This is a correctness / data-loss issue unique to the agent workflow — table
stakes here even though a generic editor wouldn't prioritize it.

### 2. Project-wide search
"Where did the agent touch X?" is the most common navigation need in a
read-the-agent's-work tool.

- UI: results panel grouped by file (renderer).
- Backend: a Node search capability (shell out to ripgrep, stream results),
  exposed via IPC + hub capability per the pattern above.

### 3. Open-file plumbing from agent edits
The file tree covers manual navigation; the high-value path is
"agent edited these files → click to open in the editor pane." If an
edited-files / diff signal exists in agent state, wire it to open here. Cheap,
high value.

## Nice-to-have (optional)

- **Go-to-line** (Ctrl+G) — in CodeMirror's search package; low effort.

## Skip (use the `terminal` engine / real IDE instead)

- Breadcrumbs, minimap, git gutter, symbol outline
- LSP / IntelliSense

These are real-IDE chrome. For that depth, the `terminal` engine runs your real
editor with your full config.

## Bottom line

If only one thing ships: **external-change detection / reload** (#1) — it
prevents data loss. Then project-wide search for navigation. Everything else is
either already free in `basicSetup` or genuinely optional for this pane's job.
