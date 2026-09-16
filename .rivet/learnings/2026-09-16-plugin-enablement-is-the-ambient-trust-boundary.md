---
title: Plugin enablement is the ambient trust boundary
date: 2026-09-16
confidence: high
suggested_doc: hub-plugin-system
related_paths:
  - services/hub/internal/plugin/manager.go
  - services/hub/internal/bus/bus.go
  - services/hub/cmd/mcp/plugins.go
  - apps/desktop/src/renderer/src/components/PluginInstallDialog.tsx
promoted: false
---

# Plugin enablement is the ambient trust boundary

## Observation

Plugin access is now intentionally ambient after installation and enablement. Legacy manifest capability, emit, consume, path, child-tool-scope, grant-pin, and per-session plugin-tool fields remain parse-compatible but do not authorize or confine runtime access. Authentication, revocation, enabled state, provider ownership within the plugin's own namespace, host-event provenance, webview asset confinement, and host-only external actions remain independent boundaries. UI copy must describe enabled plugins as user-level code with machine access rather than presenting manifest declarations as enforced permissions.
