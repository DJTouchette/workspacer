---
title: Hub Federation (hub-of-hubs): peer links, stamped events, qualified calls
tags: [hub, go, federation, event-bus, remote, multi-machine, security]
related_paths:
  - "services/hub/internal/federation/federation.go"
  - "services/hub/internal/federation/federation_test.go"
  - "services/hub/internal/busclient/client.go"
  - "services/hub/internal/busclient/subscribe_test.go"
  - "services/hub/internal/bus/rpc.go"
  - "services/hub/cmd/hub/main.go"
  - "apps/desktop/src/main/services/federationBridge.ts"
  - "apps/desktop/src/main/ipcFederationRouting.test.ts"
  - "apps/desktop/src/main/lib/snapshotLiveness.ts"
  - "apps/desktop/src/renderer/src/lib/federation.ts"
  - "apps/desktop/src/renderer/src/components/HubChip.tsx"
  - "apps/desktop/src/renderer/src/backend/webBackend.ts"
  - "apps/tui/src/federation.rs"
  - "services/hub/cmd/hub/mobile.html"
  - "services/hub/scripts/federation-harness.sh"
  - "docs/hub-federation.md"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Hub federation: peer events and qualified calls

## One local connection, peer provenance

`services/hub/internal/federation/federation.go` creates outbound bus links to
named peers. Its forwarding list is `agent.*` and `workflow.*`. Forwarded events
receive the peer name on the envelope and a fresh local broker identity; already
hub-stamped events are dropped rather than relayed again. This is one-hop event
propagation preventing recursive re-broadcast, not proof the configured peer
connection graph contains no cycles.

Clients merge peer fleets while keeping each row’s owning hub. A remote cwd is
not a path on the local machine. `snapshotIsLocalLiveSession` excludes remote
rows from local directory context; keep-warm and model-recording side effects
must likewise stay with the owning host.

## Peer configuration and authority

The peers file lives in the shared Workspacer config directory. Its array rows
contain name, URL, token and optional dispatch (default false). Dispatch is an
explicit worker-execution opt-in, distinct from linking a peer to view its fleet.
The configured peer credential limits operations at the destination.

Two save paths currently differ:

- Hub `federation.peersConfig` and `federation.savePeersConfig` require actual
  authenticated-host ownership. They redact tokens on read, preserve an omitted
  token, clear an explicitly empty token, validate/persist the new set, and call
  `Controller.Replace` to replace live links without restarting the hub.
- The legacy desktop IPC writer persists the file and restarts its app-owned hub;
  it leaves an adopted external hub running. Remote-client mode does not start a
  local hub just to reload a file for it.

The web backend uses the hub methods; it is no longer a read-only null stub.
An operator-scoped credential alone is not the host credential required by these
administrative handlers. Keep token secrecy, duplicate-name validation and
keep-versus-clear semantics in parity when changing either path.

## Qualified RPCs

`hub:<peer>/<method>` identifies the destination. The router checks the bare
method against scoped policy; plugin identities cannot use federation to gain
another host’s provenance. Unknown peers fail explicitly. Calls still need a
provider on the destination and remain subject to that connection’s credential.
Ambient filesystem tools do not remove selected-object or parameter-shape checks.

The federation hop has a 25-second budget, inside the bus router’s 30-second
budget. Timeout is not proof a remote side effect did not happen. The facade’s
fleet-list merge uses a shorter ten-second per-peer budget: a failed peer costs
its rows rather than corrupting the local listing. Per-session facade calls use
the supplied hub routing field; preserve that provenance on follow-up actions.

The hello method surface is not a complete live provider inventory. Do not infer
that an advertised/allowed method necessarily has a healthy registered provider.

## Reconnection, snapshots, and conversations

`busclient` reasserts subscriptions after reconnect. Each fleet client also needs
an explicit seed/reseed of peer snapshots: connection events can happen before
the local client has reconnected, so waiting only for a future agent event leaves
an idle peer undiscovered. Desktop’s federation bridge queries peer state at start
and on reconnection; web, mobile and TUI maintain their own equivalent projections.

A disconnected peer’s rows become offline tombstones rather than proof the agents
ended. Sparse brain snapshots are accepted by the desktop main bridge, mobile and TUI and must not
overwrite richer fields with invented defaults. PeerInfo.lastSeen uses epoch
milliseconds; the disconnected event’s lastSeen uses RFC3339. Keep those decoders
separate.

Desktop remote conversation reads are single-flight and use the main store’s
folded sequence. If the peer sequence moves backward, fetch a full rebuild rather
than continuing from the old watermark. A compact snapshot window must not replace
already-fetched full history or its offsets. Polling pauses for offline peers;
reconnects reseed rather than assuming the missed deltas will replay.

## UI action differences

The desktop can attach a GUI-only viewer to a peer session without creating a
local MessagePort. Its gate housekeeping can no-op for that remote row; explicit
local-only operations remain refused. Remote model switching is now qualified
through `hub:<peer>/claude.setModel`, with only the accepted canonical result
updating the mirror. Do not repeat the old blanket claim that every remote model
switch is forbidden. Separate permission/effort/handoff/terminal handlers have
their own current routing rules.

The renderer filters local cwd-bound terminal/review/editor pane choices for
peer-session context. This differs from full desktop remote-client mode, where
the selected server’s entire backend is remote. `/remote` remains the smaller
single-hub client; `/m`, full web/desktop and TUI have the merged fleet paths.

## Credentials, files, and verification

POSIX mode 0600 is not a Windows ACL assertion. The node credential exposure
helper uses a three-state verdict (owner-only, loose, unknown) and actual Windows
DACL inspection; do not reuse a POSIX mode check as proof a Windows credential
is private. Configuration metadata and a successful peer handshake are different
observations.

Use the Go federation/busclient tests, desktop federation bridge/IPC routing tests,
renderer sparse-row tests, and TUI federation tests for changes across this seam.
The scratch federation harness is for controlled ports and identities; inherited
WORKSPACER_PARENT_PID changes parentwatch behavior, so isolate its environment.
A source/unit review is not a claim that a live peer credential or network is
healthy. See [remote clients](../domains/remote-mobile.md) and
[the control plane](hub-bus-control-plane.md).

The web renderer can fold sparse updates, but its current `withPeerFleets` seed
explicitly skips sparse peer rows. That discovery limitation also affects desktop
bridged mode when it uses the web backend; do not claim complete headless-peer
visibility parity from the main bridge's support alone.
