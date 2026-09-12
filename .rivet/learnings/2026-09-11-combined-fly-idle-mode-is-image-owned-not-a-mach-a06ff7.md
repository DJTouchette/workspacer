---
title: Combined Fly idle mode is image-owned, not a machine environment override
date: 2026-09-11
confidence: high
suggested_doc: fly-node-deploy
related_paths:
  - deploy/fly/combined/*
  - services/hub/cmd/hub/machinepower.go
promoted: false
---

# Combined Fly idle mode is image-owned, not a machine environment override

## Observation
The isolated supervisor reads /opt/combined/power.json and sets WKS_MACHINE_IDLE_MODE on the hub itself after env -i. A Fly machine environment override would not change it. A policy-only image layered on mobile-spawn-20260911-v2 changed mode to stop, preserving binaries, identity checks, timeout and credential isolation. Live machine.power reported stop/900 seconds after deployment.
