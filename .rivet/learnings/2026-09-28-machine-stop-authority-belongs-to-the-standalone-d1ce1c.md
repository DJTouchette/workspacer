---
title: Machine stop authority belongs to the standalone owner or explicit embedding callback
date: 2026-09-28
confidence: high
suggested_doc: workspacer-serve-cli
related_paths:
  - services/hub-rs/src/services/machine_power.rs
  - services/hub/cmd/hub/machinepower.go
promoted: false
---

# Machine stop authority belongs to the standalone owner or explicit embedding callback

## Observation
Go machine.power/machine.stop implement Fly self-stop only: explicit power=fly and wake=http opt-ins plus configured app/id/token, operator trust, provider preflight, one-second reply delay, interactive websocket close4001, then fixed SIGTERM45s provider stop. Automatic idle mode must strengthen every scheduled job into a blocker and re-read fleet evidence before disconnect. Stop failures latch an error and are not automatically replayed.

## Recommendation
Keep native/library Options.machine_power_provider absent unless the embedding owner explicitly supplies an adapter. Standalone may opt in from environment. Test cloud HTTP only against fixtures and host actions only through fake adapters; never infer laptop suspend authority from platform detection or install process signals in the library.
