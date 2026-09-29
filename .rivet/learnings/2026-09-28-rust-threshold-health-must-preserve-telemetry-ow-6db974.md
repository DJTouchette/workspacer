---
title: Rust threshold health must preserve telemetry ownership and raw freshness
date: 2026-09-28
promoted: false
---

# Rust threshold health must preserve telemetry ownership and raw freshness

## Observation
The Go fleetview contract treats a present raw status_line as authoritative for context health even when it lacks context_health; falling back to the camelCase compatibility copy can resurrect stale evidence. Runtime context requires a fresh provider-matching runtime pair and exact decimal epoch; numeric epochs above JavaScript safe integer range are refused. Cumulative token totals never imply active context occupancy. Rust services/thresholds.rs preserves these rules and reads the shared context-health corpus. Internal scheduler/catalog broker connections use a private service connection mode so idle detection excludes host housekeeping without letting wire clients claim that exemption.
