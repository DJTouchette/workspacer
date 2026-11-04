# Host work queue: resource-gated jobs and prompts across federated hosts

Status: in development on `feat/host-work-queue`. This file is the wire contract
the hub (`services/hub-rs`) and the native client (`apps/native`) build against.
Change it here first if the contract has to move.

## Goals

- A host runs agent prompts and shell scripts through a **queue** that admits
  work only when it has a free **slot** and the **resource gate** passes.
- A submitter (a scheduled job, a prompt typed in a client, an agent) picks the
  **host**: this machine, a named federation peer, or `any` (best fit).
- Clients show every host's resources, slots, and queue, and let the owner
  cancel queued work and tune the limits.

Non-goals for this iteration: bin-packing, preemption, moving running work,
automatic retry of work whose outcome is unknown, Electron desktop UI.

## Invariants (security and recovery)

1. A peer's advertised resources choose a target only. They never grant
   anything. The destination re-checks admission and authority on its own.
2. Work whose outcome is uncertain (the host restarted while it ran, the peer
   dropped during submit) becomes `unknown`. It is never re-run automatically,
   locally or elsewhere.
3. Shell items are host code. Submitting one requires the server owner (the
   same check as `jobs.upsert`). Spawn items require the authority that
   `agents.spawn` requires for that caller.
4. The queue file is written atomically (0600 on POSIX), like `jobs.json`.
5. Remote submission goes through the existing qualified call
   `hub:<peer>/queue.submit`, only to peers with `dispatch: true`. Nothing new
   bypasses federation's credential checks.

## Resources: `host.resources`

The hub samples resources every 5 s. Linux PSI comes from `/proc/pressure/*`.
Elsewhere those fields are `null`, which means unknown, not zero.

`host.resources` RPC (any authenticated caller) and `host.resources` event
(published every 15 s and when gate state changes):

```json
{
  "host": "local-hub-name",
  "sampledAt": 1762300000000,
  "cpu":    { "cores": 8, "load1": 1.2, "usagePct": 23.5, "pressureAvg10": 3.1 },
  "memory": { "totalMb": 31800, "availableMb": 12000, "pressureAvg10": 0.0 },
  "io":     { "pressureAvg10": 0.4 },
  "disk":   { "path": "/home/me/.config/workspacer-hub", "freeMb": 80000, "totalMb": 500000 },
  "queue":  {
    "agentSlots": 4, "agentsRunning": 1,
    "shellSlots": 2, "shellRunning": 0,
    "queued": 3,
    "gate": { "open": true, "reasons": [] }
  }
}
```

`gate.reasons` holds human-readable strings such as
`"memory pressure 34.0 >= 25.0"` or `"available memory 700 MB < 1024 MB"`.
Any reason closes the gate. Unknown metrics never close it.

## Queue limits (shared config, `queue.*` keys)

| Key | Default | Meaning |
|---|---|---|
| `queue.maxAgents` | 4 | Concurrent queue-started agents holding a slot |
| `queue.maxShell` | 2 | Concurrent shell items |
| `queue.maxCpuPressure` | 60 | PSI cpu some avg10 (%); at or above closes the gate |
| `queue.maxMemoryPressure` | 25 | PSI memory some avg10 (%) |
| `queue.minAvailableMemoryMb` | 1024 | |
| `queue.minFreeDiskMb` | 2048 | Free space on the hub state disk |

## Queue items

```json
{
  "id": "q_01J...",
  "kind": "spawn" | "shell",
  "label": "nightly triage",
  "priority": 0,
  "state": "queued" | "running" | "done" | "failed" | "cancelled" | "unknown",
  "source": { "kind": "job" | "prompt" | "agent" | "remote", "jobId": "...", "origin": "peer-name-or-empty" },
  "spawn": { "provider": "claude", "cwd": "/abs/path", "prompt": "...", "model": "...", "effort": "...", "permissionMode": "..." },
  "shell": { "command": "make test", "cwd": "/abs/path", "timeoutSec": 3600 },
  "enqueuedAt": 0, "startedAt": 0, "finishedAt": 0,
  "sessionId": "set once a spawn item started",
  "exitCode": null,
  "detail": "last line of output, refusal, or error (≤ 2000 bytes)",
  "waitReason": "why it is still queued: no free slot / gate reasons"
}
```

Exactly one of `spawn` or `shell` is present, matching `kind`.

Ordering is by higher `priority` first, then FIFO by `enqueuedAt`. A head item
that cannot start because of a kind-specific slot does not block the other
kind.

Slot lifetime:
- **shell**: from process start to exit (timeout kills it; the default
  `timeoutSec` is 3600, capped at 86400).
- **spawn**: from `agents.spawn` until the agent first becomes idle after its
  prompt (turn finished or waiting for input), or ends, or is stopped. The
  item is then `done` (`failed` if the spawn or prompt delivery failed, or the
  session ended with an error). An agent waiting for the user does not hold a
  slot.

On hub start, items persisted as `running` become `unknown` with detail
`"hub restarted while running"`. Queued items survive restarts.

Retention: terminal items keep the newest 200; queued and running items are
never evicted.

Storage: `queue.json` next to `jobs.json` (else in the hub data dir), shape
`{"version": 1, "items": [...]}`. A file that cannot be parsed is renamed to
`queue.damaged-<ms>.json` (never overwritten) and the queue starts empty.

