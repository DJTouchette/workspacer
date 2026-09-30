---
title: Hub Plugin System
tags: [hub, rust, plugins, manifest, trusted-code, sidecar]
related_paths:
  - "services/hub-rs/src/plugins/mod.rs"
  - "services/hub-rs/src/plugins/manifest.rs"
  - "services/hub-rs/src/plugins/install.rs"
  - "services/hub-rs/src/plugins/settings.rs"
  - "services/hub-rs/src/plugins/supervisor.rs"
  - "services/hub-rs/src/mcp.rs"
owner: Damien Touchette
last_reviewed: 2026-09-30
---

# Hub Plugin System

## Current Rust ownership

Current plugin loading/identity lives in `services/hub-rs/src/plugins/mod.rs`;
`manifest.rs`, `install.rs`, `settings.rs` and `supervisor.rs` own the named
subsystems. Dynamic MCP exposure is in `services/hub-rs/src/mcp.rs`. Retained
examples live in `plugins/examples`, and public docs in `docs/plugins`.
The trust model below remains current; the `.go` module crosswalk under Key
modules describes the retained reference. Optional Node/Go plugin sidecars
remain supported external programs, not private backend companions.

Historical execution, when deliberately requested, uses the separate pinned
checkout described in [scripts/reference/README.md](../../../scripts/reference/README.md).
This crosswalk does not certify platform or release gates.

Plugins are trusted local extensions. Enabling one trusts its sidecar/install
code with the Workspacer user's machine access. Workspacer does not apply an OS
sandbox, workspace confinement, or manifest-derived method/path/event grants.
Legacy `capabilities`, path scopes, `emits`, `consumes`, and child tool-scope
fields remain parse-compatible advisory metadata.

Authentication still matters for identity and provenance. Each plugin receives
a stable token, revocation follows plugin lifecycle, host-owned topics/routes
remain protected, and provider registration is limited to the plugin's own
namespace via `provides`. Webview origin/CSP isolation protects the host app
document; it is not a filesystem sandbox for enabled plugin code.

## Key modules

- `manifest.go`: schema validation, advisory legacy fields, `provides` namespace,
  panes/settings/tools/launch integrations.
- `manager.go`: install/load/enable/disable/reload, token lifecycle, sidecar
  supervision, pane tokens, and ambient plugin registration.
- `install.go`: zip-slip/decompression guards and atomic trusted install.
- `settings.go`: typed settings, secret redaction on reads, `WKS_SETTINGS` for
  the trusted sidecar.
- `services/hub/cmd/mcp/plugins.go`: enabled plugin tools are added ambiently to supported
  spawned-agent server catalogs.

## Invariants

- `tools[].method` must match the same plugin's `provides` namespace.
- Core/provider namespaces cannot be claimed by a plugin manifest.
- Install extraction and UI asset serving retain archive/path traversal guards;
  these protect host-owned objects and do not imply runtime sandboxing.
- Unknown legacy path tokens/traversal spellings are still rejected so the
  displayed advisory intent is not misleading.
