---
title: Terminal shell allowlist must normalize host entries separately from requested argv
date: 2026-09-29
confidence: high
suggested_doc: headless-desktop-services
related_paths:
  - services/hub-rs/src/services/terminals.rs
  - services/hub/cmd/brain/shellallow.go
promoted: false
---

# Terminal shell allowlist must normalize host entries separately from requested argv

## Observation
Read-only parity audit found Go allowedShells trims and skips blank/comment SHELL entries while resolveTerminalShell preserves the raw SHELL value for an empty caller request. Rust resolve_shell currently inserts the raw default into its allowlist, so a whitespace-wrapped host entry fails its trimmed positive floor and a comment can qualify as an explicit request. Existing Rust unit/integration coverage lacks environment and /etc/shells fixture floors and asserts generic error rather than pre-spawn login-shell refusal. After the shared compiler freeze, the narrow fix normalizes only allowlist entries. A private-input fixture covers host/file/fallback floors and refusal vectors; the actual handler integration now requires the login-shell error. Validation is tracked in reviews/brain-shellallow.json.
