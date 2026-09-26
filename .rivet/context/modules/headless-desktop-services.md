---
title: Shared desktop services in the headless brain
tags: [headless, desktop-host, brain, node, services, parity, authentication, stdio, bundle]
related_paths:
  - "contracts/desktop-service-methods.json"
  - "apps/desktop/scripts/gen-desktop-services.mjs"
  - "apps/desktop/scripts/build-desktop-host.mjs"
  - "apps/desktop/src/main/headless/*.ts"
  - "apps/desktop/src/main/services/nativeDesktopServices.ts"
  - "apps/desktop/src/renderer/src/backend/desktopServices.ts"
  - "services/hub/cmd/brain/desktophost*.go"
  - "services/hub/internal/bus/desktop.go"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Shared desktop services in the headless brain

## Purpose and ownership

The headless brain can execute shared TypeScript services in a private Node
child, packaged as `desktop-host.cjs`. This reuses desktop configuration,
worktree, pricing, workflow, review, dispatch-history, and other service logic
without starting Electron. The Go brain remains the bus provider and supplies
daemon/session context to the child. This companion is distinct from the hub,
MCP facade, and plugin sidecars.

`services/hub/cmd/brain/desktophost.go` locates the bundle beside the brain
executable, unless `WKS_DESKTOP_HOST` overrides it. It starts `node <bundle>`
on demand and reuses that process for subsequent calls. `available()` checks
for a regular bundle file; it does not prove Node is installed or that startup
will succeed. Node runs as the brain's OS user and is not a sandbox boundary.

## Public registry and authority

`contracts/desktop-service-methods.json` is the method-list source of truth.
`apps/desktop/scripts/gen-desktop-services.mjs` validates it and updates the
shared TypeScript registry, Go capspec lists, and literal per-method cases in
the bus owner gate. Do not hand-edit those generated sections. The generator
formats TypeScript through the repository's Prettier configuration.

Public `desktop.*` calls require an authenticated, trusted, non-revoked host
connection. A scoped operator bearer by itself is insufficient. The manifest
separates owner methods from `ui.fonts`/`ui.asset`; do not infer identical
authority merely because both are included in generated service lists.

The renderer adapter in `apps/desktop/src/renderer/src/backend/desktopServices.ts`
maps shared `ElectronAPI` methods onto bus calls. The native implementation is
`apps/desktop/src/main/services/nativeDesktopServices.ts`. Adding a method
requires its registry entry, intended native/headless implementations, renderer
mapping, and authority/parity coverage.

## Private protocol and host context

`apps/desktop/src/main/headless/stdio.ts` reads newline-delimited JSON requests
with `id`, `method`, `params`, and `context`. Replies carry the same ID and
either `result` or `error`. Events are separate `{event,data}` frames; private
lifecycle callbacks use `hostCallId` and `hostResultId`.

Stdout belongs exclusively to this protocol. Both the bundle banner and the
entry point redirect `console.log` to stderr so service logs cannot corrupt
reply framing. Malformed input closes the reader with a failing exit code;
stdin closure stops cleanup scheduling and waits for active requests before
exit. The Go side correlates pending replies and ignores malformed output.

`desktopInternalCall` builds context outside caller-controlled parameters:
workspace/setup roots, current snapshots, daemon URL, and method-specific data
such as recent sessions or analytics snapshots. Preserve this separation;
browser-supplied params must not impersonate host observations. The public
dispatcher also handles some operations directly in Go, including runtime
status and heartbeat reads, rather than forwarding every method to Node.

Private lifecycle callbacks are not public hub RPCs. The Node host bridge caps
pending callbacks at 128 and times them out after 60 seconds; Go bounds callback
handling at 55 seconds and fences replies to the same child process generation.

## Failures and lifecycle

A missing bundle reports that desktop services are not installed; a missing
Node executable reports a startup error. Child exit fails pending operations
with an **unknown outcome** message and clears the cached process so a later
call can launch another. Caller cancellation removes its pending reply but does
not prove the operation was rolled back or cancel the child-side action. Do not
blindly retry a mutating operation after an acknowledgement failure.

The full-scope brain observes shared desktop state periodically. Spawn admission
uses private prepare/accept/cancel operations when the bundle is available;
remote-origin dispatch retains its own lease path. Without the companion,
`spawn` falls back to `spawnCore`, so bundle presence affects more than visible
Settings features. Check workflow/result/replacement integration when changing it.

## Build and validation

From `apps/desktop`:

```bash
npm run test:desktop-host
```

This builds the Node 22-targeted bundle into `dist/headless` and copies it beside
the Go binaries, then runs Node service tests and Go brain integration tests
with `WKS_DESKTOP_HOST_TEST_BUNDLE` set. The build rejects an Electron import.
An ordinary Go test run can skip companion integration when that test variable
is absent; a skipped integration test is not proof the packaged service works.

After changing the manifest or generator, inspect generated-file diffs and run
the authority tests in `services/hub/internal/bus`, plus desktop service and
backend parity tests. The generated bundle is ignored build output.
