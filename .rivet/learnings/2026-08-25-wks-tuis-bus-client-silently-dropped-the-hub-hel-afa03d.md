---
title: wks-tui's bus client silently dropped the hub `hello` frame, so no client-side tier gating was possible
date: 2026-08-25
confidence: high
suggested_doc: tui-client
related_paths:
  - apps/tui/src/bus.rs
  - apps/tui/src/app/nodes.rs
  - services/hub/internal/bus/bus.go
promoted: false
---

# wks-tui's bus client silently dropped the hub `hello` frame, so no client-side tier gating was possible

## Observation
`handle_frame` in `apps/tui/src/bus.rs` matched `result`/`error`/`event` and swallowed everything else under `_ => {} // hello / subscribed / unsubscribed acks`. The hub's `hello` frame is the ONLY place a client learns the tier it authenticated as (`{"op":"hello","scope":"operator"|<tier>,"methods":[...]}`, from `conn.helloFrame()` in services/hub/internal/bus/bus.go) — a trusted host/operator token reports "operator", a scoped token its own tier, a plugin token nothing at all. Dropping it meant the TUI could not gate any host-authority-only capability and would have offered controls that die on press. Fixed by republishing it on a synthetic topic `TOPIC_BUS_HELLO` ("_bus.hello"), the same shape as the pre-existing `TOPIC_BUS_CONNECTED`; `App::bus_scope` reads it in `apply_bus_event`. Note the borrowck wrinkle: the match scrutinee borrows `v`, so the hello arm must clone rather than move it.</observation>
<parameter name="impact">This is what `/app`'s hubBusClient already did ("hubBusClient now reads the hello frame so /app knows its tier"), and the TUI was the last client without it. Any future TUI feature gated on operator/host authority (nodes.wake, jobs.*, anything trusted-only) now has a real answer instead of having to attempt-and-fail. Absent scope reads as NOT operator, which is the safe default for anything that spends money.

## Recommendation
Read the tier from `App::bus_scope` / `App::can_wake_nodes` rather than adding a second discovery path. Do not infer authority from the presence of a token — a scoped token is still a token.
