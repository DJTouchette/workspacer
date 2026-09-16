---
title: Intent removal preserves Fleet request admission and legacy storage
date: 2026-09-15
promoted: false
---

# Intent removal preserves Fleet request admission and legacy storage

## Observation
The first-class Intent workspace feature began in b88170d2 and used a separate intent-workspaces.sqlite plus artifact files. Removing its services leaves that storage inert: no migration or deletion is needed. Fleet request resolution predates it: managerRequestService maps request intents to task IDs, dispatchHistoryStore validates requestId/intentKey/cwd admission, and TaskInspector renders those provenance keys. Preserve managerRequests, dispatch history, workflows, and task references independently of the removed workspace lifecycle.
