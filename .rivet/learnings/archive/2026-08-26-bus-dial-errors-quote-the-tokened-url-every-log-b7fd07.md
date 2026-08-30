---
title: Bus dial errors quote the tokened URL — every log site needs internal/redact
date: 2026-08-26
confidence: high
suggested_doc: hub-bus-control-plane
related_paths:
  - services/hub/internal/redact/*.go
  - services/hub/cmd/brain/bus.go
  - services/hub/internal/busclient/client.go
  - services/hub/internal/federation/federation.go
promoted: true
promoted_to: hub-bus-control-plane
---

# Bus dial errors quote the tokened URL — every log site needs internal/redact

## Observation
coder/websocket's dial error embeds the full request URL verbatim ("failed to send handshake request: Get \"https://host/bus?token=<32-char token>\": context deadline exceeded"). Every bus dial in services/hub carries its credential as a ?token= query param (a browser WS handshake can't set an Authorization header, so internal/bus treats the query form as canonical), so brain's per-reconnect log line `brain: bus disconnected (%v)` published the node's whole HUB_TOKEN once per attempt — observed live in Fly's log pipeline, dozens of lines in one 2-minute hub outage. Fixed by services/hub/internal/redact (Text/URL/Error), applied at cmd/brain/bus.go session() + run(), cmd/brain/main.go banner, internal/federation connect/disconnect, cmd/workspacer/plugindev.go, cmd/mcp/main.go.</observation>
<parameter name="impact">Any NEW log line that formats a bus URL or an error returned from a dial re-opens the leak; internal/busclient still stores the tokened URL in Client.url and today only survives because it drops dial errors silently. `workspacer serve`'s ready banner prints the token by design (the headless terminal is the pairing surface) — on a Fly node that stdout IS the log pipeline, so a token still lands there once at startup.</impact>
<parameter name="recommendation">Wrap any error or URL that could come from a bus/HTTP dial in redact.Error / redact.URL before logging it. If internal/busclient ever starts logging its dial failures, redact there too (or give Client a SafeURL()).
