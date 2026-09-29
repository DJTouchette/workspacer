---
title: Spawn key aliases must use simple lowercase rather than Rust contextual lowercase
date: 2026-09-29
promoted: false
---

# Spawn key aliases must use simple lowercase rather than Rust contextual lowercase

## Observation
The legacy Go guard compares strings.ToLower results. Rust str::to_lowercase applies contextual final sigma and expands dotted I, while an ASCII-only known-key comparison missed Kelvin-sign aliases such as tasKId. Admission now uses per-character simple lowercase and checks known canonical keys against that result. The same sanitize function protects local and federated spawn/dispatchPrepare; tests cover Kelvin,dotted-I,Greek duplicates and preservation of canonical/unknown fields.
