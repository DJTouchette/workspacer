---
title: Native question picker fixture: native-harness serve --pending-questions
date: 2026-10-05
author: claude
confidence: high
related_paths:
  - apps/native/src/harness.rs
  - apps/native/scripts/smoke.py
promoted: false
---

# Native question picker fixture: native-harness serve --pending-questions

## Observation
native-harness serve --pending-questions (2026-10-05) gives demo-0000 a 3-question mixed set (single+descriptions, multiSelect, free text), demo-0002 a long two-option question, demo-0003 free text; each claude.answer prints 'fixture claude.answer <params>' to stderr and resolves the set via an agent.snapshot event. smoke.py --session <id> opens a fixture session. The answer resolves immediately, so sent/waiting states are only reachable in GPUI tests.

## Impact
Real-window question-picker captures and answer-shape checks need no paid model or production hub.

## Recommendation
Use with the private Xvfb rig; grep the harness stderr for the exact answers/answerKinds payload.
