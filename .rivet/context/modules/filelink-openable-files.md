---
title: FileLink openable-path affordance and editor/preview buses
tags: [renderer-ui, filelink, openable-files, event-bus, ipc]
related_paths:
  - "apps/desktop/src/renderer/src/components/claude/FileLink.tsx"
  - "apps/desktop/src/renderer/src/lib/editorBus.ts"
  - "apps/desktop/src/renderer/src/lib/previewBus.ts"
  - "apps/desktop/src/renderer/src/panes/MarkdownPreviewPane.tsx"
  - "apps/desktop/src/renderer/src/components/claude/ChangedFilesCard.tsx"
owner: Damien Touchette
last_reviewed: 2026-07-11
---

# FileLink openable-path affordance and editor/preview buses

## Overview
`FileLink` is the single clickable-file-path affordance used across the chat tool-call UI: it renders a filename, resolves relativeness against a pane `cwd`, and turns click/right-click into a request on one of two decoupled window-`CustomEvent` buses. `App.tsx` is the sole listener for both buses; it owns pane lifecycle (create-or-focus, dedupe) so `FileLink` itself never touches pane state. Browser-open and reveal-in-Finder/Explorer bypass panes entirely and go straight to Electron main via `window.electronAPI`.

## Key modules
- `apps/desktop/src/renderer/src/components/claude/FileLink.tsx` — `isAbsolutePath`, `resolveWithCwd`, `isMarkdownPath`/`isHtmlPath`, `openFileDefault`, the exported `FileActionMenuItems` menu body, and the `FileLink` component itself (click = open default, right-click = context menu).
- `apps/desktop/src/renderer/src/lib/editorBus.ts` — `EDITOR_OPEN_FILE_EVENT` (`'editor:open-file'`) + `requestOpenInEditor(EditorOpenTarget)`.
- `apps/desktop/src/renderer/src/lib/previewBus.ts` — `MARKDOWN_PREVIEW_EVENT` (`'preview:open-markdown'`) + `requestMarkdownPreview(MarkdownPreviewTarget)`.
- `apps/desktop/src/renderer/src/lib/reviewBus.ts` — the pattern FileLink's buses deliberately mirror (`REVIEW_REQUEST_FILE_EVENT`/`REVIEW_OPEN_FILE_EVENT`); useful reference for the two-step "ensure pane mounted, then deliver" idiom, though editor/preview buses are single-event, not two-step.
- `apps/desktop/src/renderer/src/panes/MarkdownPreviewPane.tsx` — the `mdpreview` pane body; reads the file via `window.electronAPI.readFile` on demand (refresh button, not a live watch) and has its own "Open in editor" button calling `requestOpenInEditor` directly.
- `apps/desktop/src/renderer/src/App.tsx` (~L790-811) — the two `useEffect` listeners: `EDITOR_OPEN_FILE_EVENT` → `openFileInEditor(path)` (~L725, routes to terminal engine or `workspacer.editor` plugin pane depending on `config.editor?.engine`); `MARKDOWN_PREVIEW_EVENT` → `openMarkdownPreview({path, cwd})` (defined in `apps/desktop/src/renderer/src/hooks/useAgentManager.ts` ~L827, dedupes by `previewPath` across the active agent's tabs, otherwise mints an `mdpreview` pane) then `scrollToTab`.
- `apps/desktop/src/main/preload.ts` (~L543-546) — `fileOpenExternal` / `fileShowInFolder` IPC exposed on `window.electronAPI`, both returning `{ ok, error? }`.
- Consumers of `FileLink`/`FileActionMenuItems`: `apps/desktop/src/renderer/src/components/claude/ToolTraceCard.tsx` (tool-call target row), `apps/desktop/src/renderer/src/components/claude/DiffView.tsx` (three header instances, one per diff-card variant), `apps/desktop/src/renderer/src/components/claude/WorkCard.tsx`, `apps/desktop/src/renderer/src/components/claude/ChangedFilesCard.tsx` (via `renderContextMenuItems` on its own `FileTree`, not `FileLink` itself), `apps/desktop/src/renderer/src/panes/ContextPane.tsx`.

## Failure modes
- `openFileInEditor` (App.tsx) warns to console and no-ops if `config.editor?.engine !== 'terminal'` and the `workspacer.editor` plugin pane isn't found (`pluginPanesRef.current`) — no user-visible error surface.
- `MarkdownPreviewPane.load()` catches read errors from `window.electronAPI.readFile` and renders them inline (`Couldn't read {fileName}` + raw error text); it does not retry automatically, only on manual "Refresh" click.
- If a preview pane's `previewPath` is ever undefined (stale/migrated pane state), the pane renders a dead-end message telling the user to reopen from a file link rather than erroring.
- `fileOpenExternal`/`fileShowInFolder` return `{ok:false,error}` on failure but `FileActionMenuItems` calls them with `void ...(...)`, discarding the result — a failed browser-open or reveal fails silently in the UI.

## Gotchas
- Left-click and right-click both call `e.stopPropagation()` in `FileLink` — required because the rows/cards hosting it (tool-trace rows, work-card entries) toggle expand/collapse on click; without this the link's own click would also fire the parent toggle.
- Extension detection (`isMarkdownPath`: `md`/`markdown`, `isHtmlPath`: `html`/`htm`) is the single source of truth for three independent things: default left-click action, which extra context-menu items appear, and the small type glyph (`M↓` / `⊕`) — changing recognized extensions in one place changes all three, which is usually desired but easy to forget when only fixing one symptom.
- `isAbsolutePath` must stay in sync with any new path-origin format; it currently covers POSIX (`/…`), UNC (`\\…`), and Windows drive (`C:\…` or `C:/…`) — a path that doesn't match any of these is treated as relative and gets prefixed with `cwd` via `resolveWithCwd`, so a malformed/unexpected absolute-path style would silently corrupt the path.
- `FileActionMenuItems` is a bare menu-items fragment (no `ContextMenu` wrapper) so it can be reused inside a caller-owned `ContextMenu`/`FileTree` (`ChangedFilesCard.tsx` passes it into `FileTree`'s `renderContextMenuItems`); every item must keep calling `onClose()` via the `run()` wrapper or the host's menu will not close — new actions added to this list must follow the same `run(...)` pattern.
- The editor/preview buses are intentionally *not* merged into `reviewBus.ts`'s pattern even though they serve an analogous purpose; per their doc comments: kept separate so the two concerns stay independent — do not attempt to unify them without checking both `EditorOpenTarget`/`MarkdownPreviewTarget` payload shapes stay distinct from `ReviewFileTarget`.
- `openFileDefault` resolves the path (`resolveWithCwd`) before dispatching, but `FileLink`'s own right-click path passes the *original* `path`/`cwd` into `FileActionMenuItems`, which re-resolves internally — resolution happens twice on two different code paths for the same click session; if `resolveWithCwd`'s logic ever became stateful/non-idempotent this would break.
- `MarkdownPreviewPane` pane dedupe key is `previewPath` exactly (string equality) — opening the same markdown file via two different relative-vs-absolute-looking `path` strings that resolve to the same absolute file will not dedupe unless `FileLink`/`openFileDefault` already normalized to the same absolute string first.

## Hand-authored notes (2026-07-23)

- **Prose linkification**: file paths in ASSISTANT TEXT (and command output) now render as
  FileLinks too, not just tool-card UI. Pure detection in `lib/filePathDetect.ts`
  (tested): code spans allow single-segment code-ext filenames (`package.json`), bare
  prose requires a separator (never linkifies `notifications.post` / domains / URLs);
  `:line[:col]` suffix kept in display, stripped for opening (editorBus has no line
  support yet). Hooked into `components/markdown.tsx` `renderInlineMarkdown` (code spans +
  text runs). Relative paths resolve via React context `MarkdownFileCwdProvider`
  (ClaudePane provides `effectiveCwd`, AgentWatchPane the watched session cwd) — read
  lazily inside the link component so the module-level md parse cache stays valid across
  panes; surfaces without a provider (MarkdownPreviewPane, sidebar cards) render relative
  hits as plain text (no dead links) while absolute paths still link.

## Hand-authored notes (2026-07-29)

- **The default action is now one function, and it is rendered.** `defaultOpenTarget(path)`
  ('editor' | 'preview') is the single branch; `openFileDefault` dispatches from it and
  `FileLink` renders a leading `PaneIcon` for the same target, so the badge cannot advertise
  a surface the click doesn't open (the old gotcha — extension detection feeding three
  independent things — now has one fewer copy). The link also exposes
  `data-open-target="editor|preview"`, which is how the tests assert it.
- The old typographic type marks (`M↓` for md, `⊕` for html) are GONE, replaced by the
  destination icon. Consequence: **an `.html` path now shows the editor icon**, because
  `openFileDefault` has never special-cased html — the browser is only a right-click action.
  If html should open in the browser by default, change `defaultOpenTarget` and the icon
  follows automatically (add a `browser` target to `OPEN_TARGET_UI`).
- `FileLink`'s root span is `display: inline-block; max-width: 100%; overflow-wrap: anywhere`.
  That is load-bearing for prose linkification: without it a soft wrap between the inline
  SVG and the path text strands the icon at the end of one line with its path on the next.
  Call sites that pass `display: flex` in `style` (WorkCard's file rows) override it — that
  row was switched from `alignItems: 'baseline'` to `'center'` because the icon is a flex
  item there and an SVG's baseline is its bottom edge.
- `markdown.tsx`'s `MdPathLink` no longer passes the old `glyph={false}`: prose is where the
  badge matters most, since a linkified path there has no other at-rest affordance (the
  underline is hover-only).
