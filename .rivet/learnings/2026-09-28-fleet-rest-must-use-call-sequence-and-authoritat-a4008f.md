---
title: Fleet rest must use call sequence and authoritative empty evidence
date: 2026-09-28
confidence: high
suggested_doc: hub-jobs
related_paths:
  - services/hub-rs/src/services/quiescence.rs
  - contracts/fleet-quiescence-cases.json
promoted: false
---

# Fleet rest must use call sequence and authoritative empty evidence

## Observation
Go quiescence distinguishes raw daemon mode from desktop ambient state, skips advisory shell jobs, and requires continuous sampled dwell. Input-reporting peers use last interaction instead of passive polling time. Rust preserves these rules and additionally uses broker activity sequence to prevent same-millisecond later input from inheriting a quiescence query exemption. Missing sessions, broker clients, schedules or peers are blockers, never empty evidence.

## Recommendation
Keep the portable fleet-quiescence corpus loaded by both Go and Rust. Preserve trusted internal-client tagging and do not derive it from wire fields. Read-only machine.power must keep canStop false until the separate guarded power action is implemented.
