---
title: Renderer cross-pane event buses (window CustomEvent request/open hops)
tags: [renderer-state, event-bus, cross-pane, custom-event, ui-events]
related_paths:
  - "apps/desktop/src/renderer/src/lib/reviewBus.ts"
  - "apps/desktop/src/renderer/src/lib/editorBus.ts"
  - "apps/desktop/src/renderer/src/lib/workflowBus.ts"
  - "apps/desktop/src/renderer/src/lib/watchBus.ts"
  - "apps/desktop/src/renderer/src/lib/previewBus.ts"
  - "apps/desktop/src/renderer/src/lib/libraryBus.ts"
  - "apps/desktop/src/renderer/src/lib/settingsBus.ts"
  - "apps/desktop/src/renderer/src/lib/uiEvents.ts"
  - "apps/desktop/src/renderer/src/hooks/useUiEventBus.ts"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Renderer cross-pane event buses (window CustomEvent request/open hops)

## Overview
Panes render deep inside an agent's tab/pane tree and have no reference to the tab manager that lives in `apps/desktop/src/renderer/src/App.tsx`. Rather than threading callbacks down through every intermediate component, each pane fires a `window.dispatchEvent(new CustomEvent(...))`; `apps/desktop/src/renderer/src/App.tsx` (and a couple of dedicated overlay hosts) own matching `window.addEventListener` handlers that mutate the real tab/pane tree. This is a family of small, single-purpose "bus" modules under `apps/desktop/src/renderer/src/lib/*Bus.ts`, distinct from `apps/desktop/src/renderer/src/lib/uiEvents.ts`'s outbound publish to the hub bus for plugins/MCP.

## Key modules
- `apps/desktop/src/renderer/src/lib/reviewBus.ts` — `REVIEW_REQUEST_FILE_EVENT` / `REVIEW_OPEN_FILE_EVENT`, the canonical two-step hop; `requestReviewFile()` / `openReviewFile()`.
- `apps/desktop/src/renderer/src/lib/editorBus.ts` — `EDITOR_OPEN_FILE_EVENT`; `requestOpenInEditor()` requests a file or server directory; App chooses the configured editor/plugin path.
- `apps/desktop/src/renderer/src/lib/previewBus.ts` — `MARKDOWN_PREVIEW_EVENT`; `requestMarkdownPreview()` opens/focuses an `mdpreview` pane, used by `FileLink`.
- `apps/desktop/src/renderer/src/lib/workflowBus.ts` — `WORKFLOW_OPEN_EVENT`; `requestWorkflow(runId)` opens the full-height `WorkflowOverlay`, which re-reads the run live rather than freezing a click-time snapshot.
- `apps/desktop/src/renderer/src/lib/watchBus.ts` — bundles watch, inspector, context, and handoff events/helpers: `AGENT_WATCH_EVENT`/`requestAgentWatch`, `SESSION_WATCH_EVENT`/`requestSessionWatch`, `INSPECTOR_OPEN_EVENT`/`requestInspector`, `CONTEXT_OPEN_EVENT`/`requestContextPane`, plus `AGENT_HANDOFF_EVENT`/`requestHandoff`.
- `apps/desktop/src/renderer/src/lib/libraryBus.ts` — `LIBRARY_RUN_EVENT`/`runLibraryItem()` decouples pickers (palette, Library pane, hotkeys) from the single `LibraryHost` runner; `LIBRARY_INSERT_EVENT`/`dispatchInsert()` delivers text into a targeted Claude pane's composer.
- `apps/desktop/src/renderer/src/lib/settingsBus.ts` — `SETTINGS_SECTION_EVENT`/`requestSettingsSection()` plus a one-shot `pendingSection` module variable read via `consumePendingSettingsSection()`, covering the same-tick mount race without a rAF hop.
- `apps/desktop/src/renderer/src/lib/uiEvents.ts` — `emitUiEvent(type, data)`, the helper for publishing these UI lifecycle events to the hub bus (`window.electronAPI.hubPublish`); unrelated wire format (`ui.*` namespace, `source: 'workspacer.ui'`), fire-and-forget.
- `apps/desktop/src/renderer/src/hooks/useUiEventBus.ts` — the sole producer that calls `emitUiEvent`; diffs the whole `agents` tree (`flattenPanes`) when its effect dependencies change to derive `ui.pane.opened`/`ui.pane.closed`/`ui.workspace.focused`/`ui.tab.focused`/`ui.pane.focused`, called once from `apps/desktop/src/renderer/src/App.tsx` as `useUiEventBus(agents, activeAgentId)`.
- Consumers confirmed by grep in `apps/desktop/src/renderer/src/App.tsx` (handlers for `REVIEW_REQUEST_FILE_EVENT`, `EDITOR_OPEN_FILE_EVENT`, `MARKDOWN_PREVIEW_EVENT`, `AGENT_WATCH_EVENT`, `SESSION_WATCH_EVENT`, `INSPECTOR_OPEN_EVENT`, `CONTEXT_OPEN_EVENT`, `AGENT_HANDOFF_EVENT`), `apps/desktop/src/renderer/src/panes/ReviewPane.tsx` (`REVIEW_OPEN_FILE_EVENT`), `apps/desktop/src/renderer/src/components/WorkflowOverlay.tsx` (`WORKFLOW_OPEN_EVENT`), and `apps/desktop/src/renderer/src/panes/SettingsPane.tsx` (`SETTINGS_SECTION_EVENT`).

