---
title: Only Claude-on-PTY has a heartbeat status line; managed providers AND claude/stream are activity-driven
date: 2026-08-23
author: Damien Touchette
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/renderer/src/lib/stallDetector.ts
  - apps/desktop/src/renderer/src/hooks/useAttentionFeed.ts
  - services/hub/cmd/hub/mobile.html
  - services/claudemon/src/providers/mod.rs
  - services/claudemon/src/session/store.rs
  - services/claudemon/src/daemon/init.rs
promoted: true
promoted_to: session-lifecycle
---

# Only Claude-on-PTY has a heartbeat status line; managed providers AND claude/stream are activity-driven

## Observation
There are two entirely different sources for a session's `status_line`, and only one of them ticks on a clock.

(1) HEARTBEAT — Claude Code's own `statusLine` command. The interactive CLI re-runs it on every render; claudemon installs a forwarder into settings.json (`daemon/init.rs` `status_line_command`) that POSTs to `/statusline`, where `SessionStore::ingest_status_line` stamps a fresh `received_at`. Requires the real CLI drawing a terminal, i.e. provider `claude` + transport `pty`.

(2) ACTIVITY-DRIVEN — everything else: codex, opencode, pi, AND claude on the `stream` transport (the shipped default). The line is synthesized by `UsageAcc::status_line()` (`providers/mod.rs`, which always stamps `received_at: Some(now)`) and published only from `apply_updates`'s `if usage_changed { store.apply_status_line(...) }`. `usage_changed` is set by Usage / RateLimits / RateLimitStatus / Capabilities / Effort frames. No timer calls `apply_status_line` anywhere.

Two corollaries that are easy to get wrong:
- `set_account_usage` (store.rs, gated on `e.provider == "claude"`) is NOT the claude heartbeat. It pushes a patched line to live claude sessions but `patch_rate_limits` only stamps `received_at` when it is already `None`, so it never bumps an existing one.
- `AgentUpdate::Busy` is described as "a liveness ping" but is emitted at turn/tool boundaries (`turn/started`, `task_started`, `agent_start`), not periodically — and `apply_updates` debounces it away when the mode is unchanged. It is not a heartbeat either.

Consequence for clients: for an activity-driven session, `statusLine.receivedAt` cannot separate "alive but quiet" from "process gone". It freezes exactly when observable work does. And since `progressFingerprint` (renderer `lib/stallDetector.ts`) counts the same token totals the status line carries, a fingerprint frozen for STALL_MS *implies* a `receivedAt` frozen for STALL_MS — well past the 90s SILENT_MS — so any aliveness check keyed on it returns "dead" 100% of the time it is consulted.

Also: real managed-process death never reaches a stall check at all. The driver task exits, `deregister_managed` marks the row Stopped, and `sweep_ghost_sessions` explicitly skips any session that still holds `managed_inputs` plumbing — so a live driver keeps the row live, and a dead one leaves the working state immediately.</observation>
<parameter name="impact">This shipped as a live bug: the attention feed's stall card rendered "No signal — the agent has stopped reporting at all" for EVERY codex/opencode/pi stall and every claude/stream stall, making the card's "Not moving" half unreachable for the default transport. Fixed 2026-08-23 (commit da5e8710) by replacing `StallVerdict.alive: boolean` with `signal: 'alive' | 'silent' | 'unknown'`, where anything not heartbeat-backed reports `unknown` and the card says so instead of guessing. Two comments (stallDetector.ts, mobile.html) asserted managed providers publish NO status line at all — also false, and they were the reason the wrong fallback looked safe.</observation>
<parameter name="recommendation">Before treating `statusLine.receivedAt` (or `updated_at`, or any single field) as liveness, check which of the two sources feeds it: `(provider ?? 'claude') === 'claude' && transport !== 'stream'` is the only heartbeat-backed case. If you want a genuine per-session liveness signal for managed providers, it has to be built: a periodic tick in claudemon that calls `apply_status_line` (or a dedicated heartbeat channel) for sessions holding `managed_inputs`. Nothing client-side can synthesize it. Desktop `renderer/src/lib/stallDetector.ts` and the PWA's copy in `services/hub/cmd/hub/mobile.html` implement this identically and must be changed together.</recommendation>
<parameter name="confidence">high
