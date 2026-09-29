---
title: Broker admission regressions should inspect queue capacity before delivery
date: 2026-09-29
promoted: false
---

# Broker admission regressions should inspect queue capacity before delivery

## Observation
The Rust actor has no Go Dropped/DroppedTotal diagnostics API; those old accessors had only test consumers. Equivalent negative authority evidence must inspect the actual bounded event queue capacity and desync map before draining, then prove an allowed event arrives. New broker fixtures also retain512topic capacity, reject100k frames before membership work, bound repair topics to64, and show an undrained subscriber cannot stall another consumer.
