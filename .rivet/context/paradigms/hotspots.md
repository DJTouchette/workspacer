---
tags: [risk, refactoring, hotspots, dependencies]
related_paths:
  - "apps/desktop/src/renderer/src/hooks/useConfig.ts"
  - "apps/desktop/src/main/services/claudeSessionStore.ts"
  - "apps/desktop/src/renderer/src/types/claudeSession.ts"
  - "apps/desktop/src/main/services/configService.ts"
  - "apps/desktop/src/renderer/src/types/pane.ts"
  - "apps/desktop/src/main/shared/ipcChannels.ts"
  - "apps/desktop/src/main/shared/ipcTypes.ts"
  - "services/claudemon/src/session/state.rs"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Hotspots and change risk

## Get current evidence

Hotspot scores depend on the indexed checkout and recent history. A frozen
fan-in/churn table is not a current risk assessment. From the repository root:

```bash
rivet recon refresh
rivet recon hotspots --max 10 --human
rivet recon related apps/desktop/src/main/services/claudeSessionStore.ts --human
rivet recon context apps/desktop/src/main/services/claudeSessionStore.ts --human
```

Use the file relevant to the task for the last two commands. `related` and
`context` expect files, not directories. Check unresolved imports, parse status,
and analysis caveats before interpreting fan-in as complete. A root such as
`App.tsx` can have low fan-in and still coordinate many critical behaviors.

## Clusters to inspect together

- **Configuration:** the TypeScript `configService` and Go brain writer both
  persist configuration. They refresh and merge under the cross-process lock
  pinned by `contracts/config-lock.json`; mtime refresh alone does not prevent
  competing writes. Renderer minimal patches protect unrelated settings.
  Shared TS services also run in the headless Node companion. See
  [configuration](../domains/config.md).
- **Sessions:** Rust `SessionState`, the desktop store, and renderer snapshots
  are related but distinct models. Changes affect pending approvals, model and
  context provenance, aliveness, managed transports, and federation. A stopped
  daemon row can remain resumable even when omitted from a live client view.
  See [session lifecycle](../domains/session-lifecycle.md).
- **Pane and IPC boundary:** `Record<PaneType, ...>` icon/title maps are
  exhaustive; the rendering switch has a runtime fallback and is not a
  compiler-enforced exhaustiveness check. Menus are another registration step.
  Keep channel constants, payloads, handlers, preload, and backend mappings in
  agreement. See [pane system](../modules/pane-system.md) and
  [IPC boundary](../modules/ipc-boundary.md).
- **Bus authority:** router, capspec, token store, desktop/brain registrations,
  and facade tool schemas jointly determine behavior. Ambient agent/plugin
  access does not remove host-owner administration gates or remote view/triage
  policy. See [the control plane](../modules/hub-bus-control-plane.md).
- **Federation:** session IDs and cwd values must retain their owning hub.
  Per-session methods that bypass qualification can act on the wrong machine.
  Check both desktop federation handling and the web backend.
- **Renderer orchestration:** inspect the actual callbacks, hooks, and state
  owners in `App.tsx` for the change at hand. Historical line counts or an old
  decomposition review do not establish that current code is irreducible.

## Validation scope

Use `rivet witness select` to help find tests, then verify its selection against
the behavior being changed. No selected tests does not prove coverage. Include
cross-language contract and backend-parity tests when touching shared shapes,
and the headless companion suite when shared services change.

`make test` runs the component test suites. It does not substitute for formatting,
typechecking, linting, generation checks, or browser flows. Consult
`.github/workflows/ci.yml`, the component manifests, and the Makefile for the
current gates rather than copying an old command list. In particular, the
Makefile's hub test target uses `-race -count=1` because cross-repository test
inputs can be missed by Go's ordinary cache key.

Documentation-only changes use `rivet context lint` and
`python3 scripts/check-rivet-docs.py`, plus focused implementation checks for
behavioral claims. Neither checker establishes semantic accuracy by itself.
