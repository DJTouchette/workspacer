---
title: GitHub Copilot CLI `-p` mode has no approval gate, and its help text says the opposite
date: 2026-08-27
suggested_doc: claudemon-providers
related_paths:
  - services/claudemon/src/providers/copilot.rs
  - apps/desktop/src/renderer/src/lib/providerCaps.ts
  - apps/desktop/src/main/lib/managedSpawnOptions.ts
promoted: true
promoted_to: claudemon-providers
---

# GitHub Copilot CLI `-p` mode has no approval gate, and its help text says the opposite

## Observation
Verified live against copilot v1.0.81 (2026-08-28). `copilot --help` says `--allow-all-tools` is "required for non-interactive mode". It is not: a `copilot -p` run with NO allow flags happily ran `bash`. What the allow flags actually change is PATH/URL CONFINEMENT — with none, a write outside the session cwd comes back `tool.execution_complete {success:false, error:{message:"Permission denied and could not request permission from user", code:"denied"}}`; `--allow-all-tools` (and `--allow-all`) lift it. There is no approval event in the `-p` JSONL stream at all, and the CLI states outright that it has no channel to ask.</observation>
<parameter name="impact">A copilot session's permission pill would be lying if it read "Ask to approve". The two tiers that actually exist are cwd-confined (no allow flags) and unconfined (`--allow-all`). Also refutes the scout report's mapping, which took the help text at face value. Separately: `--deny-tool=bash` did NOT block bash, so the granular deny syntax is not the plain tool name.</observation>
<parameter name="recommendation">Keep the ask/yolo IDS (the whole bypass chain — bus clamp, brain launchPermissionMode, MCP facade — speaks them) but keep copilot's own LABELS ("Workspace only" / "Full access") in providerCaps.ts, and keep the explainUnsupportedManagedOptions line that announces it at spawn. If someone wants a real approval gate on Copilot, that requires the `--acp` long-lived path, not `-p`.</recommendation>
<parameter name="confidence">high
