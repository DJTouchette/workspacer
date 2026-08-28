---
title: The conversation store COALESCES consecutive assistant_text items, so an AgentUpdate::Error ran straight into the reply that followed it
date: 2026-08-27
confidence: high
suggested_doc: claudemon-providers
related_paths:
  - services/claudemon/src/providers/mod.rs
  - apps/desktop/src/main/shared/workerFailure.ts
  - contracts/agent-error-marker-cases.json
promoted: false
---

# The conversation store COALESCES consecutive assistant_text items, so an AgentUpdate::Error ran straight into the reply that followed it

## Observation
Observed live on a copilot session whose facade could not attach: the pane rendered "⚠️ Error: …no structured questions).OK" as ONE message. `apply_updates` turns AgentUpdate::Error into an ordinary `ConversationItem::AssistantText` with `format!("⚠️ Error: {msg}")` and no trailing separator, and the store merges consecutive assistant_text items — so a mid-turn error and the turn's real reply arrive glued together. This affects EVERY managed provider (codex/opencode/pi), not just copilot; it was simply easier to trigger here because the facade check fires before the first token.</observation>
<parameter name="impact">Beyond the display bug, workerFailure.errorMarkerReason() (which the Fleet Manager's worker-finished wake reads) takes the marker line and would have carried the glued-on reply into the failure reason.</parameter>
<parameter name="recommendation">Fixed 2026-08-28: providers/mod.rs now emits `format!("⚠️ Error: {msg}\n")`. errorMarkerReason already takes only the first line, so both readers agree; contracts/agent-error-marker-cases.json gained a case pinning the coalesced shape. Don't drop the newline.
