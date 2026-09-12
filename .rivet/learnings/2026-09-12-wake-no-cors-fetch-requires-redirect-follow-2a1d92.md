---
title: Wake no-cors fetch requires redirect follow
date: 2026-09-12
suggested_doc: remote-mobile
related_paths:
  - services/hub/cmd/hub/mobile.html
  - apps/desktop/src/renderer/src/backend/machinePower.ts
promoted: false
---

# Wake no-cors fetch requires redirect follow

## Observation
The /m and /app Wake buttons used mode:no-cors with redirect:error. Chromium rejects this combination before sending any network request (Fetch API cannot load: Request mode is no-cors but redirect mode is not follow), even when Fly /health returns 200. Use redirect:follow with credentials:omit and validated credential-free HTTPS URL; real-browser paused-to-connected regression tests cover both clients. Bump mobile service-worker cache for the embedded shell update.
