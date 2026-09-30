---
title: Hub event bus and control plane
tags: [hub, bus, rpc, authentication, provenance, plugins, federation]
related_paths:
  - "services/hub-rs/src/runtime.rs"
  - "services/hub-rs/src/auth.rs"
  - "services/hub-rs/src/protocol.rs"
  - "services/hub-rs/src/server/policy.rs"
  - "apps/desktop/src/main/services/hubCapabilities.ts"
owner: Damien Touchette
last_reviewed: 2026-09-30
---

# Hub event bus and control plane

## Current Rust ownership

`services/hub-rs/src/runtime.rs` owns the bus actor and provider routing;
`auth.rs` owns persisted identities and authorization, `protocol.rs` owns wire
frames/topic matching, and `server/policy.rs` owns HTTP/WebSocket admission.
The retained policy below still applies. Go package/symbol spellings are
historical cross-references, not current runtime dependencies. Actual owner
coverage lives in `services/hub-rs/tests/auth.rs`, `bus_remaining.rs` and the
runtime tests; declaration metadata alone does not prove registration.

Historical execution, when deliberately requested, uses the separate pinned
checkout described in [scripts/reference/README.md](../../../scripts/reference/README.md).
This crosswalk does not certify platform or release gates.

## Current trust model

`/bus` is the authenticated WebSocket transport for events and request/reply
capabilities. Host tokens, scoped human/client tokens, provider-node tokens,
per-session facade bearers and per-plugin identity tokens preserve different
provenance and registration rights. Origin checks, exact view/triage allowlists,
host-only HTTP routes, provider ownership, call correlation and lifecycle
revocation remain security boundaries.

Filesystem and plugin *grants* are not boundaries. An authenticated agent may
name any canonical absolute host path, and an enabled plugin is trusted local
code with ambient bus/host access. Legacy manifest roots, capability lists,
child tiers and session grant fields are compatibility metadata only.

## Preserved containment and integrity

- Canonicalize caller paths component-by-component and open the returned path so
  symlinks plus `..` cannot make the checked and used objects differ.
- A path derived inside a selected object stays inside that object: library
  entries inside their library, git pathspecs inside their repo, replay files
  inside their worktree, and webview assets inside the owning plugin directory.
- Reject ambiguous case-variant JSON keys where language decoders could
  authorize one value and execute another.
- Accept launch-integration output only for the exact pending plugin-owned spawn.

These are semantic object/provenance checks, not workspace-root or secret-path
filters.

## Scoped remote clients

`view`, `triage`, `operator` and `provider` remain real scopes for remote clients
and nodes. Exact method allowlists fail closed for view/triage. Operator has the broad ordinary method surface, while actual host provenance
is still required by owner-only administration handlers. Provider is orthogonal: it may register only declared provider
methods and has a tiny outbound call surface. These scopes are separate from the
retired per-agent Workspacer tool tiers.

## RPC invariants

- Provider registration is single-owner and released when that connection drops.
- Results are accepted only from the connection owning the pending provider call.
- Federation checks the bare method, preserves peer provenance and refuses
  plugin-origin federation.
- Event publisher/consumer rules and payload vocabulary remain explicit.
- Redact bearer query parameters from every bus dial error and log site.
- Local handlers take precedence and call timeouts stay bounded.

Run bus RPC/authentication, plugin ambient/provenance, capspec composition,
federation and headless/desktop parity suites for control-plane changes. A
legacy anonymous-grant harness is compatibility coverage, not production policy.
