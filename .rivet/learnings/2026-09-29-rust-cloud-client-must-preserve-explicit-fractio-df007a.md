---
title: Rust cloud client must preserve explicit fractional drain timeouts
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/nodes/cloud.rs
promoted: false
---

# Rust cloud client must preserve explicit fractional drain timeouts

## Observation
Fly API parity review found Http::stop accepted any positive Duration but serialized as_secs, turning500ms into0s and1500ms into1s. The client now emits precise duration strings and normalizes zero wait timeout to the legacy60s default. Real loopback HTTP tests verify request methods, escaped host coordinates, bearer-header-only auth, cancellation/rate lanes, unknown state classification, bounded responses and typed redacted errors. No real cloud operation ran.
