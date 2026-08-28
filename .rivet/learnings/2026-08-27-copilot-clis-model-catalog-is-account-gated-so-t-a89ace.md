---
title: Copilot CLI's `--model` catalog is ACCOUNT-gated, so there is no model list to enumerate
date: 2026-08-27
confidence: high
suggested_doc: claudemon-providers
related_paths:
  - services/claudemon/src/providers/copilot.rs
  - apps/desktop/src/renderer/src/components/SpawnAgentDialog.tsx
promoted: false
---

# Copilot CLI's `--model` catalog is ACCOUNT-gated, so there is no model list to enumerate

## Observation
Verified live (copilot v1.0.81, 2026-08-28). Three independent enumeration routes are all dead ends: there is no `copilot models` subcommand and the generated shell completions carry no values for `--model`; the ACP handshake (`--acp` → `initialize` + `session/new`) returns session modes and `configOptions` but NO model list and no model config option; and GitHub's `api.github.com/copilot_internal/v2/token` answers 403 to a `gh` OAuth token. Worse, the ids are account-gated: on the probe account EVERY explicit `--model <id>` was refused with `Model "<id>" from --model flag is not available` — including `gpt-5-mini` and `claude-haiku-4.5`, the two ids Copilot's own auto-router had just chosen for that same account. The captured `modelInfo` says why: `"model_picker_enabled": false`. Only `auto` works.</observation>
<parameter name="impact">A curated table of Copilot model ids taken from the GitHub changelog would have shipped a spawn-dialog picker where every entry fails at launch. Note the failure is at least LOUD: a rejected model exits 1 with that message on stderr and emits no `result` event, which the adapter's turn_outcome() surfaces as AgentUpdate::Error.</parameter>
<parameter name="recommendation">`copilot::list_models()` returns `auto` only, gated on a live `bin --version` liveness probe so a missing CLI errors instead of returning a plausible-looking list. Free-text model entry still works for accounts whose plan enables the picker. Do NOT add a hardcoded id table.
