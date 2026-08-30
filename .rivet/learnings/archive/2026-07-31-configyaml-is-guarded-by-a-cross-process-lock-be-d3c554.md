---
title: config.yaml is guarded by a cross-process lock because neither process can own it
date: 2026-07-31
promoted: true
---

# config.yaml is guarded by a cross-process lock because neither process can own it

## Observation
2026-07-31. config.yaml has two writers (desktop configService.ts in-process for the Settings pane, Go brain for config.save over the bus which is what the web/mobile Settings panes use). Both do refresh-if-changed -> deepMerge -> atomic write; the mtime gate each has closes only the REFRESH, nothing spans the three, so an interleaved write from the other process was silently lost with both reporting success. Single ownership was investigated and rejected on evidence: the desktop cannot own it because headless 'workspacer serve' runs with no Electron (serve.go), and the brain cannot own it because serve.go explicitly warns and serves brain-less when no binary is found - so a brain-owner design silently breaks settings on a brain-less install. Section-level ownership does not split them either; ui.theme and keybindings are written by both Settings panes, and the one section with a real owner (updates) is already host-trusted. Fix: an O_EXCL lockfile at config.yaml.lock held across all three steps by both twins (apps/desktop/src/main/lib/configLock.ts, services/hub/cmd/brain/configlock.go). A lock that cannot be taken is a REFUSED save, never a write-anyway. A lock older than staleMs is stolen so a dead holder cannot wedge config. contracts/config-lock.json pins staleMs and the filename - those are the correctness parameters, because a side expiring locks sooner steals one the other still holds and then BOTH write believing they are exclusive. Wait budgets are deliberately per-side and NOT shared: desktop saveConfig is synchronous on the Electron main thread so it waits 250ms and gives up, the brain waits 2s. Gotcha: configService.test.ts mocks 'fs' as an allowlist of primitives, so adding openSync/writeSync/closeSync to the mock was required - a new fs primitive in the config write path always needs a matching mock entry there.

## Disposition
Folded into .rivet/context/domains/config.md (cross-process lock note); also corrected the stale 'file is not locked' line in paradigms/architecture-overview.md.
