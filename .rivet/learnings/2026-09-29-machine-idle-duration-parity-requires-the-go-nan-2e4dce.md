---
title: Machine idle duration parity requires the Go nanosecond bound
date: 2026-09-29
promoted: false
---

# Machine idle duration parity requires the Go nanosecond bound

## Observation
Read-only audit of cmd/hub/machinepower.go shows machineIdleTimeout relies on time.ParseDuration, whose signed64bit unit is nanoseconds. Rust quiescence/power.rs duration_ms accumulates milliseconds and checks the i64 millisecond bound, accepting values such as2562048h that Go rejects. This changes idle mode reporting from off to observe/stop even though practical stop would be far future. Existing machinepower eight-case guards map to Rust controller/Fly/corpus tests, but parser overflow needs a focused correction before per-file evidence certification. No cloud operations are involved.
