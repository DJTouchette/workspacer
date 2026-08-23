---
title: services/hub/cmd/workspacer/* (headless serve CLI) has no context doc and duplicates supervision logic
date: 2026-07-11
confidence: high
related_paths:
  - services/hub/cmd/workspacer/*.go
  - apps/desktop/src/main/services/configService.ts
  - apps/desktop/src/main/services/remoteServer.ts
promoted: true
---

# services/hub/cmd/workspacer/* (headless serve CLI) has no context doc and duplicates supervision logic

## Observation
The `workspacer` binary (services/hub/cmd/workspacer/{main,serve,plan,child,backoff,status,token,tokencmd,install,resolve}.go) is the headless-mode launcher: `workspacer serve` spawns+supervises claudemon and hub (hub run with --brain-scope full), wires their ports/token, and prints a ready banner. It has its OWN restart-backoff/child-process supervision (backoff.go, child.go) separate from the hub's internal plugin/sidecar supervisor (likely covered by the hub-process-supervision doc) — the two are easy to conflate but are different code paths for different processes (this CLI supervises claudemon+hub; the hub supervises its own plugins/brain). It also mints/persists the bus pairing token at <config>/remote-token via loadOrCreateToken() in token.go, which is deliberately the SAME file path the desktop app's configService.ts writes (authtoken.ConfigDir() is the single shared definition) — so a phone paired against the desktop app keeps working against `workspacer serve` and vice versa without re-pairing. None of agent-spawn, hub-process-supervision, hub-bus-control-plane, or remote-mobile appear to document this launcher binary or the shared-token detail.

## Impact
Anyone changing hub-process-supervision assuming it covers `workspacer serve`'s child supervision will miss the parallel backoff/child logic in cmd/workspacer. Anyone changing the desktop's remote-token file location/format without checking cmd/workspacer/token.go will silently break CLI/desktop token interop.

## Recommendation
Add a dedicated context doc for the headless serve CLI (services/hub/cmd/workspacer) rather than assuming hub-process-supervision covers it.

## Disposition
Resolved: the recommended doc now exists — .rivet/context/modules/workspacer-serve-cli.md covers the twin supervisor and the shared remote-token invariant.
