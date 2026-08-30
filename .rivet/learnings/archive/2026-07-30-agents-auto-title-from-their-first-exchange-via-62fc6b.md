---
title: Agents auto-title from their first exchange via claudemon /oneshot
date: 2026-07-30
promoted: true
---

# Agents auto-title from their first exchange via claudemon /oneshot

## Observation
New: config.agents.autoTitle {enabled, model} (default on, haiku) + Settings → Session rows; renderer hook useAgentAutoTitle fires once per agent when the opening exchange has BOTH a real user message and an assistant reply; main's agentTitler sanitizes raw model output (sanitizeTitle rejects refusals/prose/preambles, caps 7 words/52 chars, drops trailing function words) and falls back to sessionTitles.cleanTitle. AgentWorkspace gained nameSetByUser (rename + spawn-dialog name) and autoTitled (persisted, so a restart doesn't re-title) — both ride the layout automatically since saveCurrentSession spreads ...a and migrateSessionData passes modern agents through as-is. THE TRAP: a headless  fires the user's Claude Code hooks, so claudemon's ingest registers a ghost session per call (verified: 35→36 rows; --settings '{"hooks":{}}' does not stop it). Hence the new claudemon POST /oneshot, which pins --session-id and mark_heartbeat()s it like keep-warm does; verified sessions [] before and after. Also: adding an ElectronAPI method requires triaging it in tests/backend/backendParity.test.ts or the parity test fails.

## Disposition
Folded into .rivet/context/domains/claudemon-http-api.md (/oneshot note extended with the auto-title consumer + parity-test reminder; the ghost-session trap was already there).
