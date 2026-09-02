---
title: Fleet Manager lacks a manager-scoped context setting
date: 2026-08-31
promoted: true
promoted_to: fleet-manager
---

# Fleet Manager lacks a manager-scoped context setting

## Observation
At master 5fb2bd3f, agents.managerModels and agents.managerEfforts are per-provider and resolved in the desktop spawn funnels, but there is no manager context map or resolver. Fresh Codex managers get 1M only through provider-level new-spawn defaulting, while resume preserves the prior request. A manager context setting must be wired through these same funnels and their headless counterpart.

## Disposition
Promoted into `.rivet/context/modules/fleet-manager.md` AS A CORRECTION — the claim is no longer true. `agents.managerContextWindows` shipped in `1eebab86`, is resolved beside `managerModels`/`managerEfforts` in `main/lib/roleModels.ts`, survives `configService`'s presence-aware merge, and is pinned with its Hub twin by `managerPreferenceCases` in `contracts/model-context-windows.json`.
