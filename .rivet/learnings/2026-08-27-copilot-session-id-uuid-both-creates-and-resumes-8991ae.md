---
title: `copilot --session-id <uuid>` both CREATES and RESUMES, which is what makes a one-shot `-p` adapter a real conversation
date: 2026-08-27
confidence: high
suggested_doc: claudemon-providers
related_paths:
  - services/claudemon/src/providers/copilot.rs
  - services/hub/cmd/brain/handlers.go
promoted: false
---

# `copilot --session-id <uuid>` both CREATES and RESUMES, which is what makes a one-shot `-p` adapter a real conversation

## Observation
Verified live twice (copilot v1.0.81, 2026-08-28) and again end-to-end through claudemon: passing the SAME `--session-id <uuid>` to two separate `copilot -p` processes resumes the first one's conversation (a codeword set in turn 1 was recalled in turn 2). `--resume <uuid>` works too, but is unnecessary. Also observed: the terminal `result` event is TOP-LEVEL, not under `data` — `{"type":"result","sessionId":…,"exitCode":…,"usage":{…}}` — and its `usage` is session-CUMULATIVE, while `model.model_call_success`'s `responseChunk.usage` is per-model-call (hence UsageAcc::additive). `model.turn_started` carries `modelInfo.capabilities.limits.max_context_window_tokens` = Copilot's OWN window for the model (144000 for claude-haiku-4.5, not Anthropic's 200k).</observation>
<parameter name="impact">Removes the entire `codex-threads` sidecar class of work. It also makes `restartPreservesConversation: true` honest for copilot (the only managed provider that can claim it), makes `--model`/`--effort` genuinely live (the next turn is a new argv), and means the brain needs NO wire `resume` field for copilot — reusing the prior session id is enough.</parameter>
<parameter name="recommendation">Keep the driver parked on the input channel between turns and spawn one process per message. Never key "turn over" on process exit alone: a hard Copilot failure can print prose to stderr while exiting 0 with clean JSONL and no error event, so `turn_outcome()` requires empty stderr AND a `result` frame AND exitCode 0 AND a zero process exit AND some output before emitting a bare Idle.
