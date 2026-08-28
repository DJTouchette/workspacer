---
title: webBackend.ts onFacadeOpenTerminal is currently a hard no-op stub, KNOWN_STUBS-listed
date: 2026-08-24
confidence: high
related_paths:
  - apps/desktop/src/renderer/src/backend/webBackend.ts
  - apps/desktop/src/renderer/tests/backend/backendParity.test.ts
  - apps/desktop/src/renderer/src/App.tsx
  - apps/desktop/src/renderer/src/hooks/useAgentManager.ts
  - services/hub/cmd/brain/visibleterm.go
  - services/hub/internal/capspec/eventtopics.go
promoted: false
---

# webBackend.ts onFacadeOpenTerminal is currently a hard no-op stub, KNOWN_STUBS-listed

## Observation
apps/desktop/src/renderer/src/backend/webBackend.ts:792 `onFacadeOpenTerminal: () => () => {}` is a literal no-op — it never calls client.subscribe. It's listed in KNOWN_STUBS in apps/desktop/src/renderer/tests/backend/backendParity.test.ts:172 with comment "facade-opened terminals are a desktop-pane affordance; the browser mirror has no PTY pane → no-op". But the web renderer (/app) DOES have a real 'terminal' pane type (App.tsx uses openManagedTerminal from useAgentManager.ts:1452 to open one), so the stub comment's premise is stale for /app specifically (may still be true for /m mobile which has no terminal pane). The Go brain (services/hub/cmd/brain/visibleterm.go) already publishes `facade.openTerminal` on the bus (r.publish at line 74) as its ONLY delivery path since it has no renderer to emitToRenderer to — this is the terminals.open capability's headless implementation. The topic is TopicGuardedBy terminals.open in services/hub/internal/capspec/eventtopics.go:127-141 (payload is a host command line, same disclosure level as composing one — view/triage tokens can't receive it).

## Impact
Implementing "subscribe /app to facade.openTerminal" means: (1) webBackend.ts:792 gets a real client.subscribe('facade.openTerminal', ...) body mirroring the watchFile/fs.changed pattern at webBackend.ts:552-563 (client.subscribe returns unsub, reshape ev.data into the {cwd,command,label,parentSessionId} callback shape already typed in electron.d.ts:383); (2) move 'onFacadeOpenTerminal' out of KNOWN_STUBS into BUS_BACKED in backendParity.test.ts (since it's an event subscription now, not a stub) and update/remove the now-stale comment; (3) App.tsx:1657's existing useEffect(onFacadeOpenTerminal → openManagedTerminal) needs zero changes — it already calls window.electronAPI.onFacadeOpenTerminal and will pick up the real implementation automatically.

## Recommendation
When wiring the subscription, follow the watchFile pattern (webBackend.ts:552-563), not onLayoutChanged's (webBackend.ts:786-789) — watchFile is the better template because it shows unsub cleanup + payload reshaping from ev.data, whereas onLayoutChanged has no reshaping. No client.call needed since facade.openTerminal is push-only (like onLayoutChanged, unlike watchFile which also calls fs.watch to arm it).
