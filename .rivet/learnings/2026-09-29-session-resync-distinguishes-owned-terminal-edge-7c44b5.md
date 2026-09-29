---
title: Session resync distinguishes owned terminal edges from retained history
date: 2026-09-29
promoted: false
---

# Session resync distinguishes owned terminal edges from retained history

## Observation
Go reconcileSessionStore requests include_archived/include_empty and skips unknown stopped sessions. Rust typed broadcast lag previously called its ordinary seed path, risking unrelated historical rows or losing known empty terminal states. Resync now requests full inventory and retains ended rows only if projection or durable launch journal already knows their ID; the journal matters for very short host-owned launches whose start and finish events both fell out of the receiver. Initial seed behavior stays unchanged. Added real embedded engine plus controlled typed Lagged receiver regression and public arm/sweep detached-response race regression; explicit Cargo tests needed because Witness does not map these new Rust files.
