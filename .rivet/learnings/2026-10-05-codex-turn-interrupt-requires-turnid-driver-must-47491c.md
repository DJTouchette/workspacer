---
title: Codex turn/interrupt requires turnId; driver must track the running turn
date: 2026-10-05
confidence: high
suggested_doc: claudemon-providers
related_paths:
  - services/claudemon/src/providers/codex.rs
promoted: false
---

# Codex turn/interrupt requires turnId; driver must track the running turn

## Observation
codex app-server (0.159 generate-json-schema) TurnInterruptParams require threadId AND turnId. claudemon's codex driver sent only {threadId}, so the request was refused as invalid params and the response was silently dropped: SIGINT/Stop never stopped a Codex turn (Claude stream uses a control interrupt and was fine). The turn id arrives on turn/started params.turn.id and on the turn/start response result.turn.id; turn/completed carries turn.id too. TurnTracker in codex.rs now tracks it, holds an interrupt requested between our turn/start and the id, drops one requested while idle, and logs a refused interrupt.

## Impact
Any new codex app-server request should be checked against generate-json-schema; unmatched-id responses are swallowed by handle_message, so wire-shape bugs fail silently.

## Recommendation
Validate app-server params against 'codex app-server generate-json-schema --out DIR'. Fake app-server modes interrupt_turn/interrupt_held in codex.rs tests refuse a turnId-less interrupt like the real server.
