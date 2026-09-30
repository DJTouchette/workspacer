---
title: Managed question kinds and exposed child threads need real provider-path proof
date: 2026-09-30
suggested_doc: claudemon-http-api
related_paths:
  - services/hub-rs/tests/local_spawn.rs
  - services/hub-rs/tests/fixtures/fake_claude_stream.py
  - services/hub-rs/tests/fixtures/fake_codex_subagent.py
promoted: false
---

# Managed question kinds and exposed child threads need real provider-path proof

## Observation
The Unix-isolated local_spawn fixture now drives real registered claude.answer through the owned Claude stream adapter into an inert Python control_response receiver: two numeric text values remain literal while an option-kind numeric value maps to its label. An inert loopback Codex app-server then proves sessions.subagentConversation refuses an existing rollout before parent exposure, returns its two real parsed turns after a thread/started exposure marker, and refuses hidden/missing/traversal child IDs. The owning local_spawn target passed1 with0ignored in2.8seconds; it also confirms all recorded fake provider PIDs are reaped. No model provider or production state is contacted.

## Impact
A PTY answer fixture cannot prove the managed answerKinds branch, and raw engine child replay does not prove hub forwarding preserves parent exposure.

## Recommendation
Keep literal-text and option-kind positive controls together, and keep child exposure separate from merely finding a rollout file. Record this fixture as Unix-only rather than a Windows execution claim.
