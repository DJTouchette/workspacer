---
title: Ordinary agent wakes need direct-parent routing
date: 2026-09-16
promoted: false
---

# Ordinary agent wakes need direct-parent routing

## Observation
Do not mark ordinary agents isWakeTarget: derive parentSessionId from the session facade token, route finish/catch-up/blocker events only to the live direct parent, and reserve isWakeTarget for fleet-wide block broadcasts, task ownership, and manager succession. App-owned skills use native project roots for Claude/Codex, instruction pointers for Copilot/OpenCode, and skip Pi because it has no MCP bridge.
