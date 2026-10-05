---
title: Title model selection: explicit per-harness model is exact; autoTitle.provider pins the harness
date: 2026-10-05
confidence: high
suggested_doc: config
related_paths:
  - services/hub-rs/src/services/provider_utilities/text.rs
  - apps/desktop/src/main/lib/roleModels.ts
  - apps/desktop/src/main/services/agentTitler.ts
  - apps/native/src/features.rs
promoted: false
---

# Title model selection: explicit per-harness model is exact; autoTitle.provider pins the harness

## Observation
agents.autoTitle gained provider ('' = each agent's own harness). Resolution is shared by hub-rs text::title_target and desktop roleModels.resolveTitleTarget: models[harness] is EXPLICIT and passed exactly (no serves()/resolveCompletionModel downgrade any more); only the legacy single autoTitle.model (default haiku) is filtered by whether it serves the harness; claude with nothing configured still uses haiku. A rejected model ends as autoTitle{state:fallback,reason:unsupported} with the first line of the request — never reported as a model title. Writers (desktop SessionSection and native TitleChange::Model) keep the legacy model field in step only when the id serves that harness. Native shows the effective model (own row, else legacy if servable) and labels Claude's empty row 'Default · Haiku' because the CLI catalog's marked default is the chat default, not the title default.

## Impact
Old tests pinned a silent downgrade of cross-wired map entries; that behavior is intentionally gone.
