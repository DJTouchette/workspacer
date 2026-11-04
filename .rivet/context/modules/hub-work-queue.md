---
title: "Hub Work Queue: resource-gated agent/shell items, placement across federated hosts"
tags: [hub, rust, work-queue, scheduler, federation, jobs, security-invariant]
related_paths:
  - "docs/host-work-queue.md"
  - "services/hub-rs/src/services/work_queue.rs"
  - "services/hub-rs/src/services/work_queue/model.rs"
  - "services/hub-rs/src/services/work_queue/placement.rs"
  - "services/hub-rs/src/services/work_queue/routing.rs"
  - "services/hub-rs/src/services/work_queue/spawn_watch.rs"
  - "services/hub-rs/src/services/work_queue/project_links.rs"
  - "services/hub-rs/src/services/remote_dispatch/readiness.rs"
  - "services/hub-rs/src/services/work_queue/tests.rs"
  - "services/hub-rs/src/services/host_resources.rs"
  - "services/hub-rs/src/services/jobs.rs"
  - "services/hub-rs/src/federation.rs"
  - "services/hub-rs/tests/work_queue.rs"
  - "contracts/host-trusted-config-cases.json"
owner: Damien Touchette
last_reviewed: 2026-11-04
---

# Hub work queue

The wire contract is [docs/host-work-queue.md](../../../docs/host-work-queue.md);
change it first. This note is the map for changing the code.

## Ownership

- `work_queue.rs` owns the durable queue (`queue.json`, 0600, atomic rewrite,
  next to `jobs.json`), admission, execution and the bus handlers
  (`host.resources`, `queue.list|submit|cancel|config|setConfig|targets`).
  `WorkQueue::dispatch` routes host-aware methods to the async code and
  everything else to the blocking `call` off the async workers.
- `model.rs`: pure rules — submit parsing, priority-then-FIFO order, per-kind
  slot admission (`plan`), restart recovery, retention, listing order.
- `placement.rs`: pure, deterministic scoring for `queue.targets` and
  `host: "any"` (`evaluate` → rows with `eligible`, `score`, `reasons`, plus
  `best`). Unit-tested there; keep I/O out of it.
- `routing.rs`: everything that talks to peers — `submit_routed`,
  `cancel_routed`, `list_with_peers`, `place`/`targets` (per-peer reads run
  concurrently), the peer `host.resources` cache, and `job_item_active` for
  the job scheduler's overlap rule.
- `host_resources.rs`: the 5 s sampler and the resource gate.
- Runtime wiring (`runtime.rs`): the queue installs **before** jobs (jobs get
  `options.work_queue`), and federation starts after both, so the routes are
  attached later with `attach_federation`. Before that, the queue sees no
  peers.

## Invariants

- Admission is local and re-checked at the destination: peer resources and
  readiness only choose a target, never grant anything.
- Uncertain outcomes are terminal: a `running` item found at start becomes
  `unknown` (never re-run); a forward that timed out or lost its link says
  "the outcome is unknown" and is never retried (`routing::forward_error`).
- One hop: a federated caller can only act on the hub it is connected to.
- Authority (`submit_local`): shell and `spawn.fullAccess` need `is_owner`, or
  `is_host_credential` — a federated, trusted, operator-scope caller whose
  `token_id` equals `auth::fingerprint` of this hub's host token. A scoped
  operator token never qualifies. Spawn needs `may_spawn`. The origin applies
  the same bar to its own caller before forwarding.
- `source.kind == "remote"` and `submitter` are stamped from the caller; the
  submitter fingerprint (never on the wire) is what lets a peer cancel its
  own item.
- `queue.*` config is host-trusted: non-owner `config.save` drops the whole
  section (`config::drop_host_trusted`, desktop `hostTrustedConfig.ts`, shared
  fixture `contracts/host-trusted-config-cases.json`). Only `queue.setConfig`
  (owner) writes it.

## Slot lifetime

Shell: process start to exit (timeout kills). Spawn: from `agents.spawn`
until the agent is idle after working, waits on a person, ends or stops
(`spawn_watch::classify` over `agent.snapshot`, 60 s `sessions.snapshot`
re-check as backstop). Every stored change bumps `updatedAt`.

## Placement and federation

`FORWARD_TOPICS` includes `host.*` and `queue.*`; the queue's run loop caches
forwarded peer `host.resources` by the configured peer name (`event.hub`).
`place` uses the cache when younger than 30 s, else `hub:<peer>/host.resources`
(5 s), and `hub:<peer>/fleet.dispatchCapabilities` (the remote-dispatch
readiness RPC, 10 s) for providers and directory choices. Local readiness
comes from `options.remote_dispatch_execution` in-process. Peers use the
configured name everywhere (`peers[].peer`, `targets[].host`, submit result
`host`, event `hub`).

## Project links

The same project lives at different paths on different machines.
`project_links::resolve` (pure) maps a local cwd onto one peer's advertised
choices: saved link (if still advertised) → exact path → the unique choice
with the same normalized origin → nothing, with `remote`/`name` suggestions.
A name match is never used without a saved link; an ambiguous remote is never
used. `Directory` (remote_dispatch/model.rs) carries `name` and `remote`
(serde-defaulted for older peers); `readiness::with_remotes` probes only
repository roots, concurrently, 3 s each, and `normalize_remote` strips
credentials. Links live in `project-links.json` beside `queue.json` (0600,
atomic; `Links` in project_links.rs, keyed by canonical `localPath` + peer).

Where it applies: `place` fills `Request::cwd_key`/`cwd_remote` (`identify`)
and each peer `Candidate::link`; `placement::target` resolves and reports
`resolvedCwd`/`resolvedVia`/`suggestions`. `submit_peer` rewrites
`spawn.cwd`/`shell.cwd` to the translated path and records
`source.originCwd` (`PeerCwd::Placed` from `any`, `PeerCwd::Resolve` for a
named peer or a job). If a named peer's choices cannot be read, the link or
the cwd as given is forwarded (the peer re-checks). Local items never
translate; `submit_local` clears `originCwd` for a non-federated caller.
RPCs `projects.links|linkMatrix` (reads, view/triage tiers) and
`projects.setLink|removeLink` (owner) are dispatched by the queue;
`projects.linksChanged` is published only on change and not forwarded.

## Jobs

A job with `host` (spawn/shell only) runs context steps locally, renders the
prompt, then `submit_routed_as_owner`; history `queued <id> on <host>`. The
scheduler keeps the last `(host, id)` per job in memory; local items are also
matched by `source.jobId` across restarts; an unreachable peer counts as still
running (skipped). `agents.jobFullAccess` maps to `spawn.fullAccess`.

## Tests

- `cargo test --lib work_queue` (model, placement, routing helpers, auth incl.
  host-credential shell) and `cargo test --lib jobs` (host/priority
  validation, queued spawn/shell submission, overlap skips).
- `cargo test --test work_queue`: local queue behaviour plus two-hub tests
  (shell on a host-token peer runs there; operator-token peer refused; not
  dispatch-enabled / unknown peer refused; `includePeers`; `any` placement;
  remote cancel), project links (same origin at another path for named and
  `any` submits, name-only refusal with a suggestion, a saved link used, the
  link matrix; a peer without choices gets the cwd as given) and a
  `host: "local"` job end to end.
- `cargo test --test config` (host-trusted `queue`), and the registration
  suites: `auth_composition`, `headless_completeness`, `compatibility`,
  `corpus_vocabulary`, `auth`, `quiescence`, `services`.