Identity (hub implementation notes):
- A federation peer reaches the destination over `/bus?peer=1` with a
  credential the destination issued, so the destination sees `federated: true`,
  `authenticated_host: false` (also for the host token), `trusted` only for the
  host token or an operator token, and `token_id` = that credential's
  fingerprint. The destination does not learn the peer's *name*.
- `source.kind` is stamped by the destination: a federated caller is always
  `remote` (its `source.origin` is the submitter's own display label, not
  verified); any other caller asking for `remote` gets `prompt`, and its
  `origin` is cleared. The remote submitter's credential fingerprint is stored
  with the item (never on the wire) and is what lets that peer cancel it.
- Consequence of invariant 3: a federated caller is never the server owner, so
  a peer cannot submit **shell** items to another hub. Remote placement of
  shell work needs an explicit contract change first.
- Spawn authority: the host or an operator credential (`trusted`, local or
  federated), or a scoped tier that lists `agents.spawn` (none today). Plugin
  connections are refused. The item's agent is started by the hub's own bus
  client with only `provider`, `cwd`, `prompt` (as the spawn `message`),
  `model`, `effort`, `permissionMode`, `label` and, for `source.kind == "job"`,
  `jobId`.
- Wire items always carry `sessionId`, `exitCode`, `detail` and `waitReason`
  (empty / `null` when not applicable).

## RPCs

| Method | Auth | Params → Result |
|---|---|---|
| `host.resources` | authenticated | `{}` → resources object above |
| `queue.list` | authenticated | `{ "includePeers"?: bool }` → `{ "host": "...", "items": [...], "peers": [{ "peer": "name", "ok": bool, "error"?: "...", "items": [...] }] }`. Peer fan-out uses a 10 s per-peer budget; a failed peer costs only its rows. |
| `queue.submit` | spawn: as `agents.spawn`; shell: owner | `{ "host"?: "" \| "local" \| "<peer>" \| "any", "kind", "label"?, "priority"?, "spawn"? \| "shell"?, "source"? }` → `{ "host": "<where it landed>", "item": {...} }` |
| `queue.cancel` | owner, or the submitter's own peer credential for `source.kind == "remote"` | `{ "id", "host"? }` → `{ "ok": true, "item": {...} }`. Only `queued` items can be cancelled; a running item is refused with a clear error. |
| `queue.config` | authenticated | `{}` → `{ "maxAgents": 4, ... }` (effective values) |
| `queue.setConfig` | owner | partial object of the keys above → effective config |
| `queue.targets` | authenticated | `{ "kind"?: "spawn" \| "shell", "provider"?, "cwd"? }` → `{ "targets": [ { "host": "local" \| "<peer>", "local": bool, "connected": bool, "dispatch": bool, "resources": {...} \| null, "providers": [{ "provider", "found", "authenticated" }], "cwds": ["/abs/..."], "eligible": bool, "score": number, "reasons": ["..."] } ], "best": "local" \| "<peer>" \| null }` |

Event `queue.changed`: `{ "item": {...} }` on every state change, so clients can
update without polling.

`host` semantics on submit: empty or `local` enqueues here. `<peer>` forwards
to `hub:<peer>/queue.submit` with `host: "local"` and
`source.kind = "remote"`, after checking the peer is configured, connected and
dispatch-enabled. `any` runs placement (same scoring as `queue.targets`) and
then behaves as local or `<peer>`.

Placement scoring (deterministic and explainable through `reasons`):
- Ineligible: not connected, not dispatch-enabled (peers), provider not
  installed or logged in (spawn), cwd not among the host's directory choices
  (peers; local always accepts an absolute existing cwd).
- Among eligible hosts: gate open beats closed. Then more free slots of the
  item's kind, then lower `queued`, then lower memory pressure, then local
  beats remote on a tie.

## Federation

`FORWARD_TOPICS` gains `host.*` and `queue.*`. Forwarded events carry the peer
name in `hub` as today. Clients may cache peer resources from those events,
but placement still asks `hub:<peer>/host.resources` (5 s budget) when its
cached copy is older than 30 s.

## Jobs integration

A job spec gains two optional fields:

- `host`: `""` (default: run inline as today) | `"local"` | `"<peer>"` | `"any"`.
- `priority`: integer, default 0. Used only when `host` is set.

When `host` is set, a due run (spawn or shell action) submits a queue item
instead of executing inline. The run's history row is `ok` with detail
`queued <itemId> on <host>`, or `error` if submission failed. Context steps
still run locally first, and their output is rendered into the prompt before
submission. If an item this job submitted is still `queued` or `running` on
the target, the due run records `skipped` (the overlap rule now covers the
whole item lifetime). `call` actions do not take `host`, and still reject
`hub:` and `jobs.*`.

## Clients

Native (`apps/native`):
- **Hosts view**: one card per host (local and peers) with CPU, memory and IO
  pressure, available memory, free disk, slot usage, gate state and reasons,
  and that host's queue (cancel for queued items). Owner can edit the local
  limits.
- **Launch**: a host picker (This machine / each dispatch-enabled peer /
  Best fit) and, for non-local or "queue" launches, submission through
  `queue.submit` with `kind: "spawn"`. The cwd choices come from the selected
  target's `cwds`.
- **Jobs**: show a job's `host` and `priority`. Approving or editing preserves
  them, which is already true because rows round-trip unknown fields.
