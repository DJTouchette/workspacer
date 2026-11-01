# Session roster: one hub-owned list of sessions

Status: in implementation (2026-11-01)

## Problem

Native drops sessions that are still around, and every client lists a
different set of them. The cause is that roster membership is decided in
several places, by different rules:

| Where | Rule today |
|---|---|
| claudemon | hides stopped rows idle more than 7 days; SQLite keeps the newest 100 |
| hub-rs `snapshots::visible` | hides `unknown`; hides a stopped row when a desktop layout exists and the row is not in it, otherwise after 24h |
| desktop main `hubCapabilities.ts` `sessions.snapshots` | TS copy: live store rows plus layout "ghosts"; no 24h fallback |
| desktop `agentStatusSummaryRuntime.ts` `summarySessionVisible` | third copy, used as an auth gate |
| native | replaces its list with `sessions.snapshots` every 30s; local `Settings::kept_open` read by id |
| hub `sessions.kept` | reads native's `native-settings.json` off the host disk |
| TUI | hides stopped rows not seen live this run |
| `/m-next`, `/m` | the hub list, plus `sessions.kept`, minus archive |

Concrete failures:

1. A stopped session that is not in desktop's layout is hidden from the list,
   **and its final stopped `agent.snapshot` is never published**
   (`visible()` also gates `publish`, `live_streams`, `queue`,
   `pull_requests`). Native shows it live until the next 30s fleet read, then
   it disappears.
2. A spawn/resume sits in `unknown` until its first event and is hidden. A
   fleet read during that window drops the row, and native jumps selection to
   the first session.
3. Native's kept set drops a session that ends while watched, so it falls
   straight into case 1.

## Design

### 1. One roster policy (shared contract)

`contracts/session-roster-cases.json` holds vector cases. hub-rs
(`services/hub-rs/src/services/roster.rs`) and desktop TS
(`apps/desktop/src/main/services/sessionRoster.ts`) implement the same pure
function and both load the vectors in tests.

```
listed(row, ctx) -> bool
starting(row, ctx) -> bool

ctx = { open: Set<id>, layout: Set<id>, shells: Set<id>, now }
```

A row is **listed** when its state fields are well-formed and one of these holds:

- it is **live**: mode is not `stopped`/`unknown`, and not (mode empty and
  status `ended`);
- it is **starting**: mode is `unknown`, the id is not in `shells`, and either
  `transport == "stream"` or `provider` is set and not `claude`. A Claude PTY
  row in `unknown` stays hidden, because shells are indistinguishable from it
  by row fields;
- it is **stopped** and any of these holds:
  - the id is in `open` (shared open set, see 2);
  - the id is in `layout` (desktop curation now only ever *adds* rows);
  - `updated_at` is within 24h.

`archived == true` (claudemon's derived 7-day flag) on a stopped row is not
listed unless the id is in `open` or `layout`. The shared archive
(`sessionArchive`) stays a view filter applied by clients. It is not a roster
rule.

Removed rule: "a layout exists and the row is not in it, so hide it". This is
what made native rows disappear.

Listed rows carry `"starting": true` when `starting(row)` holds, so clients can
render them as Starting instead of guessing from `unknown`.

### 2. Shared open set (`sessionOpen`)

A hub-owned document modelled on `sessionArchive`
(`services/hub-rs/src/services/session_open.rs`, persisted to
`<data>/session-open.json`):

```
{ version, open: { sessionId: sinceMs } }
sessionOpen.get  -> document
sessionOpen.set { sessionId, open: bool } -> document   (idempotent, persist-then-publish)
event sessionOpen.changed (whole document)
```

The hub maintains it itself, so every client agrees:

- Observing a **live** or **starting** row adds the id (since = now).
- Observing a row go **stopped** after this hub run saw it live removes it
  ("ended on purpose", by the user or the agent).
- While the hub is shutting down, removals are fenced. Sessions stopped by the
  app or host closing therefore stay open and read as **Paused** on the next
  run. The same is true of any session already stopped the first time this hub
  run sees it.
- `sessionArchive` archiving an id removes it from the open set.
- Clients may call `sessionOpen.set { open:false }` to let a session go (e.g.
  desktop closing a card). `open:true` is allowed for explicit pinning.
- Capped (`MAX_OPEN = 500`), oldest `since` evicted first. Ids are validated
  like the archive's.

Observation comes from the hub bus `agent.snapshot` stream, so it works both
with an embedded engine and when desktop main is the session provider.

Migration: when `session-open.json` does not exist, seed it once from
`kept_sessions::read(native-settings.json)`. `sessions.kept` keeps its
response shape (`{sessions:[{sessionId,since}]}`) but is served from the open
document, newest first.

"Paused" (every client) = row is stopped **and** id is in the open set.

### 3. Events follow the roster

Every gate that used `visible()` uses `listed()` instead. A session's stopped
transition is always within 24h and is therefore always published.

### 4. Clients render the roster

- **Native**: drop `Settings::kept_open`, `KeepSessions` and by-id reads. Paused
  comes from `sessionOpen`. A fleet read still replaces the list, but the
  selected row is always retained, so selection never jumps. Rows changed by
  events during the read still win (`fleet_overlay`). `starting` renders as
  Starting. The serde-ignored legacy `kept_open` field is migrated by the hub.
- **Desktop main** (`sessions.snapshots` / `sessions.snapshot` provider when
  claudemon is external): build the roster with `sessionRoster.ts` from the live
  store plus claudemon's retained rows, with `open` from the hub's
  `sessionOpen`. `summarySessionVisible` uses the same function.
- **Desktop renderer**: closing an agent card calls
  `sessionOpen.set {open:false}` for its session.
- **TUI**: stopped rows in the open set are shown (as paused) without
  `toggle_stopped`.
- **`/m-next`**: Paused from `sessionOpen`; the separate by-id `sessions.kept`
  reads go away because the roster lists open sessions.

## Non-goals / follow-ups

- No `agent.removed` topic yet. Clients still converge on removal by re-reading
  the list. `agents.close` publishing nothing is a pre-existing gap.
- claudemon's 7-day archive and SQLite retention are unchanged. They bound
  what can be resumed, not what is listed.
- Quiescence on hub-only/peer hubs will now see `starting` stream rows as
  `session-unknown` blockers. That matches the engine path, which already reads
  raw rows.
