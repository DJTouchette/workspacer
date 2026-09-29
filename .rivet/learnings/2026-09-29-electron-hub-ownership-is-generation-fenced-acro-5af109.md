---
title: Electron hub ownership is generation fenced across health and restart
date: 2026-09-29
promoted: false
---

# Electron hub ownership is generation fenced across health and restart

## Observation
The Rust-only hubDaemon launcher must fence the initial adoption probe as well as owned health and child exit callbacks. stopHub increments generation, aborts outstanding probes, cancels restart timers, and new starts wait for owned shutdown before probing; otherwise a late healthy response can adopt a stopped process or a stale exit can clear a newer child. AbortSignal is optional on daemonUtils.probeHealth, preserving existing callers. Seven Electron suites (279 tests) and main TypeScript typecheck passed, including stop-start races and IPv6 wildcard loopback dialing.
