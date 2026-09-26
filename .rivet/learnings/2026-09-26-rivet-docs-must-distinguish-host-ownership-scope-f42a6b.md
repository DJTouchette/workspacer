---
title: Rivet docs must distinguish host ownership, scoped operators, and facade forwarding
date: 2026-09-26
confidence: high
suggested_doc: hub-jobs
related_paths:
  - services/hub/cmd/hub/main.go
  - services/hub/cmd/hub/peersconfig.go
  - services/hub/cmd/mcp/main.go
promoted: false
---

# Rivet docs must distinguish host ownership, scoped operators, and facade forwarding

## Observation
The current jobsTrusted gate requires AuthenticatedHost, Trusted, and operator scope, while older context said IsTrusted alone. MCP operator catalogs forward on the facade bus connection, so catalog scope and hub owner credentials are distinct. Peer-config bus saves hot-replace links via Controller.Replace, while the legacy desktop IPC writer still restarts an app-owned hub.

## Recommendation
Check executable guards and forwarding credentials, not historical source comments or tool visibility, when documenting administration rights.
