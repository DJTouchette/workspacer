# Plan: Full tmux-style remote mirror (hub-owned layout document)

> **STATUS: IMPLEMENTED (all 4 phases).** Hub layout service + in-process
> provider, desktop call path, web call path, and the renderer `useLayoutSync`
> thin-view layer are in. Builds green; layout + local-provider tests pass.
> Known v1 limits: last-writer-wins (no CRDT); desktop hydrate runs once at
> startup so if the hub isn't connected yet it falls back to local restore and
> self-heals on the next remote change; viewMode still rides config, not the doc.

**Goal:** The web remote viewer mirrors the Electron app bidirectionally, like tmux —
whatever is on the desktop shows up on the web and vice versa: same agent cards, tabs,
panes, active tab, view mode, focused agent. Change one, the other follows live.

**Decisions (locked in):**
- Mirroring fidelity: **full tmux mirror** (shared layout document, not just same-agents).
- State owner: **the hub (Go bus)**.

## Where we stand today

The hard part is already done by the "Way better remote" commit (`17d5766`):

- **claudemon (Rust) = the tmux server.** Owns every Claude session + PTY. Both clients
  are viewers; neither runs anything.
- **hub (Go) = transport.** Today a *pure router* — `rpc.go`: "The hub never executes a
  capability — it only routes." Holds zero app state.
- **Web renderer = the same React app** as Electron. `apps/desktop/src/renderer/src/backend/webBackend.ts`
  reimplements the entire `window.electronAPI` surface against the hub-bus WebSocket
  (`hubBusClient.ts`). React can't tell preload-bridge from WebSocket apart.
- **Terminal content already mirrors like tmux:** both clients attach the same PTY
  (`sessions.attachTerminal` → `pty.bytes.<sessionId>`), input from either side hits the
  same PTY, resize from one reflows the other (`hubCapabilities.ts:259` — intentional).

### Why it still feels like a blank slate

The gap is **UI shell state, not session data** (tmux: pane *contents* shared, but
window/pane *layout* + active window not). In Workspacer the layout — which agent cards
exist, their tabs/panes, active tab, view mode, focused agent — lives in each renderer's
local React state (`useAgentManager` `useState`, `App.tsx`). Desktop restores it from a
saved-session file; web starts from `withGlobalWorkspace([])` (empty).

Partial recovery exists: `App.tsx:228-241` auto-adopts any live daemon session into a
fresh card — but it rebuilds a *default* layout (`defaultAgentTabs`), and the two renderers
hold *independent* copies, so neither hears the other's changes.

## The core idea

Give the hub one new job: **own a layout document, answer reads/writes, broadcast every
change.** Both renderers become thin views that render the doc and dispatch mutations —
extends the "UI is rendering-only" principle from session data to the window manager.

```
   ┌─────────────┐    layout.get / layout.apply (RPC)
   │ Electron     │◄──────────────┐
   │ renderer     │               │   ┌──────────────────────┐
   └─────────────┘   layout.changed│   │ HUB (Go)              │
        ▲ preload/main             ├──►│  + layout service     │ ← NEW: stateful
        │                          │   │    holds the doc      │   provider, not
   ┌─────────────┐                 │   │    broadcasts changes │   just a router
   │ Web renderer │◄───────────────┘   └──────────────────────┘
   └─────────────┘   (direct bus via webBackend)
```

## Phases

### Phase 1 — Hub layout service (Go) [foundational, self-contained]
New `services/hub/internal/layout` package. Holds the layout doc in memory (shape of
`AgentWorkspace[]` + globals: active agent, view mode). Registers as a bus *provider* for:
- `layout.get` → full snapshot
- `layout.apply` → a granular mutation (`openTab`, `closeTab`, `setActiveTab`,
  `splitPane`, `setActiveAgent`, `setViewMode`, `reorder`, …), applied last-writer-wins
- On every apply: `broker.Publish("layout.changed", …)` so all clients reconcile.
- Persist to a JSON file so the doc survives a hub restart (the reason hub-owned beats
  Electron-owned: the web can hold the fleet open with the desktop closed).

Can be built + tested against the bus before touching any renderer — verify the doc syncs
over `layout.changed` first.

### Phase 2 — Desktop call path (TS, main)
`hubClient.ts` today only *provides* + *publishes*. Add an outbound
`callHub(method, params)` (caller role, id-correlated, same pattern as `webBackend`'s
client). Expose `layout.get/apply` + a `layout.changed` subscription through `ipc.ts` +
`preload.ts`, matching how `config.*` is wired.

### Phase 3 — Web call path (TS, web)
Trivial: add `layoutGet`/`layoutApply`/`onLayoutChanged` to `webBackend.ts` — map straight
to `client.call('layout.…')` and `client.subscribe('layout.changed')`.

### Phase 4 — Renderer becomes a thin view (TS, renderer)
Refactor `useAgentManager`: seed from `layout.get`, subscribe to `layout.changed`, and
every setter (add tab, switch tab, split pane, focus agent, view mode) dispatches
`layout.apply` instead of mutating local state. Optimistic local apply + reconcile on the
broadcast echo to stay snappy. The auto-adopt logic in `App.tsx:228` moves server-side
(hub adds a card when a new session appears), so both clients see the same adoption.

PTY content mirroring stays exactly as-is — this layer sits on top.

## Open decision before building

Migration of saved sessions: today the desktop persists layout via saved-session files
(`sessionService`). Once the hub owns the live doc, those become *named snapshots* you load
into the live doc, not the source of truth. Clean split, but it changes desktop startup
(restore from hub doc, not last session file).

## Key files

- `services/hub/internal/bus/rpc.go`, `services/hub/internal/bus/bus.go` — router + provider registration
- `services/hub/internal/broker/broker.go` — pub/sub
- `services/hub/cmd/hub/main.go` — wiring
- `apps/desktop/src/main/services/hubClient.ts` — main-process bus client (add caller role)
- `apps/desktop/src/main/services/hubCapabilities.ts` — existing capability registrations (reference)
- `apps/desktop/src/main/ipc.ts`, `apps/desktop/src/main/preload.ts` — desktop IPC surface
- `apps/desktop/src/renderer/src/backend/webBackend.ts`, `hubBusClient.ts` — web bus client
- `apps/desktop/src/renderer/src/hooks/useAgentManager.ts`, `apps/desktop/src/renderer/src/App.tsx` — layout state
- `apps/desktop/src/renderer/src/types/pane.ts` — `AgentWorkspace`/`TabConfig`/`PaneConfig` shapes
