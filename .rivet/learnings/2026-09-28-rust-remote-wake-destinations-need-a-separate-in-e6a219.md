---
title: Rust remote wake destinations need a separate internal route identity
date: 2026-09-28
promoted: false
---

# Rust remote wake destinations need a separate internal route identity

## Observation
Remote worker parentSessionId belongs to the origin hub and can collide with a live local manager ID. Rust Wakes queues local and remote actions separately and dispatches known remote workers only through the Receiver-owned return channel; it never treats a local row with the same string as proof of the origin manager. Block alerts still fan out to local managers as in Go. Progress consults the durable receiver session mapping before checking local parents, preserves per-worker rate limits, and reports queuedTo rather than claiming remote delivery. Receiver owns terminal sequencing/replay; no caller payload can select return-channel ownership.
