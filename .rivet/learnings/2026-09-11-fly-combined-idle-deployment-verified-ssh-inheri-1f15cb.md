---
title: Fly combined idle deployment verified; SSH inherits the provider token unless explicitly cleared
date: 2026-09-11
confidence: high
suggested_doc: fly-node-deploy
related_paths:
  - deploy/fly/combined/README.md
  - deploy/fly/combined/verify-upgrade.py
  - services/hub/internal/bus/activity.go
  - services/hub/cmd/workspacer/fleetcmd.go
promoted: false
---

# Fly combined idle deployment verified; SSH inherits the provider token unless explicitly cleared

## Observation
Deployed power-observe-20260911-v2 (digest 3e1bfbb59c65bcc915a00dd524aa0e678c1a4bca1d73fa2fe61c47e0acc2e3e7) to existing workspacer-node 1857645df24448, preserving vol_r1j3gge056epwxzr and 4 CPU/8 GB. Public doorbell HTTPS started the stopped machine and returned200; public /m stays404, private Tailscale /m returns200. Hub/worker healthy; cloud credential UID10002 mode0600 unreadable by workerUID10001. Idle modeobserve running with900sec dwell. Fly SSH inherits old HUB_TOKEN provider secret, so workspacer fleet idle must use env -u HUB_TOKEN XDG_CONFIG_HOME=/data/hub/home/.config to resolve persisted hub host token instead.

## Recommendation
Keep observation mode until real blocker behavior is accepted. Reload updated clients to get explicit input reporting; older clients conservatively count polling. Use verify-upgrade.py for checks without exposing credentials. Do not replace the isolated supervisor or hub state home.
