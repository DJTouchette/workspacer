---
title: The headless Go brain resolves no supervisor/manager role model at all
date: 2026-08-27
confidence: high
suggested_doc: agent-spawn
promoted: false
---

# The headless Go brain resolves no supervisor/manager role model at all

## Observation
cmd/brain/handlers.go has no equivalent of the desktop's lib/supervisorModel or lib/roleModels: grep for supervisor|manager near 'model' in handlers.go returns nothing. It only forwards spawnParams.Model. So a supervisor or Fleet Manager spawned on a headless node (no Electron desktop running) ignores supervisor.model / supervisor.models / agents.managerModels entirely and gets the harness default. cmd/mcp/main.go DOES gate claude.defaultModel on providerIsClaude (pinned by spawndefaults_test.go), so the claude-id-to-codex leak is closed there — it is the ROLE models that are missing, not the provider gate.</observation>
<parameter name="impact">Role model settings are desktop-only. A headless/federated-peer supervisor or manager silently runs on a different model than the one configured in Settings.</parameter>
<parameter name="recommendation">If this matters, port perHarnessModel to Go beside launchPermissionMode (the existing desktop/brain twin pattern) and add it to cmd/brain/parity_test.go. Deliberately out of scope 2026-08-27; the desktop paths were the reported problem.</parameter>
<parameter name="related_paths">["services/hub/cmd/brain/handlers.go", "services/hub/cmd/mcp/spawndefaults_test.go"]
