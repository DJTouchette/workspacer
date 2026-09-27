---
title: Headless shutdown revocation relies on next-start stopped-session reconciliation
date: 2026-09-27
promoted: false
---

# Headless shutdown revocation relies on next-start stopped-session reconciliation

## Observation
Both existing workspacer serve and native local shutdown stop brain/facade/hub before claudemon. Brain has no process-exit bulk token revocation; session facade tokens persist without TTL and onEndedRetry revokes them on stopped observations. store.seed retries revocation for stopped rows at next startup, after claudemon boot hydration marks persisted sessions stopped. Abandoned database or absent rows can retain old records; this is inherited behavior, not an embedding regression, and reordering alone cannot guarantee acknowledged revocation.
