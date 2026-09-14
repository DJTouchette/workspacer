---
title: Thinking/reasoning content is dropped by design across all 3 providers; Claude CLI's own spinner verb is decorative, not thinking-derived
date: 2026-09-03
confidence: high
related_paths:
  - services/claudemon/src/providers/claude_stream.rs
  - services/claudemon/src/session/conversation.rs
  - services/claudemon/src/providers/codex.rs
  - services/claudemon/src/providers/codex_rollout.rs
  - services/claudemon/src/providers/copilot.rs
promoted: false
---

# Thinking/reasoning content is dropped by design across all 3 providers; Claude CLI's own spinner verb is decorative, not thinking-derived

## Observation
Investigated "can we show a live thinking status line" (2026-09-03). Two premises to correct first: (1) Claude Code's own CLI spinner verb ("Pondering…", "Marinating…") is picked RANDOMLY from a ~150-word decorative list (`Rk(oxe())` in the bundled cli.js, near `spinnerVerbs`/`defaultVerb`) — NOT derived from the model's thinking content; only specific system states (compacting, hooks running) override it with a real message via `overrideMessage`. (2) Real `~/.claude/projects/*.jsonl` transcripts show assistant thinking blocks are frequently EMPTY (`"thinking": ""`) with only an opaque `signature` when redacted-thinking applies, so even captured thinking text is often unavailable. Workspacer itself explicitly drops reasoning/thinking on all three providers that carry it: claude_stream.rs `translate()` only forwards `text_delta` from stream_event content_block_delta, never `thinking_delta` (comment at claude_stream.rs:139-141: "thinking stays out of the conversation"); conversation.rs's `Block` enum has no Thinking variant so a `{"type":"thinking",...}` block silently falls out of `blocks()` (test asserts "thinking skipped"); codex.rs's JSON-RPC dispatch treats `item/reasoning/summaryTextDelta` as unknown/ignored (test `unknown_method_is_ignored`); codex_rollout.rs's `reasoning` item type hits `_ => {} // reasoning, etc. — skipped`; copilot.rs's reasoning field is "encrypted reasoning" the adapter never reads.

## Impact
Anyone asked to add a live 'what is happening' status line must add new plumbing (Block/AgentUpdate/ConversationItem variant), not just surface existing data — the whole pipeline discards reasoning content by design today, on all three providers.

## Recommendation
Of the three, Codex's item/reasoning/summaryTextDelta is the most promising real source (OpenAI's reasoning summaries are designed as human-readable prose, unlike Claude's often-redacted-empty thinking and Copilot's encrypted blob). Any implementation should treat the text as ephemeral/transient (not persisted as a conversation item) given its length and half-formed nature.
