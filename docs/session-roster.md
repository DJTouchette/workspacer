# Session roster: one hub-owned list of sessions

Status: in implementation (2026-11-01; amended the same day: no sidebar time window, Hide = archive, permanent history)

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
- it is **stopped** and either:
  - the id is in `open` (on the sidebar, see 2), or
  - the id is in `layout` (desktop curation now only ever *adds* rows).

There is **no time window**. A stopped session stays on the sidebar until the
user hides it, and it is never listed only because it ended recently.

`archived == true` (claudemon's derived 7-day flag) does not matter for a row
in `open` or `layout`. A stopped archived row outside both is not listed. The
shared archive (`sessionArchive`) is the user's **Hide**. It removes the id
from `open` (see 2), so a hidden session drops off every sidebar; clients
still filter by the archive while that change propagates.

Removed rules: "a layout exists and the row is not in it, so hide it", and
"stopped more than 24h ago, so hide it". Both made rows disappear.

Listed rows carry `"starting": true` when `starting(row)` holds, so clients can
render them as Starting instead of guessing from `unknown`.

### 2. Sidebar set (`sessionOpen`)

`sessionOpen` is the shared sidebar: which sessions every client shows. It is
a hub-owned document modelled on `sessionArchive`
(`services/hub-rs/src/services/session_open.rs`, persisted to
`<data>/session-open.json`):

```
{ version, open: { sessionId: sinceMs }, ended: { sessionId: endedAtMs } }
sessionOpen.get  -> document
sessionOpen.set { sessionId, open: bool } -> document   (idempotent, persist-then-publish; low-level)
event sessionOpen.changed (whole document)
```

The hub maintains it itself, so every client agrees:

- Observing a **live** or **starting** row adds the id to `open` (since = now)
  and clears any `ended` mark (a resume).
- Observing a row go **stopped** after this hub run saw it live **keeps it in
  `open`** and records `ended[id] = now` ("ended on purpose", by the user or
  the agent). Nothing leaves the sidebar on its own.
- While the hub is shutting down, `ended` marks are fenced. Sessions stopped
  by the app or host closing therefore stay unmarked and read as **Paused** on
  the next run. The same is true of any session already stopped the first time
  this hub run sees it.
- A stop whose row carries `stop_reason: "idle"` (the engine's idle shutdown,
  see `.rivet/context/domains/session-lifecycle.md`) never gets an `ended`
  mark either: the process was freed, nobody finished the session, so it reads
  **Paused** and its next message resumes it.
- **Hide = archive.** `sessionArchive` archiving an id removes it from `open`
  and `ended`. Restoring it adds it back to `open`, with an `ended` mark if
  the row is stopped. Every client's Hide/Close calls `sessionArchive.set`.
- `sessionOpen.set` is a low-level escape hatch (pin, or unpin without
  archiving). UIs use the archive.
- Capped at `MAX_OPEN = 10_000`. Past the cap, the oldest *ended* entries are
  evicted first, then the oldest overall. Ids are validated like the archive's.

Observation comes from the hub bus `agent.snapshot` stream, so it works both
with an embedded engine and when desktop main is the session provider.

Migration: when `session-open.json` does not exist, seed it once with:

- native's `kept_open` ids (`kept_sessions::read`), unmarked, so they stay Paused;
- every row currently live;
- every row stopped within the last 24h (those not in `kept_open` get an
  `ended` mark).

Nothing a user currently sees disappears at upgrade. `sessions.kept` keeps
its response shape (`{sessions:[{sessionId,since}]}`) and is served from the
open, unended ids, newest first.

**Paused** (every client) = row is stopped **and** id is in `open` **and**
id is not in `ended`. Stopped, in `open` and in `ended` = **Ended** (still on
the sidebar).

An `open` id whose row claudemon has evicted from memory (idle more than 7
days) is still listed: the hub fills it from the history store (section 5) as
a sparse stopped row.

### 3. Events follow the roster

Every gate that used `visible()` uses `listed()` instead. A session that
stops while it is on the sidebar is still in `open`, so its stopped transition
is always published. The hub records the `ended` mark before it publishes that
transition.

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
- **Desktop renderer**: closing an agent card hides its session
  (`sessionArchive.set {archived:true}`), the same Hide as every other client.
- **TUI**: stopped rows in the open set are shown without `toggle_stopped`, as
  paused or ended per the `ended` map.
- **`/m-next`, `/m`**: Paused/Ended from `sessionOpen`. The separate by-id
  `sessions.kept` reads go away because the roster lists open sessions.
- **History panes** (native, desktop, mobile) read `sessions.history`.

### 5. History: every session, forever

History is a separate list from the sidebar. It holds every session ever
seen, including hidden (archived) ones, which are labelled as such.

- **claudemon keeps every session row.** `prune_archived` no longer deletes
  session rows. Event rows may be trimmed for size later, but the session row
  (id, cwd, provider, transport, model, label, transcript path, first and last
  event) is permanent. In-memory eviction after 7 days stays: history is read
  from SQLite, not memory.
- **Paged history query.** claudemon serves
  `GET /history?before=<cursor>&limit=<n>&q=<text>&cwd=<path>` →
  `{sessions:[row…], next?}` from SQLite, newest first (limit 1–200, default
  50; cursor `"<last_event_at>:<id>"`). `GET /sessions/{id}` answers an id
  evicted from memory with its history row (`mode:"stopped"`, `history:true`).
  Both session-list providers expose it on the bus, and the hub registers it
  like `sessions.recent`:
  ```
  sessions.history { cursor?: string, limit?: number, query?: string, cwd?: string }
    -> { sessions: [HistoryRow], next?: string }
  HistoryRow = the sessions.recent row
    { sessionId, provider, cwd, mode, transport, archived, updatedAt, startedAt, name, title, model,
      parentSessionId?: string }  // the session that started it, when the hub knows
    plus { hidden: bool        // in the shared archive (user Hide)
           onSidebar: bool     // in sessionOpen.open
           endedAt?: ms, firstPrompt?: string, userPrompts?: number, toolCalls?: number,
           transcriptCopy: bool }
  ```
  `parentSessionId` comes from the hub's launch record, else the live row;
  History stores no lineage, so a row the hub never launched has none. The
  native client nests history rows under a parent listed in the same group.
  `name` uses the same precedence as `sessions.recent`: launch label, then
  cwd rename, then auto title. `archived` keeps its legacy meaning
  (claudemon's 7-day flag); `hidden` is the user's Hide. With an engine, the
  hub's `live_controls` serves both methods. When desktop main is the session
  provider (external claudemon), desktop main serves them from the same
  claudemon routes. `sessions.recent` keeps its shape and is served from the
  first page.
- **Sidebar ids idle more than 7 days.** The roster fills an `open` id with no
  in-memory row from `GET /sessions/{id}` (cached, refreshed on
  `sessionOpen.changed`), so sidebar sessions never vanish because claudemon
  evicted them from memory.
- **Transcript copies.** claudemon copies each session's provider transcript
  into `<data>/transcripts/<provider>/<sessionId>.<ext>` when the session stops
  and at least every few minutes while it is live (only if the file changed),
  and records the original path. Before a resume, if the provider's original
  file is missing, claudemon restores the copy to the original path. History
  conversation reads fall back to the copy. Copies are deleted only by an
  explicit delete from History.
- **Delete from History**: `sessions.forget {sessionId}` → claudemon
  `POST /sessions/{id}/forget` (409 for a live session). It removes the row,
  events and transcript copy, then drops the id from `sessionOpen` and
  `sessionArchive`. It is the only way anything leaves History. Clients ask
  for confirmation first.

### 6. "Too many sessions": hub-owned cleanup prompt

Because nothing leaves the sidebar on its own, the hub tells every client when
the sidebar needs a cleanup. Each client renders the same prompt.

- The `sessionOpen` document carries a derived block, recomputed on every
  change and published with `sessionOpen.changed`:
  ```
  cleanup: { ended: <n>, threshold: <t>, suggested: <bool>, snoozedAt: <n|null> }
  ```
  `ended` counts sidebar sessions that are stopped, have an `ended` mark and
  are not archived. Paused and live sessions never count.
  `suggested = threshold > 0 && ended >= max(threshold, snoozedAt + 10)`.
  `threshold` comes from hub config (`sessions.cleanupThreshold`, default
  **20**, 0 disables).
- `sessionOpen.snoozeCleanup {}` → document. It sets `snoozedAt = ended`, so
  the prompt returns only after 10 more sessions have ended. A cleanup or a
  manual hide that brings `ended` back under the threshold clears `snoozedAt`.
- `sessionOpen.cleanup { olderThanMs?: number, dryRun?: bool }` →
  `{ hidden: [ids] }` (or `{ wouldHide: [ids] }` when `dryRun` is set). It
  hides (archives) every counted ended session whose `ended` mark is at least
  `olderThanMs` old, or all of them when `olderThanMs` is omitted. It never
  touches Paused or live sessions. This is one archive change: one version
  bump and one `sessionArchive.changed`.
- `sessionArchive.setMany { sessionIds: [..], archived: bool }` → archive
  document. It is a bulk version of `sessionArchive.set`, with one version bump
  and one event (cap 1,000 ids). Clients use it to **undo** a cleanup with the
  returned ids.
- Client UX, the same everywhere:
  - when `cleanup.suggested` is true, show a dismissible banner/notice: "N
    ended sessions on your sidebar".
  - actions: **Hide all ended**, **Hide ended older than a day**, **Not now**
    (snooze). A cleanup shows an **Undo** for a few seconds, which calls
    `setMany archived:false` with the returned ids.
  - the CLI (TUI) shows a one-line notice and a key binding for the same
    actions.
  - every hidden session stays in History.

### 7. Finished but not opened (`sessionSeen`)

A turn that ended while nobody was looking needs the user as much as an
approval does. The hub keeps that as shared view state too, in
`session-seen.json`: `{version, unseen: {sessionId: finishedAtMs}}`.

- **Marked by the hub only.** The open set's observer feeds every row it sees
  to `services/hub-rs/src/services/session_seen.rs`. A mark is added only on
  an OBSERVED working → idle/stopped transition in this hub run. A row that is
  already idle when first seen (upgrade, restart, reconnect) marks nothing,
  and a pending approval or question pauses the turn rather than ending it.
- **Cleared by any client.** `sessionSeen.mark {sessionId}` (triage tier)
  removes the mark; `sessionSeen.get` (view tier) reads the document; every
  change publishes it whole as `sessionSeen.changed`.
- **Rendering.** A marked session that is not busy reads **Finished** and sits
  under the sidebar's Needs you heading. Once seen, an idle session reads
  **Idle**. A decision or a new turn outranks the mark. Native marks the open
  conversation seen when its window is focused, once per finished turn.
  Desktop, web and the CLI do not read the marks yet.

### Clients in scope

Every surface renders the same roster, Paused/Ended/Starting, Hide = archive
and the cleanup prompt:

- **native:** `apps/native`
- **web:** `/app` (the desktop renderer built for the browser, `webBackend.ts`),
  `/m-next`, `/m` and `/remote` (`services/hub-rs/assets/web/remote.html`)
- **desktop:** the Electron app
- **CLI:** `apps/tui`

claudemon's built-in `watch` TUI reads claudemon directly, with no hub, and
stays a raw process view.

## Non-goals / follow-ups

- No `agent.removed` topic yet. Clients still converge on removal by re-reading
  the list. `agents.close` publishing nothing is a pre-existing gap.
- Quiescence on hub-only/peer hubs will now see `starting` stream rows as
  `session-unknown` blockers. That matches the engine path, which already reads
  raw rows.
