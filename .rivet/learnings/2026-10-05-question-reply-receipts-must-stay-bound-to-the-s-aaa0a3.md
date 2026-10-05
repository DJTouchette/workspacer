---
title: Question reply receipts must stay bound to the submitted question identity
date: 2026-10-05
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui.rs
  - apps/native/src/ui/features.rs
promoted: false
---

# Question reply receipts must stay bound to the submitted question identity

## Observation
The question picker reset answer state for a new pending payload and then marked answers_sent from any successful same-session answer receipt. A late reply to the previous set could therefore lock new questions. Retain the submitted signature and literal answers until acknowledgement, mark sent/error only for the same signature, and gate duplicate submits locally before the asynchronous busy frame. GPUI regression covers a new question and old receipt in the same view update; a real fixture CtrlEnter sent exactly one text-kind answer vector and preserved the composer draft.
