---
title: PTY model acceptance boundary
date: 2026-08-31
promoted: false
---

# PTY model acceptance boundary

## Observation
A durable live Claude PTY model switch must normalize the structural pair first, enqueue the legacy /model spelling only inside claudemon, then dual-write memory and SQLite after queue acceptance. The accepted switch must clear stale model/context status-line fields while preserving cost and rate-limit telemetry; otherwise the prior model frame can override the new canonical request.
