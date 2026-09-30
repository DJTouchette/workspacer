---
title: Shared desktop services in the Rust backend
tags: [headless, desktop-host, services, parity, authentication, rust]
related_paths:
  - "contracts/desktop-service-methods.json"
  - "apps/desktop/scripts/gen-desktop-services.mjs"
  - "apps/desktop/src/main/headless/desktopHost.ts"
  - "apps/desktop/src/main/services/nativeDesktopServices.ts"
  - "apps/desktop/src/renderer/src/backend/desktopServices.ts"
  - "services/hub-rs/src/backend.rs"
  - "services/hub-rs/src/services/mod.rs"
  - "services/hub-rs/COMPANION_RETIREMENT.md"
owner: Damien Touchette
last_reviewed: 2026-09-30
---

# Shared desktop services in the Rust backend

## Current owners

`services/hub-rs` supplies the standalone service and the backend embedded by the
native client. Its owned Rust services replace the private Node companion. No
`desktop-host.cjs`, private stdio host bridge, `build:desktop-host`, or
`test:desktop-host` command is required by the current product.

Electron retains `apps/desktop/src/main/services/nativeDesktopServices.ts`, the
public dispatcher in `main/headless/desktopHost.ts`, and their shared JavaScript
services. The `headless` directory name does not make those live implementations
retired. In Electron's normal local composition, Rust owns the control plane and
Electron registers its desktop services; standalone/native composition installs
Rust owners. See [the companion retirement map](../../../services/hub-rs/COMPANION_RETIREMENT.md)
for the exact boundary.

Enabled plugin sidecars and provider CLIs remain external programs. In particular,
`plugins/examples/headroom` is a public trusted Node sidecar, not a replacement
private backend process. Node also remains desktop/build/test tooling.

## Registry and authority

`contracts/desktop-service-methods.json` is the shared declaration. The current
`apps/desktop/scripts/gen-desktop-services.mjs` emits the TypeScript registry;
Rust imports the portable declaration through its own service composition. The
generator no longer edits Go capspec or bus sources. Declaration membership is
not proof a runtime handler was installed: retain actual service inventory and
call/refusal tests.

Public `desktop.*` methods require authenticated host provenance at the bus
boundary; a scoped operator bearer alone does not acquire it. The manifest's
owner methods and `ui.fonts`/`ui.asset` have distinct gates. Preserve those
boundaries when adding a method to native services, Rust services and the
renderer bus adapter in `renderer/src/backend/desktopServices.ts`.

Host snapshots, observed context, launch attribution and caller identity must
come from their actual owners, not browser-supplied fields. Rust's local lookup,
launch coordinator, session facade and workflow observers supply those facts.

## Launch and replacement ownership

The old Node `internal.prepareIntegration` callback maps to
`services/hub-rs/src/plugins/launch.rs`: an opaque permit belongs to a pending
owner launch, is rechecked after asynchronous plugin preparation, and cannot be
forged by a JSON grant. The plugin gets minimal context, not the host's credential
bearing environment. Generation-scoped lifecycle cleanup retains failed
revocations for retry.

The old private `replacement.*` callbacks map to the typed
`manager_replacements::NativeHost` and durable `ReplacementService`. The journal
owns uncertain delivery, task-before-worker transfer, held messages and actual
viewer acknowledgement. Do not recreate the callback strings as public RPCs.
Caller cancellation or lost acknowledgement is not proof a mutation was rolled
back and is not authorization to replay it. The spawn coordinator retains
accepted work beyond its caller, and actual engine observations fence teardown.

## Persistence and retirement

Current configuration has Rust and public Electron writers. The Rust runtime
owns its launch/replacement journals and standalone analytics store; it does not
silently convert all Electron history. `PERSISTED_STATE_REVIEW.md` distinguishes
per-family byte/reopen/failure evidence. Retired intent-workspace SQLite/artifact
preservation is tested across two real Backend lifetimes; preserving old user
data is separate from restoring the retired feature.

Original Go source, where retained, is a reference only. Historical private-pipe
framing, Node discovery and bundle commands are documented in Git history and
source-specific migration reviews. For deliberate oracle execution use the
pinned-checkout commands in [scripts/reference/README.md](../../../scripts/reference/README.md).
There is no automatic fallback to a current-tree Go service or private companion.

## Validation

From `apps/desktop`, `npm run test:desktop-services` runs the maintained Rust
integration targets, Electron service tests and stale private-build-output guard.
It uses both existing language owners; it does not build a private Node bundle.
From the repository root, `make check-hub-rust-assets` checks portable generators
and retained assets, and `make test-hub-rust` exercises the shared backend.

Relevant actual tests include launch lifecycle/spawn, plugin preparation,
manager replacement, workflow/task ownership, fleet review, legacy preservation,
and headless capability completeness. Browser dispatch fixtures use the actual
Rust broker with explicit test-support setup; mocked service tests alone do not
prove process ownership. Electron packaged, native installer and platform gate
receipts remain separate. This context does not mark any migration gate verified.
