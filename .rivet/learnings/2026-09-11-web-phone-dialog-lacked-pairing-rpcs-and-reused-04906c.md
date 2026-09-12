---
title: Web phone dialog lacked pairing RPCs and reused the current URL for both clients
date: 2026-09-11
confidence: high
suggested_doc: remote-mobile
related_paths:
  - services/hub/cmd/hub/remotepairing.go
  - apps/desktop/src/renderer/src/backend/webBackend.ts
  - apps/desktop/src/renderer/src/components/RemoteShareDialog.tsx
promoted: false
---

# Web phone dialog lacked pairing RPCs and reused the current URL for both clients

## Observation
createWebBackend.getRemoteInfo returned location.href as both remoteUrl and appUrl, omitted remoteTokenGetOrCreate/list/revoke, and exposed setRemoteShare as a no-op. RemoteShareDialog hides its scope picker when token management is absent, causing web/desktop pairing mismatch. Browser fallback also labelled every current token operator. Added hub-native owner-gated pairing methods, explicit caller scope/capability projection, correct /m and /app/ URLs, and hidden unsupported sharing toggle. Pairing methods exclude infrastructure/session records and reject scoped-operator administration to avoid widening their recorded grants.

## Recommendation
Use actual scope and capability info to render web admin controls. Do not equate method presence with authorization or expose provider/MCP credentials through phone-pairing UI.