## Failure modes
- Two-step race: `apps/desktop/src/renderer/src/App.tsx`'s `REVIEW_REQUEST_FILE_EVENT` handler creates/finds the Review pane's tab, then fires `openReviewFile()` inside a *double* nested `requestAnimationFrame` (`requestAnimationFrame(() => requestAnimationFrame(() => openReviewFile(...)))`) to defer delivery until after pane creation. This is a scheduling heuristic, not proof that an uncached lazy import has resolved; test the cold-mount path when changing it. Collapsing the two events into one, or firing `openReviewFile` synchronously right after pane creation, silently drops the payload — nothing is listening yet.
- `ReviewPane` also filters by `cwd`: it ignores `REVIEW_OPEN_FILE_EVENT`s whose `cwd` doesn't match its own, and App's reuse search checks normalized cwd when both sides supply one. Missing cwd is a permissive match. Preserve both checks when changing reuse; neither is filesystem canonicalization or authorization.
- Plain `window` events provide no delivery queue: if a handler isn't mounted and no rAF/pending-value trick is used, the event is lost with no error, retry, or console warning.
- `useUiEventBus` is diff-based, not action-based: it only fires open/close on transitions relative to the *previous* render's tree, guarded by `prevPanes.current`/`prevFocus.current` starting `null` so the very first render seeds silently (no burst of "opened" events on session restore). A bug that resets `agents` to a fresh array/identity without an actual pane change look like nothing (values compared by id, not identity), but any code path that races `agents` updates across renders can under/over-fire this diff.
- `emitUiEvent` catches synchronous publish errors (`try { hubPublish } catch { /* non-critical */ }`) — it does not await or attach a rejection handler to a returned Promise. Do not infer reliable delivery or asynchronous error reporting from this wrapper.

## Gotchas
- Shared shape across the `apps/desktop/src/renderer/src/lib/*Bus.ts` family: one exported `*_EVENT` string constant, a typed `interface *Target` for `detail`, and a plain `request*`/`open*` function wrapping `window.dispatchEvent`. New PaneTypes needing a similar cross-tree open should add a sibling bus file in this shape rather than threading a callback prop through the tree — every module's own doc comment says as much (watchBus.ts explicitly names reviewBus/workflowBus as the pattern precedent).
- Don't confuse this bus family with `apps/desktop/src/renderer/src/lib/uiEvents.ts`: the `apps/desktop/src/renderer/src/lib/*Bus.ts` files are inbound-to-App (renderer-internal, pane → App), while `apps/desktop/src/renderer/src/lib/uiEvents.ts`'s `emitUiEvent` is outbound-to-hub (App → hub bus → plugins/MCP), namespaced `ui.*` with `source: 'workspacer.ui'`. They never call each other directly; `useUiEventBus` observes the *result* of the `apps/desktop/src/renderer/src/lib/*Bus.ts` handlers running (the mutated `agents` tree), not the bus events themselves.
- `workflowBus.ts`'s `requestWorkflow(runId)` passes only a `runId` string as `detail`, not a run object — `WorkflowOverlay` re-derives `{ sessionId, run }` by scanning `snapshotBySession` for a workflow with that `runId` on every `snapshotBySession` change (`useMemo` dep), so the overlay tracks the live run and renders `null` if the run disappears from snapshots (e.g. session cleared) rather than freezing stale data.
- `settingsBus.ts` uses a different race-avoidance trick than reviewBus's rAF hop: a module-level `pendingSection` variable set by `requestSettingsSection()` and read once via `consumePendingSettingsSection()` in `SettingsPane`'s mount effect — this only works because `SettingsPane` calls the consumer synchronously on mount in the same effect that also subscribes to `SETTINGS_SECTION_EVENT`; adding an async gap before that consume call would reintroduce the drop hazard rAF was built to avoid elsewhere.
- Several watchBus/App.tsx handlers also side-effect the fleet/piloting `viewLevel` (`if (viewLevel === 'fleet') setViewLevel('piloting')`) so an opened pane is visibly surfaced from under the Fleet Deck overlay — a detail easy to miss when adding a new bus consumer that assumes the target pane is always visible once created.
- `libraryBus.ts`'s `dispatchInsert` carries session/pane selectors for mounted Claude panes to filter. Watch/session-watch requests are instead consumed by App, which opens or focuses the target pane; an attached viewer does not own the session lifetime. Review open-file delivery can reach multiple Review panes and filters by cwd, not a unique pane ID.

## Additional entry points

`apps/desktop/src/renderer/src/lib/browserBus.ts` carries browser-open requests,
including checked local-file and Markdown-preview detours. FileLink HTML menu
actions use this path rather than launching an external browser directly.

`settingsBus.ts` also has policy-settings navigation and task-workflow project
selection. The latter carries a pending project value and opens the supervisor
settings section; it is not a new workflow pane type. Settings pending values
are single slots, not FIFO queues, so a newer pre-mount request replaces the old
one. Preserve the target payload and its consumer together.

`watchBus.ts` also exports `requestRecentSessionsRefresh()` to nudge the sidebar
after termination. Event payload types are compile-time contracts, not runtime
validation. These window events are local to one renderer and have no replay,
acknowledgement, or cross-window delivery guarantee. The settings slots retain
the last request until consumed; handling a live section event does not itself
clear its pending slot.
